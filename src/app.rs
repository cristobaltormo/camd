use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::api::{self, ApiError, json_response};
use crate::assets;
use crate::auth::Auth;
use crate::config::{Config, SharedConfig};
use crate::db::Database;
use crate::error::{Error, Result};
use crate::notify::{Alerts, Notifier};
use crate::photos::{PhotoError, Photos};
use crate::prefs::MotionSettings;
use crate::runtime::{ClipError, Frame, ManualClip, Runtime};
use crate::viewers::{MAX_VIEWERS, ViewerPool};

pub const CAMERA_COOKIE: &str = "cam_camera";
pub const SHUTDOWN_GRACE_S: u64 = 5;
pub const SAFE_METHODS: [&str; 3] = ["GET", "HEAD", "OPTIONS"];
pub const CSP: &str = "default-src 'self'; img-src 'self' data: blob:; media-src 'self'; style-src 'self'; \
script-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";
pub const SECURITY_HEADERS: [(&str, &str); 4] = [
    ("content-security-policy", CSP),
    ("x-content-type-options", "nosniff"),
    ("referrer-policy", "no-referrer"),
    ("x-frame-options", "DENY"),
];

pub trait CameraRuntime: Send + Sync {
    fn config(&self) -> SharedConfig;
    fn db(&self) -> &Arc<Database>;
    fn photos(&self) -> &Photos;
    fn status(&self, version: &str, build: &str) -> Value;
    fn frames(&self) -> watch::Receiver<Option<Frame>>;
    fn fresh_frame(&self) -> Option<Frame>;
    fn live_available(&self) -> bool;
    fn set_mode(&self, mode: &str) -> Result<()>;
    fn snooze(&self, minutes: f64) -> Result<()>;
    fn set_privacy(&self, on: bool) -> bool;
    fn clip_start(&self, seconds: u32) -> std::result::Result<ManualClip, ClipError>;
    fn clip_stop(&self) -> Option<i64>;
    fn motion_settings(&self) -> MotionSettings;
    fn motion_settings_changed(&self, settings: MotionSettings);
    fn apply_prefs(&self, changed: &BTreeSet<String>);
    fn take_photo(&self) -> std::result::Result<Value, PhotoError>;
    fn stop(&self);
}

impl CameraRuntime for Runtime {
    fn config(&self) -> SharedConfig {
        Runtime::config(self)
    }
    fn db(&self) -> &Arc<Database> {
        Runtime::db(self)
    }
    fn photos(&self) -> &Photos {
        Runtime::photos(self)
    }
    fn status(&self, version: &str, build: &str) -> Value {
        Runtime::status(self, version, build)
    }
    fn frames(&self) -> watch::Receiver<Option<Frame>> {
        Runtime::frames(self)
    }
    fn fresh_frame(&self) -> Option<Frame> {
        Runtime::fresh_frame(self)
    }
    fn live_available(&self) -> bool {
        Runtime::live_available(self)
    }
    fn set_mode(&self, mode: &str) -> Result<()> {
        Runtime::set_mode(self, mode)
    }
    fn snooze(&self, minutes: f64) -> Result<()> {
        Runtime::snooze(self, minutes)
    }
    fn set_privacy(&self, on: bool) -> bool {
        Runtime::set_privacy(self, on)
    }
    fn clip_start(&self, seconds: u32) -> std::result::Result<ManualClip, ClipError> {
        Runtime::clip_start(self, seconds)
    }
    fn clip_stop(&self) -> Option<i64> {
        Runtime::clip_stop(self)
    }
    fn motion_settings(&self) -> MotionSettings {
        Runtime::motion_settings(self)
    }
    fn motion_settings_changed(&self, settings: MotionSettings) {
        Runtime::motion_settings_changed(self, settings)
    }
    fn apply_prefs(&self, changed: &BTreeSet<String>) {
        Runtime::apply_prefs(self, changed)
    }
    fn take_photo(&self) -> std::result::Result<Value, PhotoError> {
        Runtime::take_photo(self)
    }
    fn stop(&self) {
        Runtime::stop(self)
    }
}

pub struct Camera {
    pub id: String,
    pub runtime: Arc<dyn CameraRuntime>,
    pub viewers: Arc<ViewerPool>,
}

impl Camera {
    pub fn new(id: &str, runtime: Arc<dyn CameraRuntime>) -> Camera {
        Camera {
            id: id.to_string(),
            runtime,
            viewers: ViewerPool::new(MAX_VIEWERS),
        }
    }

    pub fn config(&self) -> SharedConfig {
        self.runtime.config()
    }
}

