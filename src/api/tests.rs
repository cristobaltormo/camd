use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio::sync::watch;
use tower::ServiceExt;

use crate::app::{AppState, Camera, CameraRuntime, router};
use crate::arming::Arming;
use crate::auth::Auth;
use crate::auth::webauthn::soft::SoftAuthenticator;
use crate::clock;
use crate::config::{Config, SharedConfig};
use crate::db::Database;
use crate::notify::{Channel, Notifier};
use crate::photos::{PhotoError, Photos};
use crate::prefs::{self, MotionSettings};
use crate::presence::Presence;
use crate::runtime::{ClipError, Frame, ManualClip};

pub const ORIGIN: &str = "http://localhost:8180";

pub struct FakeRuntime {
    cfg: SharedConfig,
    db: Arc<Database>,
    photos: Photos,
    arming: Arming,
    presence: Arc<Presence>,
    frames: watch::Sender<Option<Frame>>,
    pub standby: AtomicBool,
    pub privacy_answers: AtomicBool,
    pub photo_limit: AtomicI64,
    manual: Mutex<ManualClip>,
    pub prefs_applied: Mutex<Vec<BTreeSet<String>>>,
}

impl FakeRuntime {
    pub fn new(cfg: Config) -> Arc<FakeRuntime> {
        let db = Arc::new(Database::open(&cfg.db_path()).unwrap());
        let mut cfg = cfg;
        prefs::load_stored(&mut cfg, &db).unwrap();
        let presence = Presence::new(cfg.presence_devices.clone(), cfg.presence_grace_s);
        let cfg = cfg.shared();
        Arc::new(FakeRuntime {
            photos: Photos::new(Arc::clone(&cfg), Arc::clone(&db)),
            arming: Arming::new(Arc::clone(&db), Arc::clone(&presence)),
            presence,
            cfg,
            db,
            frames: watch::channel(None).0,
            standby: AtomicBool::new(false),
            privacy_answers: AtomicBool::new(true),
            photo_limit: AtomicI64::new(crate::photos::MAX_PHOTOS),
            manual: Mutex::new(ManualClip::inactive()),
            prefs_applied: Mutex::new(Vec::new()),
        })
    }

    pub fn publish(&self, ts: f64, jpeg: &[u8]) {
        self.frames.send_replace(Some(Frame {
            ts,
            jpeg: Bytes::copy_from_slice(jpeg),
        }));
    }

    fn enabled(&self) -> bool {
        self.cfg.read().unwrap().camera_enabled
    }

    fn online(&self) -> bool {
        self.enabled()
            && self
                .frames
                .borrow()
                .as_ref()
                .is_some_and(|f| clock::now() - f.ts < 10.0)
    }
}

impl CameraRuntime for FakeRuntime {
    fn config(&self) -> SharedConfig {
        Arc::clone(&self.cfg)
    }
    fn db(&self) -> &Arc<Database> {
        &self.db
    }
    fn photos(&self) -> &Photos {
        &self.photos
    }
    fn status(&self, version: &str, build: &str) -> Value {
        let (armed, reason) = self.arming.decision();
        let cfg = self.cfg.read().unwrap().clone();
        let (events, bytes) = self.db.events_usage().unwrap();
        let (photos, photo_bytes) = self.photos.usage();
        json!({
            "armed": armed, "reason": reason, "mode": self.arming.mode(),
            "snooze_until": self.arming.snooze_until(), "privacy": self.arming.privacy(),
            "presence": {"configured": self.presence.configured(), "home": self.presence.is_home()},
            "camera": {"enabled": cfg.camera_enabled, "online": self.online(), "standby": self.standby.load(Ordering::Relaxed)},
            "detection": {"mode": cfg.detection, "state": if cfg.detection == "off" { "off" } else { "idle" }},
            "channels": [], "version": version, "build": build,
            "storage": {"events": events, "bytes": bytes, "cap_bytes": crate::storage::cap_bytes(&cfg), "photos": photos, "photo_bytes": photo_bytes},
            "clip": self.manual.lock().unwrap().to_json(),
        })
    }
    fn frames(&self) -> watch::Receiver<Option<Frame>> {
        self.frames.subscribe()
    }
    fn fresh_frame(&self) -> Option<Frame> {
        if self.online() {
            self.frames.borrow().clone()
        } else {
            None
        }
    }
    fn live_available(&self) -> bool {
        self.enabled() && !self.standby.load(Ordering::Relaxed)
    }
    fn set_mode(&self, mode: &str) -> crate::error::Result<()> {
        self.arming.set_mode(mode)?;
        self.arming.snooze(0.0)
    }
    fn snooze(&self, minutes: f64) -> crate::error::Result<()> {
        self.arming.snooze(minutes)
    }
    fn set_privacy(&self, on: bool) -> bool {
        if !self.privacy_answers.load(Ordering::Relaxed) {
            return false;
        }
        self.arming.set_privacy(on).is_ok()
    }
    fn clip_start(&self, seconds: u32) -> Result<ManualClip, ClipError> {
        if !self.enabled() || self.standby.load(Ordering::Relaxed) || !self.online() {
            return Err(ClipError::Unavailable);
        }
        let now = clock::now();
        let clip = ManualClip {
            active: true,
            id: Some(1),
            until: now + f64::from(seconds),
            started: now,
        };
        *self.manual.lock().unwrap() = clip.clone();
        Ok(clip)
    }
    fn clip_stop(&self) -> Option<i64> {
        let mut manual = self.manual.lock().unwrap();
        let id = manual.active.then_some(manual.id).flatten();
        *manual = ManualClip::inactive();
        id
    }
    fn motion_settings(&self) -> MotionSettings {
        prefs::load_motion(&self.db)
    }
    fn motion_settings_changed(&self, _settings: MotionSettings) {}
    fn apply_prefs(&self, changed: &BTreeSet<String>) {
        self.prefs_applied.lock().unwrap().push(changed.clone());
    }
    fn take_photo(&self) -> Result<Value, PhotoError> {
        let frame = self.fresh_frame().ok_or(PhotoError::NoFrame)?;
        let limit = self.photo_limit.load(Ordering::Relaxed);
        if self.db.count_photos()? >= limit {
            return Err(PhotoError::Limit(format!("photo limit of {limit} reached")));
        }
        let id = self.db.insert_photo(frame.ts)?;
        let folder = self
            .cfg
            .read()
            .unwrap()
            .photos_dir()
            .join(clock::local(frame.ts).month_path());
        std::fs::create_dir_all(&folder)?;
        let (path, thumb) = (
            folder.join(format!("{id}.jpg")),
            folder.join(format!("{id}_t.jpg")),
        );
        std::fs::write(&path, &frame.jpeg)?;
        std::fs::write(&thumb, &frame.jpeg[..frame.jpeg.len() / 2])?;
        let size = (frame.jpeg.len() + frame.jpeg.len() / 2) as i64;
        self.db
            .set_photo_files(id, &path.to_string_lossy(), &thumb.to_string_lossy(), size)?;
        Ok(json!({"id": id, "ts": frame.ts, "size": size}))
    }
    fn stop(&self) {}
}

pub struct Recorder(pub Arc<Mutex<Vec<(String, String, String)>>>);

impl Channel for Recorder {
    fn name(&self) -> &'static str {
        "notifich"
    }
    fn send(&self, title: &str, body: &str, url: &str) -> bool {
        self.0
            .lock()
            .unwrap()
            .push((title.into(), body.into(), url.into()));
        true
    }
}

pub struct Answer {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl Answer {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
    pub fn header(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    }
    pub fn code(&self) -> u16 {
        self.status.as_u16()
    }
}

pub struct Env {
    pub dir: tempfile::TempDir,
    pub app: Arc<AppState>,
    pub router: Router,
    pub fakes: Vec<Arc<FakeRuntime>>,
    pub sent: Arc<Mutex<Vec<(String, String, String)>>>,
    pub cookies: Mutex<HashMap<String, String>>,
}

pub fn base_config(dir: &std::path::Path) -> Config {
    Config {
        rp_id: "localhost".into(),
        origin: ORIGIN.into(),
        camera_token: "t".into(),
        detection: "off".into(),
        ..Config::with_data_dir(dir)
    }
}

impl Env {
    pub fn anonymous() -> Env {
        Env::with(|cfg| cfg, &[])
    }

