use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use futures_util::stream::{self, StreamExt};
use serde_json::json;
use tokio::sync::watch;

use super::events::serve_file;
use super::files::FileReply;
use super::validate::{Body as JsonBody, Check};
use super::{ApiError, Ctx, ok};
use crate::runtime::{ClipError, Frame, LIVE_FIRST_WAIT_S, LIVE_NEXT_WAIT_S};
use crate::viewers::Viewer;

pub const BOUNDARY: &str = "frame";
pub const DEFAULT_PHOTOS: i64 = 60;

fn fresh_frame(ctx: &Ctx) -> Result<Frame, ApiError> {
    ctx.camera()
        .runtime
        .fresh_frame()
        .ok_or_else(|| ApiError::new(503, "no frame"))
}

pub fn snapshot(ctx: &Ctx) -> Result<Response, ApiError> {
    ctx.session()?;
    let mut check = Check::new();
    let download = check.query_bool(ctx, "download");
    check.finish()?;
    let frame = fresh_frame(ctx)?;
    let mut response = Response::new(Body::from(frame.jpeg));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if download {
        let value = format!("attachment; filename=\"snapshot-{}.jpg\"", frame.ts as i64);
        headers.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&value).map_err(|e| ApiError::Internal(e.to_string()))?,
        );
    }
    Ok(response)
}

struct LiveState {
    frames: watch::Receiver<Option<Frame>>,
    viewer: Viewer,
    shutdown: watch::Receiver<bool>,
    wait: Duration,
}

fn part_header(length: usize) -> Bytes {
    Bytes::from(format!(
        "--{BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {length}\r\n\r\n"
    ))
}

async fn next_frame(state: &mut LiveState) -> Option<Bytes> {
    loop {
        if state.viewer.is_cancelled() || *state.shutdown.borrow() {
            return None;
        }
        let changed = tokio::select! {
            changed = tokio::time::timeout(state.wait, state.frames.changed()) => changed,
            _ = state.viewer.cancelled() => return None,
            _ = state.shutdown.changed() => return None,
        };
        match changed {
            Ok(Ok(())) => {}
            _ => return None,
        }
        state.wait = Duration::from_secs_f64(LIVE_NEXT_WAIT_S);
        let frame = state.frames.borrow_and_update().clone();
        if let Some(frame) = frame {
            return Some(frame.jpeg);
        }
    }
}

