use std::path::PathBuf;

use axum::response::{IntoResponse, Response};
use serde_json::json;

use super::files::{self, FileReply};
use super::validate::{Body, Check};
use super::{ApiError, Ctx, ok};
use crate::db::{EventFilter, EventRow};
use crate::photos::inside;
use crate::storage::remove_event;

pub const DEFAULT_LIMIT: i64 = 50;

pub fn list(ctx: &Ctx) -> Result<Response, ApiError> {
    ctx.session()?;
    let mut check = Check::new();
    let limit = check.query_int(ctx, "limit", DEFAULT_LIMIT);
    let before = check.query_float(ctx, "before");
    let after = check.query_float(ctx, "after");
    let label = ctx.query_value("label").map(str::to_string);
    let hide_unconfirmed = check.query_bool(ctx, "hide_unconfirmed");
    check.finish()?;
    let filter = EventFilter {
        limit,
        before,
        after,
        label: label.filter(|l| !l.is_empty()),
        hide_unconfirmed,
    };
    let rows = ctx.camera().runtime.db().list_events(&filter)?;
    let events: Vec<_> = rows.iter().map(EventRow::to_json).collect();
    ok(json!({"events": events}))
}

pub fn count(ctx: &Ctx) -> Result<Response, ApiError> {
    ctx.session()?;
    let mut check = Check::new();
    let since = check.query_float(ctx, "since").unwrap_or(0.0);
    check.finish()?;
    let n = ctx
        .camera()
        .runtime
        .db()
        .count_visible_events_since(since)?;
    ok(json!({"count": n}))
}

fn finished(ctx: &Ctx, id: i64) -> Result<Option<EventRow>, ApiError> {
    Ok(ctx.camera().runtime.db().finished_event(id)?)
}

pub fn delete_many(ctx: &Ctx) -> Result<Response, ApiError> {
    let raw = ctx.body_json()?;
    ctx.session()?;
    let mut check = Check::new();
    let mut body = Body::open(&mut check, raw);
    let ids = body.int_list("ids", 1, 200);
    check.finish()?;
    let db = ctx.camera().runtime.db();
    let mut removed = 0;
    for id in ids.unwrap_or_default() {
        if let Some(row) = finished(ctx, id)? {
            remove_event(db, &row)?;
            removed += 1;
        }
    }
    ok(json!({"removed": removed}))
}

pub fn delete_one(ctx: &Ctx) -> Result<Response, ApiError> {
    let mut check = Check::new();
    let id = check.path_int("event_id", ctx.param("event_id"));
    ctx.session()?;
    check.finish()?;
    let Some(row) = finished(ctx, id.unwrap_or_default())? else {
        return Err(ApiError::new(404, "not found"));
    };
    remove_event(ctx.camera().runtime.db(), &row)?;
    ok(json!({"ok": true}))
}

pub fn keep(ctx: &Ctx) -> Result<Response, ApiError> {
    let raw = ctx.body_json()?;
    let mut check = Check::new();
    let id = check.path_int("event_id", ctx.param("event_id"));
    ctx.session()?;
    let mut body = Body::open(&mut check, raw);
    let keep = body.boolean("keep");
    check.finish()?;
    let (id, keep) = (id.unwrap_or_default(), keep.unwrap_or_default());
    if finished(ctx, id)?.is_none() {
        return Err(ApiError::new(404, "not found"));
    }
    ctx.camera().runtime.db().set_event_keep(id, keep)?;
    ok(json!({"ok": true, "keep": keep}))
}

fn event_file(ctx: &Ctx, thumb: bool) -> Result<FileReply, ApiError> {
    let mut check = Check::new();
    let id = check.path_int("event_id", ctx.param("event_id"));
    ctx.session()?;
    let download = if thumb {
        false
    } else {
        check.query_bool(ctx, "download")
    };
    check.finish()?;
    let id = id.unwrap_or_default();
    let row = ctx.camera().runtime.db().event(id)?;
    let column = row.and_then(|r| if thumb { r.thumb_path } else { r.clip_path });
    let root = ctx.config().clips_dir();
    let path = column
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .filter(|p| inside(p, &root))
        .ok_or_else(|| ApiError::new(404, "not found"))?;
    Ok(FileReply {
        path,
        content_type: if thumb { "image/jpeg" } else { "video/mp4" },
        download: download.then(|| format!("event-{id}.mp4")),
    })
}

pub async fn serve_file(ctx: Ctx, resolve: fn(&Ctx) -> Result<FileReply, ApiError>) -> Response {
    let headers = ctx.parts.headers.clone();
    match tokio::task::spawn_blocking(move || resolve(&ctx)).await {
        Ok(Ok(reply)) => files::send(reply, &headers).await,
        Ok(Err(e)) => e.into_response(),
        Err(_) => ApiError::Internal("handler panicked".into()).into_response(),
    }
}

pub async fn clip(ctx: Ctx) -> Response {
    serve_file(ctx, |ctx| event_file(ctx, false)).await
}

pub async fn thumb(ctx: Ctx) -> Response {
    serve_file(ctx, |ctx| event_file(ctx, true)).await
}
