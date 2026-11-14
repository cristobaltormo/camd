use std::collections::BTreeSet;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, RwLockReadGuard, Weak};
use std::thread::JoinHandle;
use std::time::Duration;

use bytes::Bytes;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::arming::Arming;
use crate::clock;
use crate::config::{Config, SharedConfig};
use crate::db::Database;
use crate::detect::{Detection, DetectionWorker, Loader, in_zones, nanodet_loader};
use crate::error::Result;
use crate::events::{FLUSH_TIMEOUT_S, Recorder, RecorderHooks};
use crate::grabber::{CameraClient, FrameSink, Grabber};
use crate::messages::{label_names, translate};
use crate::motion::MotionDetector;
use crate::mqtt::{Commands, Mqtt};
use crate::notify::Alerts;
use crate::photos::{PhotoError, Photos};
use crate::prefs::{self, MotionSettings};
use crate::presence::Presence;
use crate::storage::{self, PURGE_INTERVAL_S};

pub const OFFLINE_ALERT_S: f64 = 300.0;
pub const DECISION_CACHE_S: f64 = 1.0;
pub const MAINTENANCE_INTERVAL_S: f64 = 10.0;
pub const CAMERA_STATUS_INTERVAL_S: f64 = 30.0;
pub const DETECT_ACTIVE_INTERVAL_S: f64 = 0.7;
pub const DETECT_IDLE_INTERVAL_S: f64 = 2.5;
pub const DETECTIONS_FRESH_S: f64 = 3.0;
pub const LIVE_FIRST_WAIT_S: f64 = 15.0;
pub const LIVE_NEXT_WAIT_S: f64 = 5.0;
pub const STOP_JOIN_S: f64 = 3.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub ts: f64,
    pub jpeg: Bytes,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ManualClip {
    pub active: bool,
    pub id: Option<i64>,
    pub until: f64,
    pub started: f64,
}

impl ManualClip {
    pub fn inactive() -> ManualClip {
        ManualClip {
            active: false,
            id: None,
            until: 0.0,
            started: 0.0,
        }
    }