pub struct AppState {
    pub cameras: Vec<Camera>,
    pub auth: Arc<Auth>,
    pub notifier: Arc<Notifier>,
    pub primary: SharedConfig,
    pub build: &'static str,
    pub shutdown: watch::Sender<bool>,
}

impl AppState {
    pub fn new(cameras: Vec<Camera>, auth: Arc<Auth>, notifier: Arc<Notifier>) -> AppState {
        let primary = cameras.first().expect("at least one camera").config();
        AppState {
            cameras,
            auth,
            notifier,
            primary,
            build: assets::BUILD,
            shutdown: watch::channel(false).0,
        }
    }

    pub fn camera_index(&self, cookie: Option<&str>) -> usize {
        cookie
            .and_then(|id| self.cameras.iter().position(|c| c.id == id))
            .unwrap_or(0)
    }

    pub fn camera(&self, cookie: Option<&str>) -> &Camera {
        &self.cameras[self.camera_index(cookie)]
    }

    pub fn primary_config(&self) -> Config {
        self.primary
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

fn netloc(url: &str) -> &str {
    let rest = match url.split_once("://") {
        Some((scheme, rest))
            if !scheme.is_empty()
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) =>
        {
            rest
        }
        _ => match url.strip_prefix("//") {
            Some(rest) => rest,
            None => return "",
        },
    };
    rest.split(['/', '?', '#']).next().unwrap_or("")
}

pub fn same_origin(headers: &HeaderMap, configured_origin: &str) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return false;
    };
    let origin = String::from_utf8_lossy(origin.as_bytes());
    if origin == configured_origin {
        return true;
    }
    let host = headers
        .get(header::HOST)
        .map(|h| String::from_utf8_lossy(h.as_bytes()).into_owned());
    host.is_some_and(|host| netloc(&origin) == host)
}

pub fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>, trust_proxy: bool) -> String {
    if trust_proxy {
        let header_text = |name: &str| {
            headers
                .get(name)
                .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
                .unwrap_or_default()
        };
        let chain = header_text("x-forwarded-for");
        let forwarded = chain
            .split(',')
            .map(str::trim)
            .rfind(|part| !part.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| header_text("cf-connecting-ip"));
        if !forwarded.is_empty() {
            return forwarded;
        }
    }
    let loopback_proxy = peer.is_some_and(|p| p.ip() == std::net::Ipv4Addr::LOCALHOST);
    if loopback_proxy && let Some(host) = uvicorn_forwarded_client(headers) {
        return host;
    }
    peer.map(|p| p.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn forwarded_host(value: &str) -> &str {
    if let Some(inner) = value.strip_prefix('[') {
        return match inner.split_once(']') {
            Some((host, rest)) if rest.is_empty() || rest.starts_with(':') => host,
            _ => value,
        };
    }
    match value.rsplit_once(':') {
        Some((host, port)) if value.matches(':').count() == 1 && port.parse::<u16>().is_ok() => {
            host
        }
        _ => value,
    }
}

fn uvicorn_forwarded_client(headers: &HeaderMap) -> Option<String> {
    let values: Vec<String> = headers
        .get_all("x-forwarded-for")
        .iter()
        .map(|v| v.as_bytes().iter().map(|b| *b as char).collect())
        .collect();
    if values.is_empty() {
        return None;
    }
    let joined = values.join(", ");
    let hosts: Vec<&str> = joined.split(',').map(str::trim).collect();
    let trusted = |host: &str| {
        host.parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip == std::net::Ipv4Addr::LOCALHOST)
    };
    let chosen = hosts
        .iter()
        .rev()
        .map(|h| forwarded_host(h))
        .find(|h| !trusted(h))
        .or_else(|| hosts.first().map(|h| forwarded_host(h)))?;
    (!chosen.is_empty()).then(|| chosen.to_string())
}

pub fn manifest(cfg: &Config) -> Value {
    let icon = |src: &str, sizes: &str, purpose: &str| json!({"src": src, "sizes": sizes, "type": "image/png", "purpose": purpose});
    let icons = vec![
        icon("/icon-192.png", "192x192", "any"),
        icon("/icon-512.png", "512x512", "any"),
        icon("/icon-maskable-192.png", "192x192", "maskable"),
        icon("/icon-maskable-512.png", "512x512", "maskable"),
    ];
    let short: String = cfg.site_name.chars().take(12).collect();
    json!({
        "id": "/",
        "name": cfg.site_name,
        "short_name": short,
        "description": format!("{}: live view, events and alerts", cfg.site_name),
        "lang": cfg.language,
        "dir": "ltr",
        "start_url": "/#/live",
        "scope": "/",
        "display": "standalone",
        "orientation": "any",
        "background_color": "#14161b",
        "theme_color": "#14161b",
        "categories": ["utilities", "lifestyle"],
        "icons": icons,
        "shortcuts": [
            {"name": "Live", "url": "/#/live", "icons": [icons[0]]},
            {"name": "Events", "url": "/#/events", "icons": [icons[0]]},
        ],
    })
}

pub fn html_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            other => out.push(other),
        }
    }
    out
}