    pub fn with(tweak: impl Fn(Config) -> Config, extras: &[(&str, &str)]) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let primary = tweak(base_config(&dir.path().join("first")));
        let mut fakes = vec![FakeRuntime::new(primary.clone())];
        for (id, name) in extras {
            let cfg = Config {
                camera_id: id.to_string(),
                camera_name: name.to_string(),
                ..base_config(&dir.path().join(id))
            };
            fakes.push(FakeRuntime::new(cfg));
        }
        let db = Arc::clone(fakes[0].db());
        let sent = Arc::new(Mutex::new(Vec::new()));
        let mut notifier = Notifier::new(&primary, Arc::clone(&db)).unwrap();
        notifier
            .channels
            .push(Box::new(Recorder(Arc::clone(&sent))));
        let cameras = fakes
            .iter()
            .map(|f| {
                let id = f.cfg.read().unwrap().camera_id.clone();
                Camera::new(&id, f.clone() as Arc<dyn CameraRuntime>)
            })
            .collect();
        let app = Arc::new(AppState::new(
            cameras,
            Arc::new(Auth::new(&primary, db)),
            Arc::new(notifier),
        ));
        Env {
            router: router(Arc::clone(&app)),
            dir,
            app,
            fakes,
            sent,
            cookies: Mutex::new(HashMap::new()),
        }
    }

    pub async fn enrolled() -> (Env, SoftAuthenticator) {
        let env = Env::anonymous();
        let device = env.enroll().await;
        (env, device)
    }

    pub async fn enroll(&self) -> SoftAuthenticator {
        let device = SoftAuthenticator::new("localhost", ORIGIN);
        let code = self.app.auth.new_enroll_code().unwrap();
        let options = self
            .post("/api/auth/register/options", json!({"code": code}))
            .await
            .json();
        let answer = self
            .post(
                "/api/auth/register/verify",
                json!({"code": code, "credential": device.create(&options), "name": "test"}),
            )
            .await;
        assert_eq!(answer.code(), 200, "{:?}", answer.body);
        device
    }

    pub fn rt(&self) -> &Arc<FakeRuntime> {
        &self.fakes[0]
    }

    pub fn cfg(&self) -> Config {
        self.rt().cfg.read().unwrap().clone()
    }

    pub fn set_cookie(&self, name: &str, value: &str) {
        self.cookies
            .lock()
            .unwrap()
            .insert(name.into(), value.into());
    }

    pub fn forget_cookies(&self) {
        self.cookies.lock().unwrap().clear();
    }

    pub async fn send(&self, request: Request<Body>) -> Answer {
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        for value in headers.get_all("set-cookie") {
            let text = value.to_str().unwrap();
            let (pair, _) = text.split_once(';').unwrap_or((text, ""));
            let (k, v) = pair.split_once('=').unwrap();
            if text.contains("Max-Age=0") {
                self.cookies.lock().unwrap().remove(k);
            } else {
                self.cookies.lock().unwrap().insert(k.into(), v.into());
            }
        }
        let body = response.into_body().collect().await.unwrap().to_bytes();
        Answer {
            status,
            headers,
            body,
        }
    }