pub fn live_response(
    mut frames: watch::Receiver<Option<Frame>>,
    viewer: Viewer,
    shutdown: watch::Receiver<bool>,
) -> Response {
    frames.mark_unchanged();
    let state = LiveState {
        frames,
        viewer,
        shutdown,
        wait: Duration::from_secs_f64(LIVE_FIRST_WAIT_S),
    };
    let parts = stream::unfold(state, |mut state| async move {
        let jpeg = next_frame(&mut state).await?;
        let chunk = stream::iter([
            Ok::<Bytes, std::io::Error>(part_header(jpeg.len())),
            Ok(jpeg),
            Ok(Bytes::from_static(b"\r\n")),
        ]);
        Some((chunk, state))
    })
    .flatten();
    let mut response = Response::new(Body::from_stream(parts));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("multipart/x-mixed-replace; boundary=frame"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn live_check(ctx: &Ctx) -> Result<(), ApiError> {
    ctx.session()?;
    if !ctx.camera().runtime.live_available() {
        return Err(ApiError::new(503, "camera in standby"));
    }
    Ok(())
}

pub async fn live(ctx: Ctx) -> Response {
    let (ctx, checked) = match tokio::task::spawn_blocking(move || {
        let result = live_check(&ctx);
        (ctx, result)
    })
    .await
    {
        Ok(pair) => pair,
        Err(_) => return ApiError::Internal("handler panicked".into()).into_response(),
    };
    if let Err(e) = checked {
        return e.into_response();
    }
    let camera = ctx.camera();
    let frames = camera.runtime.frames();
    let viewer = camera.viewers.join();
    live_response(frames, viewer, ctx.app.shutdown.subscribe())
}

pub fn clip_start(ctx: &Ctx) -> Result<Response, ApiError> {
    let raw = ctx.body_json()?;
    ctx.session()?;
    let mut check = Check::new();
    let mut body = JsonBody::open(&mut check, raw);
    let seconds = body.int("seconds", Some(5), Some(300), Some(30));
    check.finish()?;
    let seconds = u32::try_from(seconds.unwrap_or(30)).unwrap_or(30);
    match ctx.camera().runtime.clip_start(seconds) {
        Ok(state) => ok(state.to_json()),
        Err(ClipError::Unavailable) => Err(ApiError::new(409, "camera not available")),
        Err(ClipError::Failed) => Err(ApiError::Internal("cannot start the clip".into())),
    }
}

pub fn clip_stop(ctx: &Ctx) -> Result<Response, ApiError> {
    ctx.session()?;
    match ctx.camera().runtime.clip_stop() {
        Some(_) => ok(json!({"ok": true})),
        None => Err(ApiError::new(404, "no clip in progress")),
    }
}

pub fn take_photo(ctx: &Ctx) -> Result<Response, ApiError> {
    ctx.session()?;
    match ctx.camera().runtime.take_photo() {
        Ok(value) => ok(value),
        Err(e) if e.status() == 500 => Err(ApiError::Internal(e.detail().to_string())),
        Err(e) => Err(ApiError::new(e.status(), e.detail())),
    }
}

pub fn list_photos(ctx: &Ctx) -> Result<Response, ApiError> {
    ctx.session()?;
    let mut check = Check::new();
    let limit = check.query_int(ctx, "limit", DEFAULT_PHOTOS);
    let before = check.query_float(ctx, "before");
    check.finish()?;
    let photos = ctx
        .camera()
        .runtime
        .photos()
        .list(limit.clamp(1, 200), before)?;
    ok(json!({"photos": photos}))
}

pub fn delete_photo(ctx: &Ctx) -> Result<Response, ApiError> {
    let mut check = Check::new();
    let id = check.path_int("photo_id", ctx.param("photo_id"));
    ctx.session()?;
    check.finish()?;
    if !ctx.camera().runtime.photos().remove(id.unwrap_or_default()) {
        return Err(ApiError::new(404, "not found"));
    }
    ok(json!({"ok": true}))
}

pub fn delete_photos(ctx: &Ctx) -> Result<Response, ApiError> {
    let raw = ctx.body_json()?;
    ctx.session()?;
    let mut check = Check::new();
    let mut body = JsonBody::open(&mut check, raw);
    let ids = body.int_list("ids", 1, 200);
    check.finish()?;
    let photos = ctx.camera().runtime.photos();
    let removed = ids
        .unwrap_or_default()
        .into_iter()
        .filter(|id| photos.remove(*id))
        .count();
    ok(json!({"removed": removed}))
}

fn photo_file(ctx: &Ctx, thumb: bool) -> Result<FileReply, ApiError> {
    let mut check = Check::new();
    let id = check.path_int("photo_id", ctx.param("photo_id"));
    ctx.session()?;
    let download = !thumb && check.query_bool(ctx, "download");
    check.finish()?;
    let id = id.unwrap_or_default();
    let photos = ctx.camera().runtime.photos();
    let path = photos
        .file(id, thumb)
        .ok_or_else(|| ApiError::new(404, "not found"))?;
    let download = if download {
        photos
            .get(id)
            .map(|row| format!("photo-{}.jpg", row.ts as i64))
    } else {
        None
    };
    Ok(FileReply {
        path,
        content_type: "image/jpeg",
        download,
    })
}

pub async fn photo_image(ctx: Ctx) -> Response {
    serve_file(ctx, |ctx| photo_file(ctx, false)).await
}

pub async fn photo_thumb(ctx: Ctx) -> Response {
    serve_file(ctx, |ctx| photo_file(ctx, true)).await
}
