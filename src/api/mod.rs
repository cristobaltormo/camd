pub mod auth;
pub mod control;
pub mod events;
pub mod files;
pub mod media;
pub mod notifications;
pub mod pyjson;
#[cfg(test)]
mod tests;
pub mod validate;

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{FromRequest, FromRequestParts, RawPathParams, Request};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use serde_json::{Value, json};

use crate::app::{AppState, Camera, client_ip};
use crate::config::Config;

pub const MAX_BODY_BYTES: usize = 256 * 1024;

#[derive(Debug)]
pub enum ApiError {
    Detail(StatusCode, String),
    Validation(Vec<Value>),
    Internal(String),
}

impl ApiError {
    pub fn new(status: u16, detail: impl Into<String>) -> ApiError {
        ApiError::Detail(
            StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            detail.into(),
        )
    }
}

impl From<crate::error::Error> for ApiError {
    fn from(e: crate::error::Error) -> Self {
        match e {
            crate::error::Error::Invalid(m) => ApiError::Detail(StatusCode::BAD_REQUEST, m),
            crate::error::Error::Internal(m) => ApiError::Internal(m),
        }
    }
}

pub fn json_response(status: StatusCode, value: &Value) -> Response {
    let mut response = Response::new(Body::from(value.to_string()));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

pub fn ok(value: Value) -> Result<Response, ApiError> {
    Ok(json_response(StatusCode::OK, &value))
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::Detail(status, detail) => json_response(status, &json!({"detail": detail})),
            ApiError::Validation(errors) => {
                json_response(StatusCode::UNPROCESSABLE_ENTITY, &json!({"detail": errors}))
            }
            ApiError::Internal(message) => {
                crate::error!("camd.app", "request failed: {}", message);
                let mut response = Response::new(Body::from("Internal Server Error"));
                *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                response.headers_mut().insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/plain; charset=utf-8"),
                );
                response
            }
        }
    }
}

pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let mut found = None;
    for value in headers.get_all(header::COOKIE) {
        let Ok(text) = value.to_str() else { continue };
        for pair in text.split(';') {
            let (key, value) = match pair.split_once('=') {
                Some((k, v)) => (k.trim(), v.trim()),
                None => ("", pair.trim()),
            };
            if key == name {
                let value = value
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .unwrap_or(value);
                found = Some(value.to_string());
            }
        }
    }
    found
}

pub fn percent_decode(text: &str, plus_is_space: bool) -> String {
    fn hex(byte: u8) -> Option<u8> {
        (byte as char).to_digit(16).map(|d| d as u8)
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'%'
            && i + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(high * 16 + low);
            i += 3;
            continue;
        }
        out.push(if byte == b'+' && plus_is_space {
            b' '
        } else {
            byte
        });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn parse_query(raw: Option<&str>) -> Vec<(String, String)> {
    raw.unwrap_or("")
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (k, v) = part.split_once('=').unwrap_or((part, ""));
            (percent_decode(k, true), percent_decode(v, true))
        })
        .collect()
}

pub enum BodyJson {
    Absent,
    Json(Value),
    Raw(String),
}

pub struct Ctx {
    pub app: Arc<AppState>,
    pub parts: Parts,
    pub body: Bytes,
    pub params: Vec<(String, String)>,
    pub query: Vec<(String, String)>,
    pub camera_index: usize,
}

impl FromRequest<Arc<AppState>> for Ctx {
    type Rejection = Response;

    async fn from_request(req: Request, state: &Arc<AppState>) -> Result<Self, Self::Rejection> {
        let (mut parts, body) = req.into_parts();
        let params = match RawPathParams::from_request_parts(&mut parts, state).await {
            Ok(raw) => raw
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            Err(_) => Vec::new(),
        };
        let body = match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
            Ok(bytes) => bytes,
            Err(_) => {
                return Err(ApiError::new(413, "request body too large").into_response());
            }
        };
        let query = parse_query(parts.uri.query());
        let camera_index =
            state.camera_index(cookie(&parts.headers, crate::app::CAMERA_COOKIE).as_deref());
        Ok(Ctx {
            app: Arc::clone(state),
            parts,
            body,
            params,
            query,
            camera_index,
        })
    }
}

impl Ctx {
    pub fn camera(&self) -> &Camera {
        &self.app.cameras[self.camera_index]
    }

    pub fn config(&self) -> Config {
        self.camera()
            .config()
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.parts.headers.get(name).and_then(|v| v.to_str().ok())
    }

    pub fn user_agent(&self) -> String {
        self.parts
            .headers
            .get(header::USER_AGENT)
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .unwrap_or_default()
    }

    pub fn param(&self, name: &str) -> &str {
        self.params
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    }

