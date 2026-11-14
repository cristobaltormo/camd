use axum::response::Response;
use serde_json::json;

use super::validate::{Body, Check};
use super::{ApiError, Ctx, ok};
use crate::arming::MODES;
use crate::prefs;

pub fn cameras(ctx: &Ctx) -> Result<Response, ApiError> {
    ctx.session()?;
    let list: Vec<_> = ctx
        .app
        .cameras
        .iter()
        .enumerate()
        .map(|(index, camera)| {
            let cfg = camera.config();
            let cfg = cfg.read().unwrap_or_else(|e| e.into_inner());
            let name = if cfg.camera_name.is_empty() {
                cfg.site_name.clone()
            } else {
                cfg.camera_name.clone()
            };
            json!({"id": camera.id, "name": name, "current": index == ctx.camera_index})
        })
        .collect();
    ok(json!({"cameras": list}))
}

pub fn status(ctx: &Ctx) -> Result<Response, ApiError> {
    ctx.session()?;
    ok(ctx.camera().runtime.status(crate::VERSION, ctx.app.build))
}

pub fn mode(ctx: &Ctx) -> Result<Response, ApiError> {
    let raw = ctx.body_json()?;
    ctx.session()?;
    let mut check = Check::new();
    let mut body = Body::open(&mut check, raw);
    let mode = body.string("mode", 0, None, None);
    check.finish()?;
    let mode = mode.unwrap_or_default();
    if !MODES.contains(&mode.as_str()) {
        return Err(ApiError::new(400, "invalid mode"));
    }
    ctx.camera().runtime.set_mode(&mode)?;
    ok(json!({"ok": true}))
}

pub fn snooze(ctx: &Ctx) -> Result<Response, ApiError> {
    let raw = ctx.body_json()?;
    ctx.session()?;
    let mut check = Check::new();
    let mut body = Body::open(&mut check, raw);
    let minutes = body.float("minutes", Some(0.0), Some(1440.0));
    check.finish()?;
    ctx.camera().runtime.snooze(minutes.unwrap_or_default())?;
    ok(json!({"ok": true}))
}

pub fn privacy(ctx: &Ctx) -> Result<Response, ApiError> {
    let raw = ctx.body_json()?;
    ctx.session()?;
    let mut check = Check::new();
    let mut body = Body::open(&mut check, raw);
    let on = body.boolean("on");
    check.finish()?;
    if !ctx.config().camera_enabled {
        return Err(ApiError::new(409, "camera not available"));
    }
    if !ctx.camera().runtime.set_privacy(on.unwrap_or_default()) {
        return Err(ApiError::new(502, "camera did not answer"));
    }
    ok(json!({"ok": true}))
}

pub const SETTINGS_FIELDS: [&str; 27] = [
    "sensitivity",
    "ignore_rects",
    "alert_rects",
    "retention_days",
    "pre_roll_s",
    "post_roll_s",
    "max_event_s",
    "max_disk_gb",
    "presence_grace_s",
    "push_cooldown_s",
    "unconfirmed_keep_h",
    "detect_confidence",
    "detection",
    "detect_labels",
    "language",
    "notify_motion",
    "notify_offline",
    "quiet_enabled",
    "quiet_start",
    "quiet_end",
    "camera_enabled",
    "camera_vflip",
    "camera_hmirror",
    "camera_brightness",
    "camera_contrast",
    "camera_quality",
    "camera_framesize",
];

pub fn pydantic_order(mut errors: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    errors.sort_by_key(|e| {
        let field = e["loc"][1].as_str().unwrap_or("");
        let extra = e["type"] == "extra_forbidden";
        let index = SETTINGS_FIELDS
            .iter()
            .position(|f| *f == field)
            .unwrap_or(usize::MAX);
        (extra, if extra { 0 } else { index })
    });
    errors
}

pub fn get_settings(ctx: &Ctx) -> Result<Response, ApiError> {
    ctx.session()?;
    let runtime = &ctx.camera().runtime;
    let motion = runtime.motion_settings();
    ok(prefs::settings_json(&ctx.config(), &motion))
}

pub fn post_settings(ctx: &Ctx) -> Result<Response, ApiError> {
    let raw = ctx.body_json()?;
    ctx.session()?;
    let value = match raw {
        super::BodyJson::Json(value) => value,
        other => {
            let mut check = Check::new();
            Body::open(&mut check, other);
            check.finish()?;
            serde_json::Value::Null
        }
    };
    if !value.is_object() {
        let mut check = Check::new();
        Body::open(&mut check, super::BodyJson::Json(value));
        check.finish()?;
        return Err(ApiError::Internal("non-object settings body".into()));
    }
    let body = prefs::parse_settings_body(&value)
        .map_err(|errors| ApiError::Validation(pydantic_order(errors)))?;
    let camera = ctx.camera();
    let runtime = &camera.runtime;
    let shared = camera.config();
    let applied = {
        let mut cfg = shared.write().unwrap_or_else(|e| e.into_inner());
        prefs::apply_settings(&mut cfg, runtime.db(), &body)?
    };
    runtime.motion_settings_changed(applied.motion);
    runtime.apply_prefs(&applied.changed);
    ok(json!({"ok": true}))
}
