use axum::http::{HeaderValue, header};
use axum::response::Response;
use serde_json::json;

use super::validate::{Body, Check};
use super::{ApiError, Ctx, ok, pyjson};
use crate::auth::{SESSION_COOKIE, SESSION_TTL};
use crate::clock;
use crate::error::Error;

pub const ENROLL_EXPIRES_IN: i64 = 600;

pub fn session_cookie(token: &str, secure: bool) -> String {
    let mut cookie = format!(
        "{SESSION_COOKIE}={token}; HttpOnly; Max-Age={}; Path=/; SameSite=strict",
        SESSION_TTL as i64
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

pub fn http_date(ts: f64) -> String {
    let secs = ts.floor() as i64;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let weekday = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][days.rem_euclid(7) as usize];
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let name = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][(month - 1) as usize];
    format!("{weekday}, {day:02} {name} {year} {hour:02}:{minute:02}:{second:02} GMT")
}

pub fn logout_cookie(now: f64) -> String {
    format!(
        "{SESSION_COOKIE}=\"\"; expires={}; Max-Age=0; Path=/; SameSite=lax",
        http_date(now)
    )
}

fn with_cookie(mut response: Response, cookie: &str) -> Result<Response, ApiError> {
    let value = HeaderValue::from_str(cookie).map_err(|e| ApiError::Internal(e.to_string()))?;
    response.headers_mut().append(header::SET_COOKIE, value);
    Ok(response)
}

fn signed_in(ctx: &Ctx, token: &str) -> Result<Response, ApiError> {
    let secure = ctx.config().secure_cookies;
    with_cookie(ok(json!({"ok": true}))?, &session_cookie(token, secure))
}

fn as_400(e: Error) -> ApiError {
    ApiError::from(e)
}

pub fn state(ctx: &Ctx) -> Result<Response, ApiError> {
    let cfg = ctx.config();
    let auth = &ctx.app.auth;
    ok(json!({
        "passkeys": auth.has_credentials()?,
        "password": auth.has_password()?,
        "logged_in": ctx.logged_in()?,
        "site_name": cfg.site_name,
        "language": cfg.language,
    }))
}

pub fn register_options(ctx: &Ctx) -> Result<Response, ApiError> {
    let raw = ctx.body_json()?;
    ctx.throttle()?;
    let mut check = Check::new();
    let mut body = Body::open(&mut check, raw);
    let code = body.string("code", 4, Some(64), None);
    check.finish()?;
    let code = code.unwrap_or_default();
    let options = ctx.app.auth.registration_options(&code).map_err(as_400)?;
    let mut response = Response::new(axum::body::Body::from(pyjson::dumps(&options)));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    Ok(response)
}

pub fn register_verify(ctx: &Ctx) -> Result<Response, ApiError> {
    let raw = ctx.body_json()?;
    ctx.throttle()?;
    let mut check = Check::new();
    let mut body = Body::open(&mut check, raw);
    let code = body.string("code", 4, Some(64), None);
    let credential = body.object("credential");
    let name = body.string("name", 0, Some(40), Some(""));
    check.finish()?;
    let (Some(code), Some(credential), Some(name)) = (code, credential, name) else {
        return Err(ApiError::Internal("validated body without fields".into()));
    };
    let token = ctx
        .app
        .auth
        .register(&code, &credential, &name, &ctx.user_agent())
        .map_err(as_400)?;
    signed_in(ctx, &token)
}

pub fn login_options(ctx: &Ctx) -> Result<Response, ApiError> {
    ctx.throttle()?;
    let (cid, options) = ctx.app.auth.login_options().map_err(as_400)?;
    ok(json!({"cid": cid, "options": options}))
}

pub fn login_verify(ctx: &Ctx) -> Result<Response, ApiError> {
    let raw = ctx.body_json()?;
    ctx.throttle()?;
    let mut check = Check::new();
    let mut body = Body::open(&mut check, raw);
    let cid = body.string("cid", 0, None, None);
    let credential = body.object("credential");
    check.finish()?;
    let (Some(cid), Some(credential)) = (cid, credential) else {
        return Err(ApiError::Internal("validated body without fields".into()));
    };
    match ctx.app.auth.login(&cid, &credential, &ctx.user_agent()) {
        Ok(token) => signed_in(ctx, &token),
        Err(Error::Invalid(reason)) => {
            crate::warning!("camd.api.auth", "passkey login rejected: {}", reason);
            Err(ApiError::new(401, "login failed"))
        }
        Err(e) => Err(e.into()),
    }
}

pub fn login_password(ctx: &Ctx) -> Result<Response, ApiError> {
    let raw = ctx.body_json()?;
    ctx.throttle()?;
    let mut check = Check::new();
    let mut body = Body::open(&mut check, raw);
    let password = body.string("password", 1, Some(256), None);
    check.finish()?;
    let password = password.unwrap_or_default();
    let Some(_slot) = ctx.app.auth.password_slot() else {
        return Err(ApiError::new(429, "too many attempts"));
    };
    match ctx.app.auth.login_password(&password, &ctx.user_agent()) {
        Ok(token) => signed_in(ctx, &token),
        Err(Error::Invalid(_)) => Err(ApiError::new(401, "login failed")),
        Err(e) => Err(e.into()),
    }
}

pub fn logout(ctx: &Ctx) -> Result<Response, ApiError> {
    let sid = ctx.session()?;
    ctx.app.auth.revoke(sid)?;
    with_cookie(ok(json!({"ok": true}))?, &logout_cookie(clock::now()))
}

pub fn enroll_code(ctx: &Ctx) -> Result<Response, ApiError> {
    ctx.session()?;
    let code = ctx.app.auth.new_enroll_code()?;
    ok(json!({"code": code, "expires_in": ENROLL_EXPIRES_IN}))
}

pub fn devices(ctx: &Ctx) -> Result<Response, ApiError> {
    let sid = ctx.session()?;
    let auth = &ctx.app.auth;
    ok(json!({
        "credentials": auth.credentials()?,
        "sessions": auth.sessions()?,
        "current_session": sid,
        "password": auth.has_password()?,
    }))
}

pub fn delete_session(ctx: &Ctx) -> Result<Response, ApiError> {
    let mut check = Check::new();
    let id = check.path_int("session_id", ctx.param("session_id"));
    ctx.session()?;
    check.finish()?;
    ctx.app.auth.revoke(id.unwrap_or_default())?;
    ok(json!({"ok": true}))
}

pub fn delete_credential(ctx: &Ctx) -> Result<Response, ApiError> {
    ctx.session()?;
    let auth = &ctx.app.auth;
    if auth.credentials()?.len() <= 1 && !auth.has_password()? {
        return Err(ApiError::new(400, "cannot remove the last way to sign in"));
    }
    auth.delete_credential(ctx.param("credential_id"))?;
    ok(json!({"ok": true}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies_match_starlette() {
        assert_eq!(
            session_cookie("abc", false),
            "cam_session=abc; HttpOnly; Max-Age=2592000; Path=/; SameSite=strict"
        );
        assert!(session_cookie("abc", true).ends_with("; SameSite=strict; Secure"));
        assert_eq!(
            logout_cookie(1794619973.9),
            "cam_session=\"\"; expires=Sat, 14 Nov 2026 01:32:53 GMT; Max-Age=0; Path=/; SameSite=lax"
        );
        assert_eq!(http_date(0.0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(http_date(951782400.0), "Tue, 29 Feb 2000 00:00:00 GMT");
    }
}