    pub fn request(
        &self,
        method: &str,
        path: &str,
        extra: &[(&str, &str)],
    ) -> axum::http::request::Builder {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "localhost:8180");
        let cookies = self.cookies.lock().unwrap();
        if !cookies.is_empty() {
            let text: Vec<String> = cookies.iter().map(|(k, v)| format!("{k}={v}")).collect();
            builder = builder.header("cookie", text.join("; "));
        }
        for (k, v) in extra {
            builder = builder.header(*k, *v);
        }
        builder
    }

    pub async fn get(&self, path: &str) -> Answer {
        self.get_with(path, &[]).await
    }

    pub async fn get_with(&self, path: &str, extra: &[(&str, &str)]) -> Answer {
        self.send(
            self.request("GET", path, extra)
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    pub async fn post(&self, path: &str, body: Value) -> Answer {
        self.raw(
            "POST",
            path,
            body.to_string().into_bytes(),
            &[("origin", ORIGIN), ("content-type", "application/json")],
        )
        .await
    }

    pub async fn delete(&self, path: &str) -> Answer {
        self.raw("DELETE", path, Vec::new(), &[("origin", ORIGIN)])
            .await
    }

    pub async fn raw(
        &self,
        method: &str,
        path: &str,
        body: Vec<u8>,
        extra: &[(&str, &str)],
    ) -> Answer {
        self.send(
            self.request(method, path, extra)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
    }

    pub fn frame(&self) -> Vec<u8> {
        let jpeg = sample_jpeg();
        self.rt().publish(clock::now(), &jpeg);
        jpeg
    }

    pub fn insert_event(
        &self,
        start: f64,
        end: f64,
        labels: &[&str],
        analyzed: bool,
        clip: &str,
        thumb: &str,
    ) -> i64 {
        self.rt()
            .db()
            .insert(
                "INSERT INTO events(start_ts, end_ts, status, clip_path, thumb_path, size_bytes, labels, analyzed, confirmed) VALUES(?, ?, 'done', ?, ?, 1, ?, ?, ?)",
                rusqlite::params![start, end, clip, thumb, serde_json::to_string(labels).unwrap(), analyzed, !labels.is_empty()],
            )
            .unwrap()
    }
}

pub fn sample_jpeg() -> Vec<u8> {
    let mut out = Vec::new();
    let pixels: Vec<u8> = (0..64 * 48)
        .flat_map(|i| [(i % 251) as u8, (i % 13 * 9) as u8, 90])
        .collect();
    jpeg_encoder::Encoder::new(&mut out, 90)
        .encode(&pixels, 64, 48, jpeg_encoder::ColorType::Rgb)
        .unwrap();
    out
}

#[tokio::test]
async fn mode_snooze_and_status() {
    let (env, _) = Env::enrolled().await;
    assert_eq!(
        env.post("/api/mode", json!({"mode": "bogus"})).await.code(),
        400
    );
    assert_eq!(
        env.post("/api/mode", json!({"mode": "armed"})).await.code(),
        200
    );
    let status = env.get("/api/status").await.json();
    assert!(status["armed"].as_bool().unwrap() && status["mode"] == "armed");
    assert_eq!(status["version"], crate::VERSION);
    assert_eq!(status["build"], crate::assets::BUILD);
    env.post("/api/snooze", json!({"minutes": 30})).await;
    let status = env.get("/api/status").await.json();
    assert!(!status["armed"].as_bool().unwrap() && status["reason"] == "snoozed");
    env.post("/api/snooze", json!({"minutes": 0})).await;
    assert_eq!(env.get("/api/status").await.json()["armed"], true);
    assert_eq!(
        env.post("/api/snooze", json!({"minutes": 2000}))
            .await
            .code(),
        422
    );
    assert_eq!(
        env.post("/api/snooze", json!({"minutes": -1})).await.code(),
        422
    );
}

#[tokio::test]
async fn auto_mode_follows_presence() {
    let (env, _) = Env::enrolled().await;
    env.post("/api/mode", json!({"mode": "auto"})).await;
    assert_eq!(
        env.get("/api/status").await.json()["reason"],
        "auto-without-device"
    );
    env.rt().presence.set_external(Some(true));
    assert_eq!(env.get("/api/status").await.json()["armed"], false);
    env.rt().presence.set_external(Some(false));
    let status = env.get("/api/status").await.json();
    assert!(status["armed"] == true && status["reason"] == "away");
}

#[tokio::test]
async fn snapshot_and_live_stream() {
    let (env, _) = Env::enrolled().await;
    assert_eq!(
        env.get("/api/snapshot").await.json(),
        json!({"detail": "no frame"})
    );
    let frame = env.frame();
    let snap = env.get("/api/snapshot?download=1").await;
    assert_eq!(snap.body.as_ref(), frame.as_slice());
    assert_eq!(snap.header("content-type"), "image/jpeg");
    assert_eq!(snap.header("cache-control"), "no-store");
    assert!(
        snap.header("content-disposition")
            .starts_with("attachment; filename=\"snapshot-")
    );

    let response = env
        .router
        .clone()
        .oneshot(
            env.request("GET", "/api/live?t=1", &[])
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-type"],
        "multipart/x-mixed-replace; boundary=frame"
    );
    assert_eq!(response.headers()["cache-control"], "no-store");
    let rt = Arc::clone(env.rt());
    let feeder = tokio::spawn(async move {
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            rt.publish(clock::now(), &sample_jpeg());
        }
    });
    let mut body = response.into_body();
    let mut data = Vec::new();
    while data.windows(7).filter(|w| *w == b"--frame").count() < 3 {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Ok(chunk) = frame.into_data() {
            data.extend_from_slice(&chunk);
        }
    }
    let text = String::from_utf8_lossy(&data);
    assert!(text.starts_with(&format!(
        "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
        frame.len()
    )));
    assert_eq!(env.fakes[0].frames.receiver_count(), 1);
    assert_eq!(env.app.cameras[0].viewers.len(), 1);
    drop(body);
    feeder.await.unwrap();
    assert_eq!(env.app.cameras[0].viewers.len(), 0);
}

#[tokio::test]
async fn a_fifth_viewer_ends_the_oldest_stream_and_shutdown_ends_all() {
    let (env, _) = Env::enrolled().await;
    let mut bodies = Vec::new();
    for _ in 0..5 {
        let response = env
            .router
            .clone()
            .oneshot(
                env.request("GET", "/api/live", &[])
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        bodies.push(response.into_body());
    }
    let ended = tokio::time::timeout(Duration::from_secs(2), bodies[0].frame())
        .await
        .unwrap();
    assert!(ended.is_none());
    env.app.shutdown.send(true).unwrap();
    for body in bodies.iter_mut().skip(1) {
        assert!(
            tokio::time::timeout(Duration::from_secs(2), body.frame())
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test(start_paused = true)]
async fn the_live_stream_ends_when_no_frame_arrives() {
    let (env, _) = Env::enrolled().await;
    let response = env
        .router
        .clone()
        .oneshot(
            env.request("GET", "/api/live", &[])
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let mut body = response.into_body();
    let waited = tokio::time::Instant::now();
    assert!(body.frame().await.is_none());
    let elapsed = waited.elapsed().as_secs_f64();
    assert!((15.0..16.0).contains(&elapsed), "{elapsed}");
}

#[tokio::test]
async fn stale_frames_and_standby_are_refused() {
    let (env, _) = Env::enrolled().await;
    env.rt().publish(clock::now() - 60.0, &sample_jpeg());
    assert_eq!(env.get("/api/snapshot").await.code(), 503);
    env.rt().standby.store(true, Ordering::Relaxed);
    let answer = env.get("/api/live").await;
    assert_eq!(
        (answer.code(), answer.json()),
        (503, json!({"detail": "camera in standby"}))
    );
}

#[tokio::test]
async fn security_headers_and_caching() {
    let env = Env::anonymous();
    let index = env.get("/").await;
    assert!(
        index
            .header("content-security-policy")
            .contains("script-src 'self'")
    );
    assert_eq!(index.header("x-frame-options"), "DENY");
    assert_eq!(index.header("x-content-type-options"), "nosniff");
    assert_eq!(index.header("referrer-policy"), "no-referrer");
    assert_eq!(index.header("content-type"), "text/html; charset=utf-8");
    assert_eq!(index.header("cache-control"), "private, no-cache");
    assert_eq!(
        env.get("/api/auth/state").await.header("cache-control"),
        "no-store"
    );
    let css = env.get("/style.css").await;
    assert_eq!(css.header("cache-control"), "private, no-cache");
    assert_eq!(css.header("content-type"), "text/css; charset=utf-8");
    let main = env.get("/js/main.js").await;
    assert_eq!(main.header("cache-control"), "private, no-cache");
    assert!(String::from_utf8_lossy(&main.body).contains(crate::assets::BUILD));
    let etag = main.header("etag").to_string();
    assert_eq!(
        env.get_with("/js/main.js", &[("if-none-match", &etag)])
            .await
            .code(),
        304
    );
    let sw = env.get("/sw.js").await;
    assert_eq!(sw.header("cache-control"), "no-cache");
    assert_eq!(sw.header("content-type"), "text/javascript; charset=utf-8");
    assert_eq!(env.get("/healthz").await.json(), json!({"ok": true}));
    assert_eq!(
        env.get("/healthz").await.header("cache-control"),
        "private, no-cache"
    );
    for missing in [
        "/nope.css",
        "/js/",
        "/js",
        "/api/nope",
        "/../Cargo.toml",
        "/%2e%2e/Cargo.toml",
    ] {
        let answer = env.get(missing).await;
        assert_eq!(
            (answer.code(), answer.json()),
            (404, json!({"detail": "Not Found"})),
            "{missing}"
        );
    }
    assert_eq!(
        env.get("/api/nope").await.header("cache-control"),
        "no-store"
    );
    assert_eq!(
        env.get("/fonts/inter.woff2").await.header("content-type"),
        "font/woff2"
    );
}

#[tokio::test]
async fn index_and_manifest_carry_the_site_name_and_language() {
    let env = Env::with(
        |cfg| Config {
            site_name: "Casa <\"Pepe\"> & 'co' larga".into(),
            language: "es".into(),
            ..cfg
        },
        &[],
    );
    let page = String::from_utf8(env.get("/index.html").await.body.to_vec()).unwrap();
    assert!(page.contains("<html lang=\"es\">"));
    assert!(
        page.contains("<title>Casa &lt;&quot;Pepe&quot;&gt; &amp; &#x27;co&#x27; larga</title>")
    );
    assert!(!page.contains("{{"));
    let manifest = env.get("/manifest.webmanifest").await;
    assert_eq!(manifest.header("content-type"), "application/manifest+json");
    let body = manifest.json();
    assert_eq!(body["short_name"], "Casa <\"Pepe\"");
    assert_eq!(body["lang"], "es");
    assert_eq!(body["icons"].as_array().unwrap().len(), 4);
    assert_eq!(body["shortcuts"][1]["icons"][0]["src"], "/icon-192.png");
}

#[tokio::test]
async fn clip_api_flow() {
    let (env, _) = Env::enrolled().await;
    assert_eq!(env.post("/api/clips/stop", json!({})).await.code(), 404);
    assert_eq!(
        env.post("/api/clips/start", json!({"seconds": 30}))
            .await
            .code(),
        409
    );
    env.frame();
    assert_eq!(
        env.post("/api/clips/start", json!({"seconds": 1}))
            .await
            .code(),
        422
    );
    let started = env.post("/api/clips/start", json!({})).await.json();
    assert_eq!(started["active"], true);
    assert_eq!(env.get("/api/status").await.json()["clip"]["active"], true);
    assert_eq!(env.post("/api/clips/stop", json!({})).await.code(), 200);
    assert_eq!(
        env.get("/api/status").await.json()["clip"],
        json!({"active": false, "id": null, "until": 0, "started": 0})
    );
}

#[tokio::test]
async fn photo_lifecycle() {
    let (env, _) = Env::enrolled().await;
    assert_eq!(env.post("/api/photos", json!({})).await.code(), 503);
    env.frame();
    let shot = env.post("/api/photos", json!({})).await.json();
    let id = shot["id"].as_i64().unwrap();
    assert!(shot["size"].as_i64().unwrap() > 1000);
    let listed = env.get("/api/photos").await.json();
    assert_eq!(listed["photos"][0]["id"], id);
    let image = env.get(&format!("/api/photos/{id}/img")).await;
    assert_eq!(image.header("content-type"), "image/jpeg");
    assert_eq!(image.header("cache-control"), "private, max-age=3600");
    assert_eq!(image.header("accept-ranges"), "bytes");
    assert!(
        !env.get(&format!("/api/photos/{id}/thumb"))
            .await
            .body
            .is_empty()
    );
    let download = env.get(&format!("/api/photos/{id}/img?download=1")).await;
    assert!(
        download
            .header("content-disposition")
            .starts_with("attachment; filename=\"photo-")
    );
    let stats = env.get("/api/status").await.json()["storage"].clone();
    assert_eq!(stats["photos"], 1);
    assert_eq!(env.delete(&format!("/api/photos/{id}")).await.code(), 200);
    assert_eq!(env.delete(&format!("/api/photos/{id}")).await.code(), 404);
    assert_eq!(env.get(&format!("/api/photos/{id}/img")).await.code(), 404);
    assert_eq!(env.get("/api/photos/abc/img").await.code(), 422);
}

#[tokio::test]
async fn photo_bulk_delete_and_limit() {
    let (env, _) = Env::enrolled().await;
    env.frame();
    let mut ids = Vec::new();
    for _ in 0..3 {
        ids.push(
            env.post("/api/photos", json!({})).await.json()["id"]
                .as_i64()
                .unwrap(),
        );
    }
    let answer = env
        .post("/api/photos/delete", json!({"ids": [ids[0], ids[1], 999]}))
        .await;
    assert_eq!(answer.json(), json!({"removed": 2}));
    assert_eq!(
        env.post("/api/photos/delete", json!({"ids": []}))
            .await
            .code(),
        422
    );
    let limited = Env::anonymous();
    let _device = limited.enroll().await;
    limited.frame();
    limited.rt().photo_limit.store(0, Ordering::Relaxed);
    let refused = limited.post("/api/photos", json!({})).await;
    assert_eq!(
        (refused.code(), refused.json()),
        (409, json!({"detail": "photo limit of 0 reached"}))
    );
}

#[tokio::test]
async fn push_test_uses_the_configured_language() {
    let (env, _) = Env::enrolled().await;
    env.rt().cfg.write().unwrap().language = "es".into();
    assert_eq!(
        env.post("/api/push/test", json!({})).await.json(),
        json!({"sent": 1})
    );
    let sent = env.sent.lock().unwrap();
    assert_eq!(sent[0].0, "Prueba de avisos");
    assert_eq!(sent[0].1, "Los avisos de Room camera funcionan");
    assert_eq!(sent[0].2, "/");
}

#[tokio::test]
async fn push_subscriptions_must_be_public_https() {
    let (env, _) = Env::enrolled().await;
    for endpoint in [
        "http://push.example.org/x",
        "https://127.0.0.1/x",
        "https://192.168.1.5/x",
        "https://localhost/x",
        "ftp://a/b",
    ] {
        let answer = env
            .post(
                "/api/push/subscribe",
                json!({"subscription": {"endpoint": endpoint}}),
            )
            .await;
        assert_eq!(
            (answer.code(), answer.json()),
            (400, json!({"detail": "invalid subscription"}))
        );
    }
    assert_eq!(
        env.post(
            "/api/push/subscribe",
            json!({"subscription": {"endpoint": 5}})
        )
        .await
        .code(),
        400
    );
    assert_eq!(
        env.post("/api/push/subscribe", json!({"subscription": "x"}))
            .await
            .code(),
        422
    );
    let endpoint = "https://fcm.googleapis.com/send/abc";
    assert_eq!(
        env.post(
            "/api/push/subscribe",
            json!({"subscription": {"endpoint": endpoint, "keys": {}}})
        )
        .await
        .code(),
        200
    );
    assert_eq!(env.app.notifier.web.count(), 1);
    let key = env.get("/api/push/key").await.json()["key"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(key, env.app.notifier.web.public_key());
    assert_eq!(
        env.post("/api/push/unsubscribe", json!({"endpoint": endpoint}))
            .await
            .json(),
        json!({"ok": true})
    );
    assert_eq!(env.app.notifier.web.count(), 0);
}

#[tokio::test]
async fn everything_requires_a_session() {
    let env = Env::anonymous();
    for path in [
        "/api/status",
        "/api/events",
        "/api/live",
        "/api/snapshot",
        "/api/settings",
        "/api/push/key",
        "/api/photos",
        "/api/cameras",
        "/api/auth/devices",
        "/api/events/1/clip",
        "/api/events/count",
        "/api/photos/1/img",
    ] {
        let answer = env.get(path).await;
        assert_eq!(
            (answer.code(), answer.json()),
            (401, json!({"detail": "login required"})),
            "{path}"
        );
    }
    for path in [
        "/api/mode",
        "/api/snooze",
        "/api/privacy",
        "/api/settings",
        "/api/clips/start",
        "/api/clips/stop",
        "/api/photos",
        "/api/photos/delete",
        "/api/events/delete",
        "/api/events/1/keep",
        "/api/push/subscribe",
        "/api/push/unsubscribe",
        "/api/push/test",
        "/api/auth/logout",
        "/api/auth/enroll-code",
    ] {
        assert_eq!(env.post(path, json!({})).await.code(), 401, "{path}");
    }
    for path in [
        "/api/events/1",
        "/api/photos/1",
        "/api/auth/sessions/1",
        "/api/auth/credentials/x",
    ] {
        assert_eq!(env.delete(path).await.code(), 401, "{path}");
    }
    assert_eq!(env.get("/healthz").await.code(), 200);
    assert_eq!(env.get("/").await.code(), 200);
    let state = env.get("/api/auth/state").await.json();
    assert_eq!(
        state,
        json!({"passkeys": false, "password": false, "logged_in": false, "site_name": "Room camera", "language": "en"})
    );
}

#[tokio::test]
async fn register_login_logout() {
    let env = Env::anonymous();
    let mut device = env.enroll().await;
    assert_eq!(env.get("/api/status").await.code(), 200);
    let first = env.cookies.lock().unwrap().clone();
    env.forget_cookies();
    let options = env.post("/api/auth/login/options", json!({})).await.json();
    assert_eq!(options["cid"].as_str().unwrap().len(), 22);
    assert_eq!(options["options"]["rpId"], "localhost");
    let assertion = device.get(&options["options"]);
    let login = env
        .post(
            "/api/auth/login/verify",
            json!({"cid": options["cid"], "credential": assertion}),
        )
        .await;
    assert_eq!(login.code(), 200);
    assert!(
        login
            .header("set-cookie")
            .ends_with("; HttpOnly; Max-Age=2592000; Path=/; SameSite=strict")
    );
    assert_eq!(env.get("/api/status").await.code(), 200);
    let devices = env.get("/api/auth/devices").await.json();
    assert_eq!(devices["sessions"].as_array().unwrap().len(), 2);
    assert_eq!(devices["credentials"][0]["name"], "test");
    assert_eq!(devices["password"], false);
    let logout = env.post("/api/auth/logout", json!({})).await;
    assert!(
        logout
            .header("set-cookie")
            .starts_with("cam_session=\"\"; expires=")
    );
    assert!(
        logout
            .header("set-cookie")
            .ends_with("; Max-Age=0; Path=/; SameSite=lax")
    );
    assert_eq!(env.get("/api/status").await.code(), 401);
    *env.cookies.lock().unwrap() = first;
    assert_eq!(env.get("/api/status").await.code(), 200);
}

#[tokio::test]
async fn secure_cookies_are_marked() {
    let env = Env::with(
        |cfg| Config {
            secure_cookies: true,
            ..cfg
        },
        &[],
    );
    env.app.auth.set_password("correct horse battery").unwrap();
    let login = env
        .post(
            "/api/auth/login/password",
            json!({"password": "correct horse battery"}),
        )
        .await;
    assert!(
        login
            .header("set-cookie")
            .ends_with("; SameSite=strict; Secure")
    );
}

#[tokio::test]
async fn bad_signature_and_replay_are_rejected() {
    let env = Env::anonymous();
    let mut device = env.enroll().await;
    env.forget_cookies();
    let options = env.post("/api/auth/login/options", json!({})).await.json();
    let corrupt = device.get_with(&options["options"], 0x05, true);
    let answer = env
        .post(
            "/api/auth/login/verify",
            json!({"cid": options["cid"], "credential": corrupt}),
        )
        .await;
    assert_eq!(
        (answer.code(), answer.json()),
        (401, json!({"detail": "login failed"}))
    );
    let options = env.post("/api/auth/login/options", json!({})).await.json();
    let credential = device.get(&options["options"]);
    let body = json!({"cid": options["cid"], "credential": credential});
    assert_eq!(
        env.post("/api/auth/login/verify", body.clone())
            .await
            .code(),
        200
    );
    env.forget_cookies();
    assert_eq!(env.post("/api/auth/login/verify", body).await.code(), 401);
}

#[tokio::test]
async fn enroll_codes_are_single_use_and_duplicates_are_400() {
    let env = Env::anonymous();
    let answer = env
        .post("/api/auth/register/options", json!({"code": "deadbeef"}))
        .await;
    assert_eq!(
        (answer.code(), answer.json()),
        (400, json!({"detail": "invalid or expired code"}))
    );
    let device = SoftAuthenticator::new("localhost", ORIGIN);
    let code = env.app.auth.new_enroll_code().unwrap();
    let options = env
        .post("/api/auth/register/options", json!({"code": code}))
        .await;
    assert_eq!(options.header("content-type"), "application/json");
    let text = String::from_utf8(options.body.to_vec()).unwrap();
    assert!(text.starts_with("{\"rp\": {\"name\": \"Room camera\", \"id\": \"localhost\"}, \"user\": {\"id\": \"b3duZXI\""));
    let body = json!({"code": code, "credential": device.create(&options.json())});
    assert_eq!(
        env.post("/api/auth/register/verify", body.clone())
            .await
            .code(),
        200
    );
    assert_eq!(
        env.post("/api/auth/register/verify", body).await.code(),
        400
    );
    let code = env.app.auth.new_enroll_code().unwrap();
    let options = env
        .post("/api/auth/register/options", json!({"code": code}))
        .await
        .json();
    let again = env
        .post(
            "/api/auth/register/verify",
            json!({"code": code, "credential": device.create(&options)}),
        )
        .await;
    assert_eq!(again.code(), 400);
    assert!(
        again.json()["detail"]
            .as_str()
            .unwrap()
            .starts_with("registration failed: ")
    );
    assert_eq!(
        env.post("/api/auth/register/options", json!({"code": "ab"}))
            .await
            .code(),
        422
    );
    assert_eq!(
        env.post(
            "/api/auth/register/verify",
            json!({"code": code, "credential": {}, "name": "x".repeat(41)})
        )
        .await
        .code(),
        422
    );
}

#[tokio::test]
async fn wrong_origin_is_blocked_on_writes() {
    let (env, _) = Env::enrolled().await;
    let body = json!({"mode": "armed"}).to_string().into_bytes();
    let evil = env
        .raw(
            "POST",
            "/api/mode",
            body.clone(),
            &[("origin", "https://evil.example")],
        )
        .await;
    assert_eq!(
        (evil.code(), evil.json()),
        (403, json!({"detail": "bad origin"}))
    );
    assert_eq!(evil.header("x-frame-options"), "DENY");
    assert_eq!(
        env.raw("POST", "/api/mode", body.clone(), &[]).await.code(),
        403
    );
    assert_eq!(
        env.raw("POST", "/nowhere", body.clone(), &[]).await.code(),
        403
    );
    assert_eq!(
        env.raw("PUT", "/api/mode", body.clone(), &[]).await.code(),
        403
    );
    assert_eq!(
        env.post("/api/mode", json!({"mode": "armed"})).await.code(),
        200
    );
    let other = [
        ("origin", "http://192.168.1.50:8180"),
        ("host", "192.168.1.50:8180"),
    ];
    let mut request = Request::builder().method("POST").uri("/api/mode");
    for (k, v) in other {
        request = request.header(k, v);
    }
    let cookie = env
        .cookies
        .lock()
        .unwrap()
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ");
    let same_host = env
        .send(
            request
                .header("cookie", &cookie)
                .body(Body::from(body.clone()))
                .unwrap(),
        )
        .await;
    assert_eq!(same_host.code(), 200);
    let forged = Request::builder()
        .method("POST")
        .uri("/api/mode")
        .header("origin", "http://evil.example")
        .header("host", "192.168.1.50:8180")
        .header("cookie", &cookie)
        .body(Body::from(body))
        .unwrap();
    assert_eq!(env.send(forged).await.code(), 403);
}

#[tokio::test]
async fn unknown_methods_and_routes() {
    let (env, _) = Env::enrolled().await;
    let put = env
        .raw("PUT", "/api/mode", Vec::new(), &[("origin", ORIGIN)])
        .await;
    assert_eq!(
        (put.code(), put.json()),
        (405, json!({"detail": "Method Not Allowed"}))
    );
    assert_eq!(put.header("cache-control"), "no-store");
    let get = env.get("/api/mode").await;
    assert_eq!(
        (get.code(), get.json()),
        (404, json!({"detail": "Not Found"}))
    );
    let post = env.post("/style.css", json!({})).await;
    assert_eq!(
        (post.code(), post.json()),
        (405, json!({"detail": "Method Not Allowed"}))
    );
    let head = env
        .send(
            env.request("HEAD", "/healthz", &[])
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(head.code(), 200);
}

#[tokio::test]
async fn attempts_are_rate_limited_per_client() {
    let env = Env::anonymous();
    let mut codes = Vec::new();
    for i in 0..14 {
        codes.push(
            env.post(
                "/api/auth/register/options",
                json!({"code": format!("abcd{i:04}")}),
            )
            .await
            .code(),
        );
    }
    assert_eq!(codes.iter().filter(|c| **c == 429).count(), 4);
    let answer = env
        .post("/api/auth/login/password", json!({"password": "x"}))
        .await;
    assert_eq!(
        (answer.code(), answer.json()),
        (429, json!({"detail": "too many attempts"}))
    );
}

#[tokio::test]
async fn the_last_way_in_cannot_be_removed() {
    let (env, _) = Env::enrolled().await;
    let credential = env.get("/api/auth/devices").await.json()["credentials"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let answer = env
        .delete(&format!("/api/auth/credentials/{credential}"))
        .await;
    assert_eq!(
        (answer.code(), answer.json()),
        (
            400,
            json!({"detail": "cannot remove the last way to sign in"})
        )
    );
    env.app.auth.set_password("a long enough password").unwrap();
    assert_eq!(
        env.delete(&format!("/api/auth/credentials/{credential}"))
            .await
            .code(),
        200
    );
    assert!(
        env.get("/api/auth/devices").await.json()["credentials"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        env.delete("/api/auth/sessions/999").await.json(),
        json!({"ok": true})
    );
    assert_eq!(env.delete("/api/auth/sessions/abc").await.code(), 422);
}

#[tokio::test]
async fn password_login() {
    let env = Env::anonymous();
    assert_eq!(env.get("/api/auth/state").await.json()["password"], false);
    env.app.auth.set_password("correct horse battery").unwrap();
    assert_eq!(env.get("/api/auth/state").await.json()["password"], true);
    let wrong = env
        .post("/api/auth/login/password", json!({"password": "wrong"}))
        .await;
    assert_eq!(
        (wrong.code(), wrong.json()),
        (401, json!({"detail": "login failed"}))
    );
    assert_eq!(
        env.post("/api/auth/login/password", json!({"password": ""}))
            .await
            .code(),
        422
    );
    assert_eq!(
        env.post(
            "/api/auth/login/password",
            json!({"password": "x".repeat(257)})
        )
        .await
        .code(),
        422
    );
    assert_eq!(env.get("/api/status").await.code(), 401);
    assert_eq!(
        env.post(
            "/api/auth/login/password",
            json!({"password": "correct horse battery"})
        )
        .await
        .code(),
        200
    );
    assert_eq!(env.get("/api/status").await.code(), 200);
    assert_eq!(env.get("/api/auth/state").await.json()["logged_in"], true);
    let code = env.post("/api/auth/enroll-code", json!({})).await.json();
    assert_eq!(code["code"].as_str().unwrap().len(), 8);
    assert_eq!(code["expires_in"], 600);
}

#[test]
fn the_client_address_comes_from_the_proxy_not_from_the_client() {
    use crate::app::client_ip;
    let mut forged = HeaderMap::new();
    forged.insert("x-forwarded-for", "1.2.3.4, 203.0.113.9".parse().unwrap());
    forged.insert("cf-connecting-ip", "6.6.6.6".parse().unwrap());
    let peer = Some("10.0.0.2:5000".parse().unwrap());
    assert_eq!(client_ip(&forged, peer, true), "203.0.113.9");
    assert_eq!(client_ip(&forged, peer, false), "10.0.0.2");
    let mut cf = HeaderMap::new();
    cf.insert("cf-connecting-ip", "6.6.6.6".parse().unwrap());
    assert_eq!(client_ip(&cf, peer, true), "6.6.6.6");
    let mut empty = HeaderMap::new();
    empty.insert("x-forwarded-for", " , ".parse().unwrap());
    assert_eq!(client_ip(&empty, peer, true), "10.0.0.2");
    assert_eq!(client_ip(&HeaderMap::new(), None, false), "unknown");
}

#[test]
fn a_loopback_proxy_is_trusted_like_uvicorn_did() {
    use crate::app::client_ip;
    let local = Some("127.0.0.1:5000".parse().unwrap());
    let mut chain = HeaderMap::new();
    chain.insert(
        "x-forwarded-for",
        "1.2.3.4, 203.0.113.9:4444, 127.0.0.1".parse().unwrap(),
    );
    assert_eq!(client_ip(&chain, local, false), "203.0.113.9");
    let mut only_local = HeaderMap::new();
    only_local.insert("x-forwarded-for", "127.0.0.1, 127.0.0.1".parse().unwrap());
    assert_eq!(client_ip(&only_local, local, false), "127.0.0.1");
    let mut v6 = HeaderMap::new();
    v6.insert("x-forwarded-for", "[2001:db8::7]:80".parse().unwrap());
    assert_eq!(client_ip(&v6, local, false), "2001:db8::7");
    assert_eq!(client_ip(&HeaderMap::new(), local, false), "127.0.0.1");
    let lan = Some("192.168.1.9:5000".parse().unwrap());
    assert_eq!(client_ip(&chain, lan, false), "192.168.1.9");
}

#[tokio::test]
async fn events_clip_range_thumb_and_delete() {
    let (env, _) = Env::enrolled().await;
    let folder = env.cfg().clips_dir().join("2026/10/12");
    std::fs::create_dir_all(&folder).unwrap();
    let (clip, thumb) = (folder.join("1.mp4"), folder.join("1.jpg"));
    let content: Vec<u8> = (0..40).flat_map(|_| 0..=255u8).collect();
    std::fs::write(&clip, &content).unwrap();
    std::fs::write(&thumb, sample_jpeg()).unwrap();
    let id = env.insert_event(
        100.0,
        110.0,
        &[],
        false,
        &clip.to_string_lossy(),
        &thumb.to_string_lossy(),
    );
    let event = env.get("/api/events").await.json()["events"][0].clone();
    assert_eq!(event["duration"], 10.0);
    assert_eq!(event["labels"], json!([]));
    let full = env.get(&format!("/api/events/{id}/clip")).await;
    assert_eq!(
        (full.code(), full.header("content-type")),
        (200, "video/mp4")
    );
    assert_eq!(full.body.len(), content.len());
    assert_eq!(full.header("cache-control"), "private, max-age=3600");
    let part = env
        .get_with(
            &format!("/api/events/{id}/clip"),
            &[("range", "bytes=0-99")],
        )
        .await;
    assert_eq!((part.code(), part.body.len()), (206, 100));
    assert_eq!(
        part.header("content-range"),
        format!("bytes 0-99/{}", content.len())
    );
    assert_eq!(part.body.as_ref(), &content[..100]);
    let tail = env
        .get_with(
            &format!("/api/events/{id}/clip"),
            &[("range", "bytes=10200-")],
        )
        .await;
    assert_eq!(tail.body.as_ref(), &content[10200..]);
    let beyond = env
        .get_with(
            &format!("/api/events/{id}/clip"),
            &[("range", "bytes=999999-")],
        )
        .await;
    assert_eq!(beyond.code(), 416);
    let download = env
        .get(&format!("/api/events/{id}/clip?download=true"))
        .await;
    assert_eq!(
        download.header("content-disposition"),
        format!("attachment; filename=\"event-{id}.mp4\"")
    );
    assert_eq!(
        env.get(&format!("/api/events/{id}/thumb"))
            .await
            .header("content-type"),
        "image/jpeg"
    );
    assert_eq!(env.delete(&format!("/api/events/{id}")).await.code(), 200);
    assert!(!clip.exists());
    assert_eq!(env.get(&format!("/api/events/{id}/clip")).await.code(), 404);
    assert_eq!(
        env.delete(&format!("/api/events/{id}")).await.json(),
        json!({"detail": "not found"})
    );
    let bad = env.get("/api/events/abc/clip").await;
    assert_eq!(bad.code(), 422);
    assert_eq!(bad.json()["detail"][0]["loc"], json!(["path", "event_id"]));
}

#[tokio::test]
async fn paths_outside_the_clips_folder_are_refused() {
    let (env, _) = Env::enrolled().await;
    let outside = env.dir.path().join("secret.mp4");
    std::fs::write(&outside, b"secret").unwrap();
    let id = env.insert_event(100.0, 110.0, &[], false, &outside.to_string_lossy(), "");
    assert_eq!(env.get(&format!("/api/events/{id}/clip")).await.code(), 404);
    let sneaky = env.cfg().clips_dir().join("../../secret.mp4");
    let id = env.insert_event(100.0, 110.0, &[], false, &sneaky.to_string_lossy(), "");
    assert_eq!(env.get(&format!("/api/events/{id}/clip")).await.code(), 404);
    assert_eq!(
        env.get(&format!("/api/events/{id}/thumb")).await.code(),
        404
    );
}

#[tokio::test]
async fn keep_toggle() {
    let (env, _) = Env::enrolled().await;
    let id = env.insert_event(100.0, 110.0, &[], false, "", "");
    assert_eq!(
        env.post(&format!("/api/events/{id}/keep"), json!({"keep": true}))
            .await
            .json(),
        json!({"ok": true, "keep": true})
    );
    assert_eq!(
        env.get("/api/events").await.json()["events"][0]["keep"],
        true
    );
    assert_eq!(
        env.post("/api/events/999/keep", json!({"keep": true}))
            .await
            .code(),
        404
    );
    assert_eq!(
        env.post(&format!("/api/events/{id}/keep"), json!({"keep": "nah"}))
            .await
            .code(),
        422
    );
    assert_eq!(
        env.post(&format!("/api/events/{id}/keep"), json!({"keep": "off"}))
            .await
            .json()["keep"],
        false
    );
}

#[tokio::test]
async fn bulk_delete_and_unseen_count() {
    let (env, _) = Env::enrolled().await;
    let now = clock::now();
    let ids: Vec<i64> = (0..3)
        .map(|i| {
            env.insert_event(
                now - f64::from(i) * 100.0,
                now - f64::from(i) * 100.0 + 5.0,
                &[],
                false,
                "",
                "",
            )
        })
        .collect();
    assert_eq!(
        env.get(&format!("/api/events/count?since={}", now - 150.0))
            .await
            .json(),
        json!({"count": 2})
    );
    assert_eq!(
        env.post("/api/events/delete", json!({"ids": [ids[0], ids[1], 9999]}))
            .await
            .json(),
        json!({"removed": 2})
    );
    assert_eq!(
        env.get("/api/events").await.json()["events"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        env.post(
            "/api/events/delete",
            json!({"ids": (0..201).collect::<Vec<_>>()})
        )
        .await
        .code(),
        422
    );
}

#[tokio::test]
async fn label_filter_and_hidden_noise() {
    let (env, _) = Env::enrolled().await;
    let person = env.insert_event(300.0, 310.0, &["person"], true, "", "");
    env.insert_event(200.0, 210.0, &["cat"], true, "", "");
    let noise = env.insert_event(100.0, 110.0, &[], true, "", "");
    let found = env.get("/api/events?label=person").await.json();
    assert_eq!(
        found["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        [person]
    );
    let quoted = env.get("/api/events?label=%22person").await.json();
    assert_eq!(quoted["events"].as_array().unwrap().len(), 1);
    let visible = env.get("/api/events?hide_unconfirmed=yes").await.json()["events"]
        .as_array()
        .unwrap()
        .clone();
    assert!(visible.iter().all(|e| e["id"] != noise) && visible.len() == 2);
    assert_eq!(
        env.get("/api/events/count?since=0").await.json()["count"],
        2
    );
    let bad = env.get("/api/events?limit=abc").await;
    assert_eq!(
        bad.json(),
        json!({"detail": [{"type": "int_parsing", "loc": ["query", "limit"], "msg": "Input should be a valid integer, unable to parse string as an integer", "input": "abc"}]})
    );
}

#[tokio::test]
async fn events_can_be_listed_for_a_time_range() {
    let (env, _) = Env::enrolled().await;
    let now = 1_000_000.0;
    for offset in [0.0, 3600.0, 7200.0] {
        env.insert_event(now + offset, now + offset + 5.0, &[], false, "", "");
    }
    let got = env
        .get(&format!(
            "/api/events?after={}&before={}",
            now + 3000.0,
            now + 7000.0
        ))
        .await
        .json();
    let starts: Vec<f64> = got["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["start"].as_f64().unwrap())
        .collect();
    assert_eq!(starts, [now + 3600.0]);
}

#[tokio::test]
async fn settings_apply_live() {
    let (env, _) = Env::enrolled().await;
    let answer = env.post("/api/settings", json!({"post_roll_s": 25, "notify_motion": false, "camera_brightness": 1, "quiet_start": "22:30"})).await;
    assert_eq!(answer.code(), 200, "{:?}", answer.body);
    let got = env.get("/api/settings").await.json();
    assert_eq!(got["post_roll_s"], 25.0);
    assert_eq!(got["notify_motion"], false);
    assert_eq!(got["camera_brightness"], 1);
    assert_eq!(got["sensitivity"], 5);
    assert_eq!(got["available_labels"].as_array().unwrap().len(), 80);
    assert_eq!(env.cfg().post_roll_s, 25.0);
    let applied = env
        .rt()
        .prefs_applied
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert!(applied.contains("camera_brightness") && applied.contains("post_roll_s"));
}

#[tokio::test]
async fn motion_settings_roundtrip_partial_updates_and_validation() {
    let (env, _) = Env::enrolled().await;
    assert_eq!(
        env.post(
            "/api/settings",
            json!({"sensitivity": 8, "ignore_rects": [[0.1, 0.1, 0.2, 0.2]], "retention_days": 30})
        )
        .await
        .code(),
        200
    );
    env.post("/api/settings", json!({"pre_roll_s": 5})).await;
    let got = env.get("/api/settings").await.json();
    assert_eq!(
        (
            got["sensitivity"].clone(),
            got["ignore_rects"].clone(),
            got["retention_days"].clone()
        ),
        (json!(8), json!([[0.1, 0.1, 0.2, 0.2]]), json!(30))
    );
    let bad = env
        .post("/api/settings", json!({"ignore_rects": [[2, 0, 0, 0]]}))
        .await;
    assert_eq!(
        (bad.code(), bad.json()),
        (400, json!({"detail": "invalid rect"}))
    );
    assert_eq!(
        env.post(
            "/api/settings",
            json!({"alert_rects": [[0.1, 0.2, 0.3, 0.4]]})
        )
        .await
        .code(),
        200
    );
    assert_eq!(
        env.post(
            "/api/settings",
            json!({"alert_rects": [[0.1, 0.2, 0.3, 2]]})
        )
        .await
        .code(),
        400
    );
    assert_eq!(
        env.get("/api/settings").await.json()["alert_rects"],
        json!([[0.1, 0.2, 0.3, 0.4]])
    );
}

#[tokio::test]
async fn invalid_settings_are_rejected() {
    let (env, _) = Env::enrolled().await;
    let cases = [
        (json!({"post_roll_s": 9999}), 400),
        (json!({"quiet_start": "25:00"}), 400),
        (json!({"notify_motion": "yes"}), 422),
        (json!({"camera_brightness": 3}), 400),
        (json!({"nope": 1}), 422),
        (json!({"detection": "maybe"}), 400),
        (json!({"detect_labels": ["unicorn"]}), 400),
        (json!({"detect_labels": []}), 400),
        (json!({"language": "xx"}), 400),
        (json!({"detect_confidence": 0.99}), 400),
        (json!({"camera_framesize": 640}), 400),
        (json!({"camera_quality": 3}), 400),
        (json!({"camera_quality": true}), 400),
        (json!({"sensitivity": 11}), 422),
        (json!([1, 2]), 422),
        (json!(null), 422),
    ];
    for (body, status) in cases {
        assert_eq!(
            env.post("/api/settings", body.clone()).await.code(),
            status,
            "{body}"
        );
    }
    for ok in [
        json!({"camera_quality": 0}),
        json!({"camera_quality": 12}),
        json!({"camera_framesize": 8}),
        json!({"detection": null}),
    ] {
        assert_eq!(
            env.post("/api/settings", ok.clone()).await.code(),
            200,
            "{ok}"
        );
    }
    let got = env.get("/api/settings").await.json();
    assert_eq!(
        (
            got["camera_quality"].clone(),
            got["camera_framesize"].clone()
        ),
        (json!(12), json!(8))
    );
}

#[tokio::test]
async fn detection_settings_are_stored_deduplicated_and_persist() {
    let (env, _) = Env::enrolled().await;
    assert_eq!(env.post("/api/settings", json!({"detection": "strict", "detect_labels": ["person", "dog", "person"], "detect_confidence": 0.6, "max_disk_gb": 3, "quiet_enabled": true})).await.code(), 200);
    let got = env.get("/api/settings").await.json();
    assert_eq!(
        (
            got["detection"].clone(),
            got["detect_labels"].clone(),
            got["detect_confidence"].clone()
        ),
        (json!("strict"), json!(["person", "dog"]), json!(0.6))
    );
    let mut fresh = Config::with_data_dir(env.cfg().data_dir);
    prefs::load(&mut fresh, env.rt().db()).unwrap();
    assert!(fresh.max_disk_gb == 3.0 && fresh.quiet_enabled && fresh.detection == "strict");
}

#[tokio::test]
async fn privacy_needs_an_enabled_camera_that_answers() {
    let (env, _) = Env::enrolled().await;
    assert_eq!(
        env.post("/api/privacy", json!({"on": true})).await.json(),
        json!({"ok": true})
    );
    assert_eq!(env.get("/api/status").await.json()["privacy"], true);
    env.rt().privacy_answers.store(false, Ordering::Relaxed);
    let answer = env.post("/api/privacy", json!({"on": false})).await;
    assert_eq!(
        (answer.code(), answer.json()),
        (502, json!({"detail": "camera did not answer"}))
    );
    assert_eq!(
        env.post("/api/settings", json!({"camera_enabled": false}))
            .await
            .code(),
        200
    );
    let answer = env.post("/api/privacy", json!({"on": false})).await;
    assert_eq!(
        (answer.code(), answer.json()),
        (409, json!({"detail": "camera not available"}))
    );
    assert_eq!(
        env.post("/api/privacy", json!({"on": "maybe"}))
            .await
            .code(),
        422
    );
}

#[tokio::test]
async fn cameras_are_listed_and_the_cookie_selects_one() {
    let env = Env::with(|cfg| cfg, &[("porch", "Porch")]);
    env.enroll().await;
    let names = |v: Value| {
        v["cameras"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["id"].as_str().unwrap().to_string(),
                    c["current"].as_bool().unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };
    let listed = env.get("/api/cameras").await.json();
    assert_eq!(
        names(listed.clone()),
        [("main".to_string(), true), ("porch".to_string(), false)]
    );
    assert_eq!(listed["cameras"][0]["name"], "Room camera");
    assert_eq!(listed["cameras"][1]["name"], "Porch");
    env.set_cookie("cam_camera", "porch");
    assert_eq!(
        names(env.get("/api/cameras").await.json()),
        [("main".to_string(), false), ("porch".to_string(), true)]
    );
    env.set_cookie("cam_camera", "gone");
    assert_eq!(
        names(env.get("/api/cameras").await.json()),
        [("main".to_string(), true), ("porch".to_string(), false)]
    );
}

#[tokio::test]
async fn each_camera_keeps_its_own_events_and_settings_and_sign_in_is_shared() {
    let env = Env::with(|cfg| cfg, &[("porch", "Porch")]);
    env.enroll().await;
    env.fakes[1].db().execute("INSERT INTO events(start_ts, end_ts, status, kind) VALUES(1000, 1005, 'done', 'motion')", []).unwrap();
    assert_eq!(env.get("/api/events").await.json()["events"], json!([]));
    env.set_cookie("cam_camera", "porch");
    assert_eq!(env.get("/api/status").await.code(), 200);
    assert_eq!(
        env.get("/api/events").await.json()["events"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        env.post("/api/settings", json!({"sensitivity": 9, "language": "es"}))
            .await
            .code(),
        200
    );
    assert_eq!(env.get("/api/auth/state").await.json()["language"], "es");
    env.set_cookie("cam_camera", "main");
    assert_ne!(env.get("/api/settings").await.json()["sensitivity"], 9);
    assert_eq!(env.get("/api/auth/state").await.json()["language"], "en");
}

#[tokio::test]
async fn hostile_bodies_never_panic() {
    let (env, _) = Env::enrolled().await;
    let routes = [
        "/api/auth/register/options",
        "/api/auth/register/verify",
        "/api/auth/login/verify",
        "/api/auth/login/password",
        "/api/mode",
        "/api/snooze",
        "/api/privacy",
        "/api/settings",
        "/api/clips/start",
        "/api/photos/delete",
        "/api/events/delete",
        "/api/events/1/keep",
        "/api/push/subscribe",
        "/api/push/unsubscribe",
    ];
    let bodies: Vec<Vec<u8>> = vec![
        b"".to_vec(), b"{".to_vec(), b"null".to_vec(), b"[]".to_vec(), b"\"x\"".to_vec(), b"1e999".to_vec(),
        b"{\"code\": {\"a\": 1}, \"credential\": [], \"cid\": 5, \"password\": null}".to_vec(),
        b"{\"credential\": {\"id\": \"\\u0000\", \"rawId\": \"%%%\", \"response\": {\"clientDataJSON\": \"AAAA\", \"attestationObject\": \"////\"}}, \"code\": \"abcdabcd\", \"cid\": \"x\"}".to_vec(),
        b"{\"ids\": [1e30, -1, \"2\"], \"seconds\": 1e400, \"minutes\": \"nan\", \"on\": 2, \"keep\": [], \"mode\": \"\\ud800\"}".to_vec(),
        b"{\"subscription\": {\"endpoint\": \"https://[::1\"}, \"endpoint\": 7}".to_vec(),
        vec![0xff, 0xfe, 0x00, 0x7b],
        "{\"sensitivity\": 1e308, \"ignore_rects\": [[1e308, -1e308, \"a\", null]], \"detect_labels\": [1]}".as_bytes().to_vec(),
    ];
    for route in routes {
        for body in &bodies {
            for content_type in ["application/json", "text/plain"] {
                let answer = env
                    .raw(
                        "POST",
                        route,
                        body.clone(),
                        &[("origin", ORIGIN), ("content-type", content_type)],
                    )
                    .await;
                assert!(
                    answer.code() < 500,
                    "{route} {:?} -> {}",
                    String::from_utf8_lossy(body),
                    answer.code()
                );
            }
        }
    }
    let huge = vec![b' '; super::MAX_BODY_BYTES + 10];
    assert_eq!(
        env.raw("POST", "/api/mode", huge, &[("origin", ORIGIN)])
            .await
            .code(),
        413
    );
    let env = Env::anonymous();
    let mut device = env.enroll().await;
    let options = env.post("/api/auth/login/options", json!({})).await.json();
    let mut assertion = device.get(&options["options"]);
    assertion["response"]["authenticatorData"] = json!("AAAA");
    assert_eq!(
        env.post(
            "/api/auth/login/verify",
            json!({"cid": options["cid"], "credential": assertion})
        )
        .await
        .code(),
        401
    );
}

#[tokio::test]
async fn validation_errors_have_the_fastapi_shape() {
    let (env, _) = Env::enrolled().await;
    let broken = env
        .raw(
            "POST",
            "/api/mode",
            b"{".to_vec(),
            &[("origin", ORIGIN), ("content-type", "application/json")],
        )
        .await;
    assert_eq!(broken.code(), 422);
    assert_eq!(broken.json()["detail"][0]["type"], "json_invalid");
    let missing = env.post("/api/mode", json!({})).await.json();
    assert_eq!(
        missing,
        json!({"detail": [{"type": "missing", "loc": ["body", "mode"], "msg": "Field required", "input": {}}]})
    );
    let empty = env
        .raw("POST", "/api/mode", Vec::new(), &[("origin", ORIGIN)])
        .await
        .json();
    assert_eq!(
        empty,
        json!({"detail": [{"type": "missing", "loc": ["body"], "msg": "Field required", "input": null}]})
    );
    let snooze = env
        .post("/api/snooze", json!({"minutes": 2000}))
        .await
        .json();
    assert_eq!(
        snooze,
        json!({"detail": [{"type": "less_than_equal", "loc": ["body", "minutes"], "msg": "Input should be less than or equal to 1440", "input": 2000, "ctx": {"le": 1440.0}}]})
    );
    let unauthenticated = Env::anonymous();
    let first = unauthenticated
        .raw(
            "POST",
            "/api/mode",
            b"{".to_vec(),
            &[("origin", ORIGIN), ("content-type", "application/json")],
        )
        .await;
    assert_eq!(first.code(), 422);
}

async fn raw_request(port: u16, request: &[u8]) -> tokio::net::TcpStream {
    use tokio::io::AsyncWriteExt;
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    stream.write_all(request).await.unwrap();
    stream
}

async fn read_all(stream: &mut tokio::net::TcpStream, within: Duration) -> Option<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut out = Vec::new();
    match tokio::time::timeout(within, stream.read_to_end(&mut out)).await {
        Ok(_) => Some(out),
        Err(_) => None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_headers_are_cut_and_connections_are_capped() {
    let env = Env::anonymous();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let limits = crate::app::Limits {
        header_read_timeout: Duration::from_millis(600),
        max_connections: 2,
    };
    let server = tokio::spawn(crate::app::serve_with(
        Arc::clone(&env.app),
        listener,
        async move {
            let _ = stop_rx.await;
        },
        limits,
    ));
    let mut slow_a = raw_request(port, b"GET /healthz HTTP/1.1\r\nHost: x\r\n").await;
    let mut slow_b = raw_request(port, b"GET /healthz HTTP/1.1\r\nHost: x\r\n").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut waiting = raw_request(
        port,
        b"GET /healthz HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(
        read_all(&mut waiting, Duration::from_millis(250))
            .await
            .is_none()
    );
    assert!(
        read_all(&mut slow_a, Duration::from_secs(5))
            .await
            .is_some()
    );
    assert!(
        read_all(&mut slow_b, Duration::from_secs(5))
            .await
            .is_some()
    );
    let answer = read_all(&mut waiting, Duration::from_secs(5))
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&answer).starts_with("HTTP/1.1 200"));
    stop_tx.send(()).unwrap();
    server.await.unwrap().unwrap();
}