pub fn render_index(cfg: &Config) -> String {
    assets::text("index.html")
        .unwrap_or_default()
        .replace("{{site_name}}", &html_escape(&cfg.site_name))
        .replace("{{lang}}", &cfg.language)
}

fn typed(body: impl Into<Body>, content_type: &'static str) -> Response {
    let mut response = Response::new(body.into());
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

async fn healthz() -> Response {
    json_response(StatusCode::OK, &json!({"ok": true}))
}

async fn index(State(state): State<Arc<AppState>>) -> Response {
    typed(
        render_index(&state.primary_config()),
        "text/html; charset=utf-8",
    )
}

async fn web_manifest(State(state): State<Arc<AppState>>) -> Response {
    typed(
        manifest(&state.primary_config()).to_string(),
        "application/manifest+json",
    )
}

async fn service_worker() -> Response {
    let mut response = typed(
        assets::text("sw.js").unwrap_or_default(),
        "text/javascript; charset=utf-8",
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

fn method_not_allowed() -> Response {
    ApiError::new(405, "Method Not Allowed").into_response()
}

async fn static_files(
    method: Method,
    request_headers: HeaderMap,
    uri: axum::http::Uri,
) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return method_not_allowed();
    }
    let path = api::percent_decode(uri.path(), false);
    let asset = if path.ends_with('/') || path.contains("/../") || path.contains("/./") {
        None
    } else {
        assets::get(&path)
    };
    let Some(asset) = asset else {
        return ApiError::new(404, "Not Found").into_response();
    };
    let etag = format!("\"{}-{}\"", asset.etag, assets::BUILD);
    let matches = request_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .any(|t| t.trim().trim_start_matches("W/") == etag)
        });
    let mut response = if matches {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::NOT_MODIFIED;
        response
    } else {
        typed(asset.body, asset.content_type)
    };
    if let Ok(value) = HeaderValue::from_str(&etag) {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}

async fn guard(State(state): State<Arc<AppState>>, request: Request, next: Next) -> Response {
    let api_path = request.uri().path().starts_with("/api/");
    let method = request.method().clone();
    let safe = SAFE_METHODS.contains(&request.method().as_str());
    let allowed = safe || {
        let cfg = state.primary.read().unwrap_or_else(|e| e.into_inner());
        same_origin(request.headers(), &cfg.origin)
    };
    let mut response = if !allowed {
        ApiError::new(403, "bad origin").into_response()
    } else {
        next.run(request).await
    };
    if response.status() == StatusCode::METHOD_NOT_ALLOWED
        && !response.headers().contains_key(header::CONTENT_TYPE)
        && (method == Method::GET || method == Method::HEAD)
    {
        response = ApiError::new(404, "Not Found").into_response();
    } else if response.status() == StatusCode::METHOD_NOT_ALLOWED
        && !response.headers().contains_key(header::CONTENT_TYPE)
    {
        let allow = response.headers().get(header::ALLOW).cloned();
        response = method_not_allowed();
        if let Some(allow) = allow {
            response.headers_mut().insert(header::ALLOW, allow);
        }
    }
    let headers = response.headers_mut();
    for (name, value) in SECURITY_HEADERS {
        headers.insert(name, HeaderValue::from_static(value));
    }
    if !headers.contains_key(header::CACHE_CONTROL) {
        let value = if api_path {
            "no-store"
        } else {
            "private, no-cache"
        };
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(value));
    }
    response
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/manifest.webmanifest", get(web_manifest))
        .route("/sw.js", get(service_worker))
        .merge(api::router())
        .fallback(static_files)
        .layer(middleware::from_fn_with_state(Arc::clone(&state), guard))
        .with_state(state)
}

async fn shutdown_signal() {
    let interrupt = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(_) => {
                    let _ = interrupt.await;
                    return;
                }
            };
        tokio::select! {
            _ = interrupt => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = interrupt.await;
    }
}

