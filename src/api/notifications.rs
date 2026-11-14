use std::net::IpAddr;

use axum::response::Response;
use serde_json::{Value, json};

use super::validate::{Body, Check};
use super::{ApiError, Ctx, ok};
use crate::messages::translate;
use crate::notify::{Alerts, is_global_ip};

pub fn is_private_host(host: &str) -> bool {
    if host.is_empty() || host == "localhost" || host.ends_with(".local") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(ip) => !is_global_ip(ip),
        Err(_) => false,
    }
}

pub fn split_endpoint(endpoint: &str) -> (String, String) {
    let Some((scheme, rest)) = endpoint.split_once("://") else {
        return (String::new(), String::new());
    };
    let netloc = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = netloc.rsplit_once('@').map(|(_, h)| h).unwrap_or(netloc);
    let host = if let Some(inner) = host_port.strip_prefix('[') {
        inner.split(']').next().unwrap_or("")
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    (scheme.to_ascii_lowercase(), host.to_ascii_lowercase())
}

pub fn valid_endpoint(endpoint: &str) -> bool {
    let (scheme, host) = split_endpoint(endpoint);
    scheme == "https" && !is_private_host(&host)
}

pub fn key(ctx: &Ctx) -> Result<Response, ApiError> {
    ctx.session()?;
    ok(json!({"key": ctx.app.notifier.web.public_key()}))
}

pub fn subscribe(ctx: &Ctx) -> Result<Response, ApiError> {
    let raw = ctx.body_json()?;
    ctx.session()?;
    let mut check = Check::new();
    let mut body = Body::open(&mut check, raw);
    let subscription = body.object("subscription");
    check.finish()?;
    let subscription = subscription.unwrap_or(Value::Null);
    let endpoint = subscription
        .get("endpoint")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !valid_endpoint(endpoint) {
        return Err(ApiError::new(400, "invalid subscription"));
    }
    ctx.app.notifier.web.subscribe(&subscription)?;
    ok(json!({"ok": true}))
}

pub fn unsubscribe(ctx: &Ctx) -> Result<Response, ApiError> {
    let raw = ctx.body_json()?;
    ctx.session()?;
    let mut check = Check::new();
    let mut body = Body::open(&mut check, raw);
    let endpoint = body.string("endpoint", 0, None, None);
    check.finish()?;
    ctx.app
        .notifier
        .web
        .unsubscribe(&endpoint.unwrap_or_default())?;
    ok(json!({"ok": true}))
}

pub fn test(ctx: &Ctx) -> Result<Response, ApiError> {
    ctx.session()?;
    let cfg = ctx.config();
    let title = translate(&cfg.language, "test_title", &[]);
    let body = translate(&cfg.language, "test_body", &[("site", &cfg.site_name)]);
    let sent = ctx.app.notifier.send(&title, &body, "/", "test");
    ok(json!({"sent": sent}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_endpoints_must_be_public_https() {
        for bad in [
            "http://push.example.org/x",
            "https://127.0.0.1/x",
            "https://192.168.1.5/x",
            "https://localhost/x",
            "ftp://a/b",
            "https://[::1]/x",
            "https://[fe80::1]:443/x",
            "https://printer.local/x",
            "https://10.1.2.3:8443/x",
            "https://100.64.0.1/x",
            "https://[::ffff:192.168.0.1]/x",
            "https:///x",
            "",
            "garbage",
        ] {
            assert!(!valid_endpoint(bad), "{bad}");
        }
        for good in [
            "https://fcm.googleapis.com/send/abc",
            "HTTPS://updates.push.services.mozilla.com/wpush/v2/x",
            "https://8.8.8.8/x",
            "https://[2a00:1450::1]/x",
            "https://user@web.push.apple.com:443/x",
        ] {
            assert!(valid_endpoint(good), "{good}");
        }
    }
}