    pub fn to_json(&self) -> Value {
        if self.active {
            json!({"active": true, "id": self.id, "until": self.until, "started": self.started})
        } else {
            json!({"active": false, "id": null, "until": 0, "started": 0})
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipError {
    Unavailable,
    Failed,
}

struct State {
    motion_now: bool,
    last_detections: (f64, Vec<Detection>),
    camera_info: Option<Value>,
    started_at: f64,
    enabled_at: f64,
    decision: (bool, &'static str),
    decision_at: f64,
    last_push: f64,
    last_detect: f64,
    offline_alerted: bool,
}

pub struct Runtime {
    cfg: SharedConfig,
    db: Arc<Database>,
    alerts: Arc<dyn Alerts>,
    presence: Arc<Presence>,
    arming: Arming,
    photos: Photos,
    mqtt: Mqtt,
    motion: Mutex<MotionDetector>,
    analyzer: Arc<DetectionWorker>,
    recorder: Recorder,
    grabber: Arc<Grabber>,
    state: Mutex<State>,
    pipeline: Mutex<()>,
    halted: Mutex<bool>,
    halt: Condvar,
    threads: Mutex<Vec<JoinHandle<()>>>,
    me: Weak<Runtime>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

struct Hooks(Weak<Runtime>);

impl RecorderHooks for Hooks {
    fn analysis_ready(&self) -> bool {
        self.0.upgrade().is_some_and(|rt| rt.analysis_ready())
    }

    fn on_start(&self, event_id: i64, start_ts: f64, analyzed: bool) {
        if let Some(rt) = self.0.upgrade() {
            rt.event_started(event_id, start_ts, analyzed);
        }
    }

    fn on_confirm(&self, event_id: i64, start_ts: f64, labels: &[String]) {
        if let Some(rt) = self.0.upgrade() {
            rt.event_confirmed(event_id, start_ts, labels);
        }
    }

    fn on_finish(&self, event_id: i64) {
        if let Some(rt) = self.0.upgrade() {
            rt.event_finished(event_id);
        }
    }
}

struct Sink(Weak<Runtime>);

impl FrameSink for Sink {
    fn on_frame(&self, ts: f64, jpeg: &Bytes) {
        if let Some(rt) = self.0.upgrade() {
            rt.on_frame(ts, jpeg);
        }
    }

    fn on_connect(&self) {
        let me = self.0.clone();
        let _ = std::thread::Builder::new()
            .name("on-connect".into())
            .spawn(move || {
                if let Some(rt) = me.upgrade() {
                    rt.apply_camera();
                }
            });
    }
}

impl Runtime {
    pub fn new(
        cfg: SharedConfig,
        db: Arc<Database>,
        alerts: Arc<dyn Alerts>,
    ) -> Result<Arc<Runtime>> {
        Runtime::with_loader(cfg, db, alerts, nanodet_loader())
    }

    pub fn with_loader(
        cfg: SharedConfig,
        db: Arc<Database>,
        alerts: Arc<dyn Alerts>,
        loader: Loader,
    ) -> Result<Arc<Runtime>> {
        let snapshot = {
            let mut guard = cfg.write().unwrap_or_else(|e| e.into_inner());
            prefs::load_stored(&mut guard, &db)?;
            guard.clone()
        };
        let presence = Presence::new(snapshot.presence_devices.clone(), snapshot.presence_grace_s);
        let motion = MotionDetector::new(prefs::load_motion(&db));
        let camera = CameraClient::new(
            &snapshot.camera_host,
            &snapshot.camera_token,
            snapshot.camera_api_port,
            snapshot.camera_stream_port,
        );
        let now = clock::now();
        Ok(Arc::new_cyclic(|me: &Weak<Runtime>| Runtime {
            arming: Arming::new(Arc::clone(&db), Arc::clone(&presence)),
            photos: Photos::new(Arc::clone(&cfg), Arc::clone(&db)),
            mqtt: Mqtt::new(&snapshot),
            motion: Mutex::new(motion),
            analyzer: Arc::new(DetectionWorker::with_loader(Arc::clone(&cfg), loader)),
            recorder: Recorder::new(
                Arc::clone(&cfg),
                Arc::clone(&db),
                Arc::new(Hooks(me.clone())),
            ),
            grabber: Arc::new(Grabber::new(camera)),
            state: Mutex::new(State {
                motion_now: false,
                last_detections: (0.0, Vec::new()),
                camera_info: None,
                started_at: now,
                enabled_at: 0.0,
                decision: (false, "init"),
                decision_at: 0.0,
                last_push: 0.0,
                last_detect: 0.0,
                offline_alerted: false,
            }),
            pipeline: Mutex::new(()),
            halted: Mutex::new(false),
            halt: Condvar::new(),
            threads: Mutex::new(Vec::new()),
            me: me.clone(),
            presence,
            cfg,
            db,
            alerts,
        }))
    }

    fn cfg(&self) -> RwLockReadGuard<'_, Config> {
        self.cfg.read().unwrap_or_else(|e| e.into_inner())
    }

    fn spawn(&self, name: &str, work: impl FnOnce() + Send + 'static) {
        match std::thread::Builder::new()
            .name(name.to_string())
            .spawn(work)
        {
            Ok(handle) => lock(&self.threads).push(handle),
            Err(e) => crate::error!("camd.runtime", "cannot start the {name} thread: {e}"),
        }
    }

    pub fn start(self: &Arc<Self>) {
        let (host_missing, enabled) = {
            let cfg = self.cfg();
            (
                cfg.camera_host.is_empty() || cfg.camera_token.is_empty(),
                cfg.camera_enabled,
            )
        };
        if host_missing {
            crate::warning!(
                "camd.runtime",
                "camera_host or the camera token is empty: set camera_host and CAMD_CAMERA_TOKEN"
            );
        }
        if let Err(e) = self.recorder.recover() {
            crate::error!("camd.runtime", "event recovery failed: {e}");
        }
        if let Err(e) = self.presence.start() {
            crate::error!("camd.runtime", "cannot start presence probing: {e}");
        }
        let (presence, mode, privacy) = (self.me.clone(), self.me.clone(), self.me.clone());
        self.mqtt.start(Commands {
            presence: Arc::new(move |home| {
                if let Some(rt) = presence.upgrade() {
                    rt.external_presence(home);
                }
            }),
            mode: Arc::new(move |value: &str| match mode.upgrade() {
                Some(rt) => rt.set_mode(value),
                None => Ok(()),
            }),
            privacy: Arc::new(move |on| privacy.upgrade().is_some_and(|rt| rt.set_privacy(on))),
        });
        let (analyzer, me) = (Arc::clone(&self.analyzer), self.me.clone());
        self.spawn("detector", move || {
            analyzer.run(&|ts, jpeg, found| match me.upgrade() {
                Some(rt) => rt.on_detections(ts, jpeg, found),
                None => Ok(()),
            })
        });
        self.grabber.set_enabled(enabled);
        if self.arming.privacy() && !(enabled && self.grabber.set_standby(true)) {
            if enabled {
                crate::warning!(
                    "camd.runtime",
                    "privacy is on but the camera did not confirm standby: not streaming until it does"
                );
            }
            self.grabber.mark_standby(true);
        }
        let (grabber, sink) = (Arc::clone(&self.grabber), Sink(self.me.clone()));
        self.spawn("grabber", move || grabber.run(&sink));
        let me = self.me.clone();
        self.spawn("maintenance", move || {
            let mut last_purge = 0.0;
            let mut last_status = 0.0;
            loop {
                let Some(rt) = me.upgrade() else {
                    break;
                };
                if rt.wait_halt(MAINTENANCE_INTERVAL_S) {
                    break;
                }
                rt.maintain(&mut last_purge, &mut last_status);
            }
        });
    }

    fn wait_halt(&self, seconds: f64) -> bool {
        let guard = lock(&self.halted);
        let (guard, _) = self
            .halt
            .wait_timeout_while(guard, Duration::from_secs_f64(seconds), |h| !*h)
            .unwrap_or_else(|e| e.into_inner());
        *guard
    }

    pub fn stop(&self) {
        *lock(&self.halted) = true;
        self.halt.notify_all();
        self.grabber.stop();
        self.analyzer.stop();
        self.presence.stop();
        {
            let _pipeline = lock(&self.pipeline);
            self.recorder.close_open_event();
        }
        self.recorder.flush(FLUSH_TIMEOUT_S);
        let threads: Vec<JoinHandle<()>> = lock(&self.threads).drain(..).collect();
        let deadline = clock::now() + STOP_JOIN_S;
        for handle in threads {
            while !handle.is_finished() && clock::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            if handle.is_finished() {
                let _ = handle.join();
            } else {
                let name = handle.thread().name().unwrap_or("worker").to_string();
                crate::warning!(
                    "camd.runtime",
                    "the {name} thread did not stop in time, leaving it"
                );
            }
        }
        self.mqtt.stop();
    }

    pub fn config(&self) -> SharedConfig {
        Arc::clone(&self.cfg)
    }

    pub fn db(&self) -> &Arc<Database> {
        &self.db
    }

    pub fn arming(&self) -> &Arming {
        &self.arming
    }

    pub fn photos(&self) -> &Photos {
        &self.photos
    }

    pub fn presence(&self) -> &Arc<Presence> {
        &self.presence
    }

    pub fn grabber(&self) -> &Arc<Grabber> {
        &self.grabber
    }

    pub fn recorder(&self) -> &Recorder {
        &self.recorder
    }

    pub fn analyzer(&self) -> &Arc<DetectionWorker> {
        &self.analyzer
    }

    pub fn mqtt(&self) -> &Mqtt {
        &self.mqtt
    }

    pub fn ingest(&self, ts: f64, jpeg: Bytes) -> bool {
        if !self.grabber.publish(ts, jpeg.clone()) {
            return false;
        }
        self.on_frame(ts, &jpeg);
        true
    }

    pub fn status(&self, version: &str, build: &str) -> Value {
        let (armed, reason) = self.arming.decision();
        let (enabled, detection, cap_bytes) = {
            let cfg = self.cfg();
            (
                cfg.camera_enabled,
                cfg.detection.clone(),
                storage::cap_bytes(&cfg),
            )
        };
        let last = self
            .db
            .last_visible_event()
            .ok()
            .flatten()
            .map(|row| row.to_json());
        let (events, bytes) = self.db.events_usage().unwrap_or((0, 0));
        let (photos, photo_bytes) = self.photos.usage();
        let (motion_now, info, detected) = {
            let state = lock(&self.state);
            let (at, items) = &state.last_detections;
            let fresh = clock::now() - at < DETECTIONS_FRESH_S;
            (
                state.motion_now,
                state.camera_info.clone().unwrap_or(Value::Null),
                if fresh {
                    items.iter().map(Detection::to_json).collect()
                } else {
                    Vec::new()
                },
            )
        };
        let field = |key: &str| info.get(key).cloned().unwrap_or(Value::Null);
        json!({
            "armed": armed,
            "reason": reason,
            "mode": self.arming.mode(),
            "snooze_until": self.arming.snooze_until(),
            "privacy": self.arming.privacy(),
            "presence": {"configured": self.presence.configured(), "home": self.presence.is_home()},
            "camera": {
                "enabled": enabled,
                "online": self.grabber.online(),
                "standby": self.grabber.standby(),
                "rssi": field("rssi"),
                "fw": field("fw"),
                "uptime_s": field("uptime_s"),
                "fps": round1(self.grabber.fps()),
            },
            "motion": motion_now,
            "recording": self.recorder.recording(),
            "last_event": last,
            "detection": {
                "mode": detection,
                "state": if detection == "off" { "off" } else { self.analyzer.state() },
                "error": self.analyzer.error(),
                "inference_ms": round1(self.analyzer.last_ms()),
                "items": detected,
            },
            "push_subscriptions": self.alerts.push_subscriptions(),
            "channels": self.alerts.channel_names(),
            "version": version,
            "build": build,
            "storage": {
                "events": events,
                "bytes": bytes,
                "cap_bytes": cap_bytes,
                "photos": photos,
                "photo_bytes": photo_bytes,
            },
            "clip": self.recorder.manual_state().to_json(),
        })
    }

    pub fn frames(&self) -> watch::Receiver<Option<Frame>> {
        self.grabber.subscribe()
    }

    fn private(&self) -> bool {
        self.arming.privacy() || self.grabber.standby()
    }

    pub fn fresh_frame(&self) -> Option<Frame> {
        if !self.cfg().camera_enabled || self.private() {
            return None;
        }
        let latest = self.grabber.latest()?;
        self.grabber.online().then_some(latest)
    }

    pub fn live_available(&self) -> bool {
        self.cfg().camera_enabled && !self.private()
    }

    fn reset_decision(&self) {
        lock(&self.state).decision_at = 0.0;
    }

    pub fn set_mode(&self, mode: &str) -> Result<()> {
        self.arming.set_mode(mode)?;
        self.arming.snooze(0.0)?;
        self.reset_decision();
        Ok(())
    }

    pub fn snooze(&self, minutes: f64) -> Result<()> {
        self.arming.snooze(minutes)?;
        self.reset_decision();
        Ok(())
    }

    pub fn set_privacy(&self, on: bool) -> bool {
        if !self.grabber.set_standby(on) {
            return false;
        }
        if let Err(e) = self.arming.set_privacy(on) {
            crate::error!("camd.runtime", "cannot store the privacy setting: {e}");
        }
        self.reset_decision();
        if on {
            let _pipeline = lock(&self.pipeline);
            self.recorder.close_open_event();
            lock(&self.motion).reset();
        }
        true
    }

    pub fn external_presence(&self, home: Option<bool>) {
        self.presence.set_external(home);
    }

    pub fn clip_start(&self, seconds: u32) -> std::result::Result<ManualClip, ClipError> {
        if !self.cfg().camera_enabled || self.private() || !self.grabber.online() {
            return Err(ClipError::Unavailable);
        }
        if let Err(e) = self.recorder.start_manual(seconds as f64, clock::now()) {
            crate::error!("camd.runtime", "cannot start the clip: {e}");
            return Err(ClipError::Failed);
        }
        Ok(self.recorder.manual_state())
    }

    pub fn clip_stop(&self) -> Option<i64> {
        self.recorder.stop_manual()
    }

    pub fn manual_state(&self) -> ManualClip {
        self.recorder.manual_state()
    }

    pub fn motion_settings(&self) -> MotionSettings {
        prefs::load_motion(&self.db)
    }

    pub fn motion_settings_changed(&self, settings: MotionSettings) {
        lock(&self.motion).update_settings(settings);
    }

    pub fn apply_camera(&self) -> Vec<(String, bool)> {
        if self.grabber.standby() {
            return Vec::new();
        }
        let controls: Vec<(&str, i64)> = {
            let cfg = self.cfg();
            let mut controls = vec![
                ("vflip", cfg.camera_vflip as i64),
                ("hmirror", cfg.camera_hmirror as i64),
                ("brightness", cfg.camera_brightness),
                ("contrast", cfg.camera_contrast),
            ];
            if cfg.camera_quality != 0 {
                controls.push(("quality", cfg.camera_quality));
            }
            if cfg.camera_framesize != 0 {
                controls.push(("framesize", cfg.camera_framesize));
            }
            controls
        };
        self.grabber.camera.apply_controls(&controls)
    }

    pub fn apply_prefs(&self, changed: &BTreeSet<String>) {
        let (grace, enabled) = {
            let cfg = self.cfg();
            (cfg.presence_grace_s, cfg.camera_enabled)
        };
        self.presence.set_grace_s(grace);
        if changed.contains("camera_enabled") {
            self.grabber.set_enabled(enabled);
            lock(&self.state).enabled_at = clock::now();
            if !enabled {
                let _pipeline = lock(&self.pipeline);
                self.recorder.close_open_event();
                lock(&self.motion).reset();
                lock(&self.state).offline_alerted = false;
            }
        }
        if changed.iter().any(|k| prefs::is_camera_key(k)) {
            let me = self.me.clone();
            let _ = std::thread::Builder::new()
                .name("apply-camera".into())
                .spawn(move || {
                    if let Some(rt) = me.upgrade() {
                        rt.apply_camera();
                    }
                });
        }
        if changed.iter().any(|k| prefs::is_detection_key(k)) {
            self.analyzer.poke();
        }
    }

    pub fn take_photo(&self) -> std::result::Result<Value, PhotoError> {
        let frame = self.fresh_frame().ok_or(PhotoError::NoFrame)?;
        self.photos.save(&frame.jpeg, frame.ts)
    }

    fn analysis_ready(&self) -> bool {
        self.cfg().detection != "off" && self.analyzer.ready()
    }

    fn armed(&self) -> bool {
        let now = clock::now();
        {
            let state = lock(&self.state);
            if now - state.decision_at <= DECISION_CACHE_S {
                return state.decision.0;
            }
        }
        let decision = self.arming.decision();
        let mut state = lock(&self.state);
        state.decision = decision;
        state.decision_at = now;
        decision.0
    }

    fn on_frame(&self, ts: f64, jpeg: &Bytes) {
        let _pipeline = lock(&self.pipeline);
        if self.grabber.standby() || !self.grabber.enabled() || self.grabber.halted() {
            return;
        }
        let result = lock(&self.motion).process(jpeg);
        lock(&self.state).motion_now = result.motion;
        self.request_analysis(ts, jpeg, result.motion);
        let armed = self.armed();
        if let Err(e) = self
            .recorder
            .feed(ts, jpeg, result.motion, result.score, armed)
        {
            crate::error!("camd.runtime", "frame pipeline error: {e}");
        }
    }

    fn request_analysis(&self, ts: f64, jpeg: &Bytes, motion: bool) {
        if !self.analysis_ready() || !self.armed() {
            return;
        }
        let gap = if motion {
            DETECT_ACTIVE_INTERVAL_S
        } else if self.recorder.recording() {
            DETECT_IDLE_INTERVAL_S
        } else {
            return;
        };
        if ts - lock(&self.state).last_detect < gap {
            return;
        }
        if self.analyzer.submit(ts, jpeg.clone()) {
            lock(&self.state).last_detect = ts;
        }
    }

    fn on_detections(&self, ts: f64, jpeg: &Bytes, found: Vec<Detection>) -> Result<()> {
        let zones = lock(&self.motion).settings.alert_rects.clone();
        let found: Vec<Detection> = found.into_iter().filter(|d| in_zones(d, &zones)).collect();
        lock(&self.state).last_detections = (ts, found.clone());
        self.mqtt
            .publish("object", if found.is_empty() { "OFF" } else { "ON" });
        self.recorder.observe(ts, jpeg, &found)
    }

    fn may_notify(&self, now: f64) -> bool {
        let (allowed, cooldown) = {
            let cfg = self.cfg();
            (
                cfg.notify_motion && !prefs::in_quiet_hours(&cfg, now),
                cfg.push_cooldown_s,
            )
        };
        if !allowed {
            return false;
        }
        let mut state = lock(&self.state);
        if now - state.last_push < cooldown {
            return false;
        }
        state.last_push = now;
        true
    }

    fn send(&self, title: &str, body: &str, url: &str, tag: &str) -> usize {
        let (name, camera) = {
            let cfg = self.cfg();
            (cfg.camera_name.clone(), cfg.camera_id.clone())
        };
        if name.is_empty() {
            return self.alerts.send(title, body, url, tag);
        }
        self.alerts.send(
            &format!("{title} \u{b7} {name}"),
            body,
            &format!(
                "/?camera={camera}#{}",
                url.strip_prefix("/#").unwrap_or(url)
            ),
            &format!("{tag}-{camera}"),
        )
    }

    fn notify(&self, title: String, body: String, event_id: i64, tag: &'static str) {
        let me = self.me.clone();
        let _ = std::thread::Builder::new()
            .name("notify".into())
            .spawn(move || {
                if let Some(rt) = me.upgrade() {
                    rt.send(&title, &body, &format!("/#/events/{event_id}"), tag);
                }
            });
    }

    fn event_started(&self, event_id: i64, start_ts: f64, analyzed: bool) {
        self.mqtt.publish("motion", "ON");
        if analyzed || !self.may_notify(clock::now()) {
            return;
        }
        let language = self.cfg().language.clone();
        self.notify(
            translate(&language, "motion_title", &[]),
            translate(
                &language,
                "at_time",
                &[("time", &clock::local(start_ts).hhmm())],
            ),
            event_id,
            "motion",
        );
    }

    fn event_confirmed(&self, event_id: i64, start_ts: f64, labels: &[String]) {
        self.mqtt.publish("object", "ON");
        if !self.may_notify(clock::now()) {
            return;
        }
        let language = self.cfg().language.clone();
        self.notify(
            translate(
                &language,
                "object_title",
                &[("labels", &label_names(&language, labels))],
            ),
            translate(
                &language,
                "at_time",
                &[("time", &clock::local(start_ts).hhmm())],
            ),
            event_id,
            "motion",
        );
    }

    fn event_finished(&self, _event_id: i64) {
        if !self.recorder.recording() {
            self.mqtt.publish("motion", "OFF");
            self.mqtt.publish("object", "OFF");
        }
        self.purge();
    }

    fn purge(&self) {
        let cfg = self.cfg().clone();
        if let Err(e) = storage::purge(&cfg, &self.db) {
            crate::error!("camd.runtime", "purge failed: {e}");
        }
    }

    fn maintain(&self, last_purge: &mut f64, last_status: &mut f64) {
        let now = clock::now();
        let (armed, _) = self.arming.decision();
        self.mqtt.publish("armed", if armed { "ON" } else { "OFF" });
        self.mqtt.publish("mode", &self.arming.mode());
        self.mqtt
            .publish("privacy", if self.arming.privacy() { "ON" } else { "OFF" });
        self.mqtt.publish(
            "camera",
            if self.grabber.online() {
                "online"
            } else {
                "offline"
            },
        );
        let enabled = self.cfg().camera_enabled;
        if enabled
            && now - *last_status >= CAMERA_STATUS_INTERVAL_S
            && (self.grabber.online() || self.grabber.standby())
        {
            *last_status = now;
            let info = self.grabber.camera.status();
            lock(&self.state).camera_info = info;
            self.enforce_standby();
        }
        self.recorder.tick(clock::now());
        self.check_offline();
        if now - *last_purge > PURGE_INTERVAL_S {
            self.purge();
            *last_purge = now;
        }
    }

    fn enforce_standby(&self) {
        let awake = {
            let state = lock(&self.state);
            state
                .camera_info
                .as_ref()
                .is_some_and(|info| !match info.get("standby") {
                    Some(Value::Bool(b)) => *b,
                    Some(Value::Number(n)) => n.as_f64().is_some_and(|v| v != 0.0),
                    Some(Value::String(s)) => !s.is_empty(),
                    Some(Value::Array(a)) => !a.is_empty(),
                    Some(Value::Object(o)) => !o.is_empty(),
                    Some(Value::Null) | None => false,
                })
        };
        if self.arming.privacy() && !self.grabber.standby() {
            self.grabber.mark_standby(true);
        }
        if self.grabber.standby() && awake {
            crate::info!(
                "camd.runtime",
                "camera woke up while privacy is on, putting it back in standby"
            );
            self.grabber.set_standby(true);
        }
    }

    fn check_offline(&self) {
        let (enabled, notify, language) = {
            let cfg = self.cfg();
            (cfg.camera_enabled, cfg.notify_offline, cfg.language.clone())
        };
        if !enabled {
            lock(&self.state).offline_alerted = false;
            return;
        }
        let latest = self.grabber.latest_ts().unwrap_or(0.0);
        let (started_at, enabled_at) = {
            let state = lock(&self.state);
            (state.started_at, state.enabled_at)
        };
        let silent_for = clock::now() - latest.max(started_at).max(enabled_at);
        if self.grabber.standby() || silent_for < OFFLINE_ALERT_S {
            let back = self.grabber.online() && {
                let mut state = lock(&self.state);
                std::mem::replace(&mut state.offline_alerted, false)
            };
            if back && notify {
                self.send(
                    &translate(&language, "back_title", &[]),
                    &translate(&language, "back_body", &[]),
                    "/",
                    "offline",
                );
            }
            return;
        }
        let first = !std::mem::replace(&mut lock(&self.state).offline_alerted, true);
        if first && notify {
            self.send(
                &translate(&language, "offline_title", &[]),
                &translate(&language, "offline_body", &[]),
                "/",
                "offline",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::FakeDetector;
    use crate::motion::tests::scene;

    #[derive(Default)]
    pub struct FakeAlerts {
        pub sent: Mutex<Vec<(String, String, String, String)>>,
    }

    impl Alerts for FakeAlerts {
        fn send(&self, title: &str, body: &str, url: &str, tag: &str) -> usize {
            self.sent
                .lock()
                .unwrap()
                .push((title.into(), body.into(), url.into(), tag.into()));
            1
        }
        fn push_subscriptions(&self) -> i64 {
            2
        }
        fn channel_names(&self) -> Vec<String> {
            vec!["ntfy".into()]
        }
    }

    impl FakeAlerts {
        fn titles(&self) -> Vec<String> {
            self.sent
                .lock()
                .unwrap()
                .iter()
                .map(|s| s.0.clone())
                .collect()
        }
    }

    struct Env {
        rt: Arc<Runtime>,
        alerts: Arc<FakeAlerts>,
        _dir: tempfile::TempDir,
    }

    fn env_with(tweak: impl FnOnce(&mut Config), loader: Loader) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::with_data_dir(dir.path());
        cfg.camera_token = "t".into();
        cfg.detection = "off".into();
        tweak(&mut cfg);
        let db = Arc::new(Database::open(&cfg.db_path()).unwrap());
        let alerts = Arc::new(FakeAlerts::default());
        let rt = Runtime::with_loader(cfg.shared(), db, alerts.clone(), loader).unwrap();
        Env {
            rt,
            alerts,
            _dir: dir,
        }
    }

    fn env() -> Env {
        env_with(|_| {}, FakeDetector::loader(Vec::new()))
    }

    fn frame(env: &Env) {
        assert!(
            env.rt
                .ingest(clock::now(), Bytes::from(scene(0, 90, false)))
        );
    }

    fn wait_for(seconds: f64, condition: impl Fn() -> bool) -> bool {
        let deadline = clock::now() + seconds;
        while clock::now() < deadline {
            if condition() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn stored_prefs_and_retention_override_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::with_data_dir(dir.path());
        let db = Arc::new(Database::open(&cfg.db_path()).unwrap());
        db.set_setting("retention_days", &json!(3)).unwrap();
        db.set_setting("prefs", &json!({"post_roll_s": 20.0, "language": "es"}))
            .unwrap();
        let shared = cfg.shared();
        let _rt = Runtime::new(Arc::clone(&shared), db, Arc::new(FakeAlerts::default())).unwrap();
        let cfg = shared.read().unwrap();
        assert_eq!((cfg.retention_days, cfg.post_roll_s), (3, 20.0));
        assert_eq!(cfg.language, "es");
    }

    #[test]
    fn disabling_the_camera_is_reflected_everywhere() {
        let env = env();
        frame(&env);
        assert!(env.rt.fresh_frame().is_some() && env.rt.live_available());
        env.rt.config().write().unwrap().camera_enabled = false;
        env.rt
            .apply_prefs(&BTreeSet::from(["camera_enabled".to_string()]));
        let status = env.rt.status("3.0.0", "dev");
        assert_eq!(status["camera"]["enabled"], json!(false));
        assert_eq!(status["camera"]["online"], json!(false));
        assert!(!env.rt.grabber().enabled() && env.rt.grabber().latest().is_none());
        assert!(!env.rt.live_available());
        assert!(env.rt.fresh_frame().is_none());
        assert_eq!(env.rt.take_photo(), Err(PhotoError::NoFrame));
        assert_eq!(env.rt.clip_start(30), Err(ClipError::Unavailable));
        env.rt.config().write().unwrap().camera_enabled = true;
        env.rt
            .apply_prefs(&BTreeSet::from(["camera_enabled".to_string()]));
        assert!(env.rt.grabber().enabled());
    }

    #[test]
    fn frames_in_flight_are_dropped_while_disabled() {
        let env = env();
        env.rt.grabber().set_enabled(false);
        assert!(
            !env.rt
                .ingest(clock::now(), Bytes::from(scene(0, 90, false)))
        );
        assert!(env.rt.grabber().latest().is_none() && !env.rt.grabber().online());
        env.rt.grabber().set_enabled(true);
        frame(&env);
        assert!(env.rt.grabber().online());
    }

    #[test]
    fn photos_come_from_the_fresh_frame() {
        let env = env();
        assert_eq!(env.rt.take_photo(), Err(PhotoError::NoFrame));
        frame(&env);
        let photo = env.rt.take_photo().unwrap();
        assert!(photo["size"].as_i64().unwrap() > 1000);
        let thumb = env
            .rt
            .photos()
            .file(photo["id"].as_i64().unwrap(), true)
            .unwrap();
        let small = crate::imaging::decode_rgb(&std::fs::read(thumb).unwrap()).unwrap();
        assert_eq!((small.width, small.height), (360, 270));
    }

    #[test]
    fn offline_alert_is_sent_once_and_never_while_disabled() {
        let env = env();
        lock(&env.rt.state).started_at = clock::now() - 3600.0;
        env.rt.config().write().unwrap().camera_enabled = false;
        env.rt.check_offline();
        assert!(env.alerts.titles().is_empty());
        env.rt
            .apply_prefs(&BTreeSet::from(["camera_enabled".to_string()]));
        env.rt.config().write().unwrap().camera_enabled = true;
        env.rt
            .apply_prefs(&BTreeSet::from(["camera_enabled".to_string()]));
        env.rt.check_offline();
        assert!(env.alerts.titles().is_empty());
        lock(&env.rt.state).enabled_at = clock::now() - 3600.0;
        env.rt.check_offline();
        env.rt.check_offline();
        assert_eq!(env.alerts.titles(), vec!["Camera offline"]);
    }

    #[test]
    fn recovery_notice_follows_an_alert() {
        let env = env();
        lock(&env.rt.state).started_at = clock::now() - 3600.0;
        env.rt.check_offline();
        frame(&env);
        env.rt.check_offline();
        env.rt.check_offline();
        assert_eq!(env.alerts.titles(), vec!["Camera offline", "Camera back"]);
    }

    #[test]
    fn named_cameras_tag_their_alerts() {
        let env = env_with(
            |c| {
                c.camera_id = "porch".into();
                c.camera_name = "Porch".into();
            },
            FakeDetector::loader(Vec::new()),
        );
        env.rt.send("Motion detected", "b", "/#/events/4", "motion");
        let sent = env.alerts.sent.lock().unwrap();
        assert_eq!(
            sent[0],
            (
                "Motion detected \u{b7} Porch".to_string(),
                "b".to_string(),
                "/?camera=porch#/events/4".to_string(),
                "motion-porch".to_string()
            )
        );
    }

    #[test]
    fn a_camera_that_woke_up_is_put_back_in_standby_while_private() {
        let env = env();
        env.rt.grabber().mark_standby(true);
        lock(&env.rt.state).camera_info = Some(json!({"standby": 0}));
        env.rt.enforce_standby();
        assert!(env.rt.grabber().standby());
        lock(&env.rt.state).camera_info = Some(json!({"standby": 1}));
        env.rt.enforce_standby();
        assert!(env.rt.grabber().standby());
    }

    #[test]
    fn notifications_are_localised_and_labelled() {
        let env = env();
        env.rt.config().write().unwrap().language = "es".into();
        env.rt
            .event_confirmed(7, 1000.0, &["person".into(), "cat".into()]);
        assert!(wait_for(5.0, || env.alerts.titles().len() == 1));
        assert_eq!(env.alerts.titles()[0], "Persona, Gato detectado");
        assert_eq!(env.alerts.sent.lock().unwrap()[0].2, "/#/events/7");
        env.rt.config().write().unwrap().language = "en".into();
        lock(&env.rt.state).last_push = 0.0;
        env.rt.event_confirmed(8, 1000.0, &["dog".into()]);
        assert!(wait_for(5.0, || env.alerts.titles().len() == 2));
        assert_eq!(env.alerts.titles()[1], "Dog detected");
    }

    #[test]
    fn unanalysed_events_notify_at_start_and_analysed_ones_wait() {
        let env = env();
        env.rt.event_started(1, 1000.0, true);
        std::thread::sleep(Duration::from_millis(200));
        assert!(env.alerts.titles().is_empty());
        env.rt.event_started(2, 1000.0, false);
        assert!(wait_for(5.0, || env.alerts.titles() == vec!["Motion detected"]));
    }

    #[test]
    fn cooldown_and_quiet_hours_silence_notifications() {
        let env = env();
        env.rt.event_confirmed(1, 1000.0, &["person".into()]);
        env.rt.event_confirmed(2, 1000.0, &["person".into()]);
        assert!(wait_for(5.0, || env.alerts.titles().len() == 1));
        lock(&env.rt.state).last_push = 0.0;
        env.rt.config().write().unwrap().notify_motion = false;
        env.rt.event_confirmed(3, 1000.0, &["person".into()]);
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(env.alerts.titles().len(), 1);
    }

    #[test]
    fn status_exposes_fresh_detections_only() {
        let env = env_with(
            |c| c.detection = "alert".into(),
            FakeDetector::loader(Vec::new()),
        );
        let person = Detection::new("person", 0.8, [0.1, 0.1, 0.2, 0.4]);
        lock(&env.rt.state).last_detections = (clock::now(), vec![person.clone()]);
        let status = env.rt.status("3.0.0", "abc");
        assert_eq!(status["detection"]["items"][0]["label"], json!("person"));
        assert_eq!(status["detection"]["state"], json!("idle"));
        assert_eq!(
            (status["version"].clone(), status["build"].clone()),
            (json!("3.0.0"), json!("abc"))
        );
        assert_eq!(status["push_subscriptions"], json!(2));
        assert_eq!(status["channels"], json!(["ntfy"]));
        assert_eq!(
            status["clip"],
            json!({"active": false, "id": null, "until": 0, "started": 0})
        );
        lock(&env.rt.state).last_detections = (clock::now() - 60.0, vec![person]);
        assert_eq!(
            env.rt.status("3.0.0", "abc")["detection"]["items"],
            json!([])
        );
        env.rt.config().write().unwrap().detection = "off".into();
        assert_eq!(
            env.rt.status("3.0.0", "abc")["detection"]["state"],
            json!("off")
        );
    }

    #[test]
    fn the_last_event_card_skips_unfinished_and_unconfirmed_events() {
        let env = env();
        let db = env.rt.db();
        db.execute("INSERT INTO events(start_ts, end_ts, status, analyzed, confirmed) VALUES(100, 110, 'done', 1, 1)", []).unwrap();
        db.execute("INSERT INTO events(start_ts, end_ts, status, analyzed, confirmed) VALUES(200, 210, 'done', 1, 0)", []).unwrap();
        db.execute(
            "INSERT INTO events(start_ts, status) VALUES(300, 'recording')",
            [],
        )
        .unwrap();
        assert_eq!(
            env.rt.status("3.0.0", "dev")["last_event"]["start"],
            json!(100.0)
        );
    }

    #[test]
    fn detections_outside_the_alert_zones_are_ignored() {
        let env = env();
        env.rt.motion_settings_changed(MotionSettings {
            alert_rects: vec![vec![0.0, 0.0, 0.5, 0.5]],
            ..MotionSettings::default()
        });
        let inside = Detection::new("person", 0.9, [0.1, 0.1, 0.2, 0.2]);
        let outside = Detection::new("person", 0.9, [0.7, 0.7, 0.2, 0.2]);
        env.rt
            .on_detections(clock::now(), &Bytes::new(), vec![inside.clone(), outside])
            .unwrap();
        assert_eq!(lock(&env.rt.state).last_detections.1, vec![inside]);
    }

    #[test]
    fn mode_and_snooze_reset_the_cached_decision() {
        let env = env();
        env.rt.set_mode("armed").unwrap();
        assert!(env.rt.armed());
        env.rt.snooze(10.0).unwrap();
        assert!(!env.rt.armed());
        env.rt.set_mode("armed").unwrap();
        assert!(env.rt.armed() && env.rt.arming().snooze_until() == 0.0);
        assert!(env.rt.set_mode("bogus").is_err());
        env.rt.set_mode("disarmed").unwrap();
        assert!(!env.rt.armed());
    }

    #[test]
    fn privacy_needs_the_camera_to_answer() {
        let env = env();
        assert!(!env.rt.set_privacy(true));
        assert!(!env.rt.arming().privacy());
    }

    #[test]
    fn privacy_holds_when_the_camera_is_unreachable_at_start() {
        let env = env_with(
            |c| c.camera_host = "127.0.0.1".into(),
            FakeDetector::loader(Vec::new()),
        );
        env.rt.arming().set_privacy(true).unwrap();
        env.rt.start();
        assert!(env.rt.grabber().standby());
        assert!(!env.rt.live_available());
        assert!(
            !env.rt
                .ingest(clock::now(), Bytes::from(scene(0, 90, false)))
        );
        assert!(env.rt.fresh_frame().is_none());
        assert_eq!(env.rt.status("3.0.0", "dev")["privacy"], json!(true));
        env.rt.stop();
    }

    #[test]
    fn privacy_alone_refuses_live_snapshots_photos_and_clips() {
        let env = env();
        frame(&env);
        assert!(env.rt.live_available() && env.rt.fresh_frame().is_some());
        env.rt.arming().set_privacy(true).unwrap();
        assert!(!env.rt.grabber().standby());
        assert!(!env.rt.live_available());
        assert!(env.rt.fresh_frame().is_none());
        assert_eq!(env.rt.take_photo(), Err(PhotoError::NoFrame));
        assert_eq!(env.rt.clip_start(30), Err(ClipError::Unavailable));
        env.rt.enforce_standby();
        assert!(env.rt.grabber().standby());
    }

    #[test]
    fn frames_in_flight_after_standby_never_open_an_event() {
        let env = env();
        env.rt.set_mode("armed").unwrap();
        let quiet = Bytes::from(scene(0, 90, false));
        let busy = Bytes::from(scene(0, 90, true));
        let mut t = clock::now() - 10.0;
        for _ in 0..25 {
            env.rt.on_frame(t, &quiet);
            t += 0.1;
        }
        env.rt.grabber().mark_standby(true);
        for _ in 0..10 {
            env.rt.on_frame(t, &busy);
            t += 0.1;
        }
        assert!(!env.rt.recorder().recording());
    }

    #[test]
    fn a_clip_that_cannot_start_is_an_error() {
        let env = env();
        frame(&env);
        let spool_dir = env.rt.config().read().unwrap().spool_dir();
        let _ = std::fs::remove_dir_all(&spool_dir);
        std::fs::write(&spool_dir, b"not a directory").unwrap();
        assert_eq!(env.rt.clip_start(30), Err(ClipError::Failed));
    }

    #[test]
    fn stop_closes_the_open_event_without_waiting_for_a_stuck_model_download() {
        let slow: Loader = Box::new(|_: &std::path::Path| {
            std::thread::sleep(Duration::from_secs(30));
            Err(crate::error::Error::internal("too slow"))
        });
        let env = env_with(|c| c.detection = "alert".into(), slow);
        env.rt.start();
        assert!(wait_for(5.0, || env.rt.analyzer().state() == "loading"));
        frame(&env);
        env.rt.clip_start(30).unwrap();
        let started = std::time::Instant::now();
        env.rt.stop();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(!env.rt.recorder().recording());
    }

    #[test]
    fn external_presence_drives_auto_mode() {
        let env = env();
        env.rt.external_presence(Some(false));
        assert_eq!(env.rt.arming().decision(), (true, "away"));
        env.rt.external_presence(Some(true));
        assert_eq!(env.rt.arming().decision(), (false, "home"));
    }

    #[test]
    fn manual_clips_need_a_live_camera() {
        let env = env();
        assert_eq!(env.rt.clip_start(30), Err(ClipError::Unavailable));
        frame(&env);
        let clip = env.rt.clip_start(30).unwrap();
        assert!(clip.active && clip.id.is_some());
        assert_eq!(env.rt.manual_state(), clip);
        assert_eq!(env.rt.clip_stop(), clip.id);
        assert_eq!(env.rt.clip_stop(), None);
        env.rt.recorder().flush(30.0);
    }

    #[test]
    fn detection_runs_on_motion_at_the_active_cadence() {
        let person = Detection::new("person", 0.9, [0.3, 0.3, 0.3, 0.5]);
        let env = env_with(
            |c| c.detection = "alert".into(),
            FakeDetector::loader(vec![person]),
        );
        env.rt.set_mode("armed").unwrap();
        let analyzer = Arc::clone(env.rt.analyzer());
        let me = Arc::downgrade(&env.rt);
        let worker = std::thread::spawn(move || {
            analyzer.run(&|ts, jpeg, found| match me.upgrade() {
                Some(rt) => rt.on_detections(ts, jpeg, found),
                None => Ok(()),
            })
        });
        assert!(wait_for(5.0, || env.rt.analyzer().ready()));
        let quiet = Bytes::from(scene(0, 90, false));
        let busy = Bytes::from(scene(0, 90, true));
        let mut t = clock::now() - 10.0;
        for _ in 0..25 {
            env.rt.grabber().publish(t, quiet.clone());
            env.rt.on_frame(t, &quiet);
            t += 0.1;
        }
        assert_eq!(lock(&env.rt.state).last_detect, 0.0);
        let first = t;
        for _ in 0..3 {
            env.rt.on_frame(t, &busy);
            t += 0.1;
        }
        assert_eq!(lock(&env.rt.state).last_detect, first);
        assert!(wait_for(5.0, || !lock(&env.rt.state)
            .last_detections
            .1
            .is_empty()));
        env.rt.analyzer().stop();
        worker.join().unwrap();
        env.rt.recorder().close_open_event();
        env.rt.recorder().flush(30.0);
    }
}