pub fn build_state(configs: Vec<Config>) -> Result<(Arc<AppState>, Vec<Arc<Runtime>>)> {
    let Some(primary_cfg) = configs.first().cloned() else {
        return Err(Error::internal("no camera configured"));
    };
    let primary_db = Arc::new(Database::open(&primary_cfg.db_path())?);
    let notifier = Arc::new(Notifier::new(&primary_cfg, Arc::clone(&primary_db))?);
    let auth = Arc::new(Auth::new(&primary_cfg, Arc::clone(&primary_db)));
    let alerts: Arc<dyn Alerts> = notifier.clone();
    let mut runtimes = Vec::new();
    let mut cameras = Vec::new();
    for (index, cfg) in configs.into_iter().enumerate() {
        let db = if index == 0 {
            Arc::clone(&primary_db)
        } else {
            Arc::new(Database::open(&cfg.db_path())?)
        };
        let id = cfg.camera_id.clone();
        let runtime = Runtime::new(cfg.shared(), db, Arc::clone(&alerts))?;
        cameras.push(Camera::new(&id, runtime.clone()));
        runtimes.push(runtime);
    }
    Ok((Arc::new(AppState::new(cameras, auth, notifier)), runtimes))
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub header_read_timeout: Duration,
    pub max_connections: usize,
}

pub const HEADER_READ_TIMEOUT_S: u64 = 10;
pub const MAX_CONNECTIONS: usize = 256;

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            header_read_timeout: Duration::from_secs(HEADER_READ_TIMEOUT_S),
            max_connections: MAX_CONNECTIONS,
        }
    }
}

pub async fn serve(
    state: Arc<AppState>,
    listener: tokio::net::TcpListener,
    stop: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    serve_with(state, listener, stop, Limits::default()).await
}

pub async fn serve_with(
    state: Arc<AppState>,
    listener: tokio::net::TcpListener,
    stop: impl std::future::Future<Output = ()> + Send + 'static,
    limits: Limits,
) -> Result<()> {
    use hyper_util::rt::{TokioIo, TokioTimer};
    let app = router(Arc::clone(&state));
    let graceful = hyper_util::server::graceful::GracefulShutdown::new();
    let slots = Arc::new(tokio::sync::Semaphore::new(limits.max_connections.max(1)));
    let mut builder = hyper::server::conn::http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(limits.header_read_timeout);
    tokio::pin!(stop);
    loop {
        let permit = tokio::select! {
            _ = &mut stop => break,
            permit = Arc::clone(&slots).acquire_owned() => match permit {
                Ok(permit) => permit,
                Err(_) => break,
            },
        };
        let (stream, peer) = tokio::select! {
            _ = &mut stop => break,
            accepted = listener.accept() => match accepted {
                Ok(pair) => pair,
                Err(e) => {
                    crate::warning!("camd.app", "accept failed: {}", e);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            },
        };
        let _ = stream.set_nodelay(true);
        let app = app.clone();
        let service = hyper::service::service_fn(
            move |mut request: hyper::Request<hyper::body::Incoming>| {
                request
                    .extensions_mut()
                    .insert(axum::extract::ConnectInfo(peer));
                let mut app = app.clone();
                async move { tower_service::Service::call(&mut app, request).await }
            },
        );
        let connection = graceful.watch(builder.serve_connection(TokioIo::new(stream), service));
        tokio::spawn(async move {
            let _ = connection.await;
            drop(permit);
        });
    }
    drop(listener);
    let _ = state.shutdown.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(SHUTDOWN_GRACE_S), graceful.shutdown()).await;
    Ok(())
}

pub async fn run(configs: Vec<Config>) -> Result<()> {
    let (host, port) = match configs.first() {
        Some(cfg) => (cfg.listen_host.clone(), cfg.listen_port),
        None => return Err(Error::internal("no camera configured")),
    };
    let port =
        u16::try_from(port).map_err(|_| Error::invalid(format!("invalid listen_port {port}")))?;
    let (state, runtimes) = tokio::task::spawn_blocking(move || build_state(configs))
        .await
        .map_err(|e| Error::internal(e.to_string()))??;
    let listener = tokio::net::TcpListener::bind((host.as_str(), port))
        .await
        .map_err(|e| Error::internal(format!("cannot listen on {host}:{port}: {e}")))?;
    for runtime in &runtimes {
        runtime.start();
    }
    crate::info!("camd.app", "listening on {}:{}", host, port);
    let served = serve(state, listener, shutdown_signal()).await;
    crate::info!("camd.app", "stopping");
    let stopped = tokio::task::spawn_blocking(move || {
        std::thread::scope(|scope| {
            for runtime in &runtimes {
                scope.spawn(|| runtime.stop());
            }
        });
    })
    .await;
    if let Err(e) = stopped {
        crate::error!("camd.app", "stopping the cameras failed: {}", e);
    }
    served
}