    pub fn query_value(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn body_json(&self) -> Result<BodyJson, ApiError> {
        if self.body.is_empty() {
            return Ok(BodyJson::Absent);
        }
        let is_json = match self.header("content-type") {
            None => true,
            Some(value) => {
                let mime = value
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                match mime.split_once('/') {
                    Some(("application", sub)) => sub == "json" || sub.ends_with("+json"),
                    _ => false,
                }
            }
        };
        if !is_json {
            return Ok(BodyJson::Raw(
                String::from_utf8_lossy(&self.body).into_owned(),
            ));
        }
        match serde_json::from_slice::<Value>(&self.body) {
            Ok(value) => Ok(BodyJson::Json(value)),
            Err(e) => Err(validate::json_invalid(&self.body, &e)),
        }
    }

    pub fn session(&self) -> Result<i64, ApiError> {
        let token = cookie(&self.parts.headers, crate::auth::SESSION_COOKIE);
        match self.app.auth.check_session(token.as_deref())? {
            Some(id) => Ok(id),
            None => Err(ApiError::new(401, "login required")),
        }
    }

    pub fn logged_in(&self) -> Result<bool, ApiError> {
        let token = cookie(&self.parts.headers, crate::auth::SESSION_COOKIE);
        Ok(self.app.auth.check_session(token.as_deref())?.is_some())
    }

    pub fn throttle(&self) -> Result<(), ApiError> {
        let trust_proxy = self
            .camera()
            .config()
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .trust_proxy;
        let peer = self
            .parts
            .extensions
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|c| c.0);
        let ip = client_ip(&self.parts.headers, peer, trust_proxy);
        if self.app.auth.limiter.allow(&ip) {
            Ok(())
        } else {
            Err(ApiError::new(429, "too many attempts"))
        }
    }
}

pub type Handler = fn(&Ctx) -> Result<Response, ApiError>;

pub async fn run(ctx: Ctx, handler: Handler) -> Response {
    match tokio::task::spawn_blocking(move || handler(&ctx)).await {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => error.into_response(),
        Err(_) => ApiError::Internal("handler panicked".into()).into_response(),
    }
}

macro_rules! sync {
    ($handler:path) => {
        |ctx: $crate::api::Ctx| $crate::api::run(ctx, $handler)
    };
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/auth/state", get(sync!(auth::state)))
        .route(
            "/api/auth/register/options",
            post(sync!(auth::register_options)),
        )
        .route(
            "/api/auth/register/verify",
            post(sync!(auth::register_verify)),
        )
        .route("/api/auth/login/options", post(sync!(auth::login_options)))
        .route("/api/auth/login/verify", post(sync!(auth::login_verify)))
        .route(
            "/api/auth/login/password",
            post(sync!(auth::login_password)),
        )
        .route("/api/auth/logout", post(sync!(auth::logout)))
        .route("/api/auth/enroll-code", post(sync!(auth::enroll_code)))
        .route("/api/auth/devices", get(sync!(auth::devices)))
        .route(
            "/api/auth/sessions/{session_id}",
            delete(sync!(auth::delete_session)),
        )
        .route(
            "/api/auth/credentials/{credential_id}",
            delete(sync!(auth::delete_credential)),
        )
        .route("/api/cameras", get(sync!(control::cameras)))
        .route("/api/status", get(sync!(control::status)))
        .route("/api/mode", post(sync!(control::mode)))
        .route("/api/snooze", post(sync!(control::snooze)))
        .route("/api/privacy", post(sync!(control::privacy)))
        .route(
            "/api/settings",
            get(sync!(control::get_settings)).post(sync!(control::post_settings)),
        )
        .route("/api/snapshot", get(sync!(media::snapshot)))
        .route("/api/live", get(media::live))
        .route("/api/clips/start", post(sync!(media::clip_start)))
        .route("/api/clips/stop", post(sync!(media::clip_stop)))
        .route(
            "/api/photos",
            get(sync!(media::list_photos)).post(sync!(media::take_photo)),
        )
        .route("/api/photos/delete", post(sync!(media::delete_photos)))
        .route("/api/photos/{photo_id}", delete(sync!(media::delete_photo)))
        .route("/api/photos/{photo_id}/img", get(media::photo_image))
        .route("/api/photos/{photo_id}/thumb", get(media::photo_thumb))
        .route("/api/events", get(sync!(events::list)))
        .route("/api/events/count", get(sync!(events::count)))
        .route("/api/events/delete", post(sync!(events::delete_many)))
        .route("/api/events/{event_id}", delete(sync!(events::delete_one)))
        .route("/api/events/{event_id}/keep", post(sync!(events::keep)))
        .route("/api/events/{event_id}/clip", get(events::clip))
        .route("/api/events/{event_id}/thumb", get(events::thumb))
        .route("/api/push/key", get(sync!(notifications::key)))
        .route("/api/push/subscribe", post(sync!(notifications::subscribe)))
        .route(
            "/api/push/unsubscribe",
            post(sync!(notifications::unsubscribe)),
        )
        .route("/api/push/test", post(sync!(notifications::test)))
}
