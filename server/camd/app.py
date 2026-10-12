import json
import logging
from contextlib import asynccontextmanager
from pathlib import Path

from fastapi import Depends, FastAPI, HTTPException, Request, Response
from fastapi.responses import FileResponse, JSONResponse, StreamingResponse
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel, Field, StrictBool

from . import prefs
from .arming import MODES
from .auth import SESSION_TTL, Auth, AuthError
from .config import Config
from .db import Database
from .events import remove_event
from .photos import PhotoLimit
from .runtime import Runtime
from .viewers import ViewerPool

log = logging.getLogger("camd.app")
WEB_DIR = Path(__file__).parent / "web"
VERSION = "1.2.1"
BUILD_FILE = Path(__file__).parent / "build.txt"
BUILD = BUILD_FILE.read_text().strip() if BUILD_FILE.exists() else "dev"
COOKIE = "cam_session"
MAX_VIEWERS = 4
MAX_CLIP_S = 300

CSP = (
    "default-src 'self'; img-src 'self' data: blob:; media-src 'self'; style-src 'self'; "
    "script-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"
)


class CodeBody(BaseModel):
    code: str = Field(min_length=4, max_length=64)


class RegisterBody(CodeBody):
    credential: dict
    name: str = Field(default="", max_length=40)


class LoginBody(BaseModel):
    cid: str
    credential: dict


class ModeBody(BaseModel):
    mode: str


class SnoozeBody(BaseModel):
    minutes: float = Field(ge=0, le=1440)


class PrivacyBody(BaseModel):
    on: bool


class SettingsBody(BaseModel):
    model_config = {"extra": "forbid"}

    sensitivity: int | None = Field(default=None, ge=1, le=10)
    ignore_rects: list[list[float]] | None = Field(default=None, max_length=10)
    retention_days: int | None = Field(default=None, ge=1, le=90)
    pre_roll_s: float | None = None
    post_roll_s: float | None = None
    max_event_s: float | None = None
    max_disk_gb: float | None = None
    presence_grace_s: float | None = None
    push_cooldown_s: float | None = None
    notify_motion: StrictBool | None = None
    notify_offline: StrictBool | None = None
    quiet_enabled: StrictBool | None = None
    quiet_start: str | None = None
    quiet_end: str | None = None
    camera_vflip: StrictBool | None = None
    camera_hmirror: StrictBool | None = None
    camera_brightness: int | None = None
    camera_contrast: int | None = None
    camera_quality: int | None = None


class IdsBody(BaseModel):
    ids: list[int] = Field(min_length=1, max_length=200)


class ClipBody(BaseModel):
    seconds: int = Field(default=30, ge=5, le=MAX_CLIP_S)


class KeepBody(BaseModel):
    keep: bool


class PushBody(BaseModel):
    subscription: dict


class EndpointBody(BaseModel):
    endpoint: str


def create_app(cfg: Config, db: Database | None = None, runtime: Runtime | None = None, start: bool = True) -> FastAPI:
    db = db or Database(cfg.db_path)
    rt = runtime or Runtime(cfg, db)
    auth = Auth(cfg, db)

    @asynccontextmanager
    async def lifespan(_app):
        if start:
            rt.start()
        yield
        if start:
            rt.stop()

    app = FastAPI(lifespan=lifespan, docs_url=None, redoc_url=None, openapi_url=None)
    app.state.runtime = rt
    app.state.auth = auth

    def client_ip(request: Request) -> str:
        if cfg.trust_proxy:
            return request.headers.get("cf-connecting-ip") or request.client.host
        return request.client.host if request.client else "unknown"

    def throttle(request: Request) -> None:
        if not auth.limiter.allow(client_ip(request)):
            raise HTTPException(429, "too many attempts")

    def session(request: Request) -> int:
        sid = auth.check_session(request.cookies.get(COOKIE))
        if sid is None:
            raise HTTPException(401, "login required")
        return sid

    @app.middleware("http")
    async def guard(request: Request, call_next):
        if request.method not in ("GET", "HEAD", "OPTIONS"):
            if request.headers.get("origin") != cfg.origin:
                return JSONResponse({"detail": "bad origin"}, status_code=403)
        response = await call_next(request)
        response.headers["Content-Security-Policy"] = CSP
        response.headers["X-Content-Type-Options"] = "nosniff"
        response.headers["Referrer-Policy"] = "no-referrer"
        response.headers["X-Frame-Options"] = "DENY"
        if "cache-control" not in response.headers:
            response.headers["Cache-Control"] = "no-store" if request.url.path.startswith("/api/") else "private, no-cache"
        return response

    def set_cookie(response: Response, token: str) -> None:
        response.set_cookie(
            COOKIE, token, max_age=SESSION_TTL, httponly=True, samesite="strict", secure=cfg.secure_cookies, path="/"
        )

    @app.get("/healthz")
    def healthz():
        return {"ok": True}

    # ---- auth ----

    @app.get("/api/auth/state")
    def auth_state(request: Request):
        return {
            "enrolled": auth.has_credentials(),
            "logged_in": auth.check_session(request.cookies.get(COOKIE)) is not None,
        }

    @app.post("/api/auth/register/options")
    def register_options(body: CodeBody, request: Request):
        throttle(request)
        try:
            return Response(auth.registration_options(body.code), media_type="application/json")
        except AuthError as exc:
            raise HTTPException(400, str(exc))

    @app.post("/api/auth/register/verify")
    def register_verify(body: RegisterBody, request: Request, response: Response):
        throttle(request)
        try:
            token = auth.register(body.code, body.credential, body.name, request.headers.get("user-agent", ""))
        except AuthError as exc:
            raise HTTPException(400, str(exc))
        set_cookie(response, token)
        return {"ok": True}

    @app.post("/api/auth/login/options")
    def login_options(request: Request):
        throttle(request)
        try:
            cid, options = auth.login_options()
        except AuthError as exc:
            raise HTTPException(400, str(exc))
        return {"cid": cid, "options": json.loads(options)}

    @app.post("/api/auth/login/verify")
    def login_verify(body: LoginBody, request: Request, response: Response):
        throttle(request)
        try:
            token = auth.login(body.cid, body.credential, request.headers.get("user-agent", ""))
        except AuthError as exc:
            log.warning("login rejected from %s: %s", client_ip(request), exc)
            raise HTTPException(401, "login failed")
        set_cookie(response, token)
        return {"ok": True}

    @app.post("/api/auth/logout")
    def logout(request: Request, response: Response, sid: int = Depends(session)):
        auth.revoke(sid)
        response.delete_cookie(COOKIE, path="/")
        return {"ok": True}

    @app.post("/api/auth/enroll-code")
    def enroll_code(_: int = Depends(session)):
        return {"code": auth.new_enroll_code(), "expires_in": 600}

    @app.get("/api/auth/devices")
    def devices(request: Request, sid: int = Depends(session)):
        return {"credentials": auth.credentials(), "sessions": auth.sessions(), "current_session": sid}

    @app.delete("/api/auth/sessions/{session_id}")
    def delete_session(session_id: int, _: int = Depends(session)):
        auth.revoke(session_id)
        return {"ok": True}

    @app.delete("/api/auth/credentials/{cred_id}")
    def delete_credential(cred_id: str, _: int = Depends(session)):
        if len(auth.credentials()) <= 1:
            raise HTTPException(400, "cannot remove the last passkey")
        auth.delete_credential(cred_id)
        return {"ok": True}

    # ---- state and control ----

    @app.get("/api/status")
    def status(_: int = Depends(session)):
        armed, reason = rt.arming.decision()
        last = db.one("SELECT id, start_ts, status FROM events ORDER BY start_ts DESC LIMIT 1")
        info = rt.camera_info or {}
        usage = db.one("SELECT COUNT(*) AS n, COALESCE(SUM(size_bytes), 0) AS bytes FROM events")
        photo_count, photo_bytes = rt.photos.usage()
        return {
            "armed": armed,
            "reason": reason,
            "mode": rt.arming.mode,
            "snooze_until": rt.arming.snooze_until,
            "privacy": rt.arming.privacy,
            "presence": {"configured": rt.presence.configured, "home": rt.presence.is_home},
            "camera": {
                "online": rt.grabber.online,
                "standby": rt.grabber.standby,
                "rssi": info.get("rssi"),
                "fw": info.get("fw"),
                "uptime_s": info.get("uptime_s"),
            },
            "motion": rt.motion_now,
            "recording": rt.recorder.recording,
            "last_event": dict(last) if last else None,
            "push_subscriptions": rt.push.count(),
            "notifich": rt.push.notifich_enabled,
            "version": VERSION,
            "build": BUILD,
            "storage": {
                "events": usage["n"],
                "bytes": usage["bytes"],
                "cap_bytes": int(cfg.max_disk_gb * 1024**3),
                "photos": photo_count,
                "photo_bytes": photo_bytes,
            },
            "clip": rt.recorder.manual_state(),
        }

    @app.post("/api/mode")
    def set_mode(body: ModeBody, _: int = Depends(session)):
        if body.mode not in MODES:
            raise HTTPException(400, "invalid mode")
        rt.arming.set_mode(body.mode)
        rt.arming.snooze(0)
        return {"ok": True}

    @app.post("/api/snooze")
    def snooze(body: SnoozeBody, _: int = Depends(session)):
        rt.arming.snooze(body.minutes)
        return {"ok": True}

    @app.post("/api/privacy")
    def privacy(body: PrivacyBody, _: int = Depends(session)):
        if not rt.set_privacy(body.on):
            raise HTTPException(502, "camera did not answer")
        return {"ok": True}

    @app.get("/api/settings")
    def get_settings(_: int = Depends(session)):
        s = rt.load_motion_settings()
        return {
            "sensitivity": s.sensitivity,
            "ignore_rects": s.ignore_rects,
            "retention_days": cfg.retention_days,
            **prefs.snapshot(cfg),
        }

    @app.post("/api/settings")
    def post_settings(body: SettingsBody, _: int = Depends(session)):
        data = body.model_dump(exclude_none=True)
        motion = rt.load_motion_settings()
        sensitivity = data.pop("sensitivity", motion.sensitivity)
        rects = data.pop("ignore_rects", motion.ignore_rects)
        retention = data.pop("retention_days", cfg.retention_days)
        for rect in rects:
            if len(rect) != 4 or any(not 0 <= v <= 1 for v in rect):
                raise HTTPException(400, "invalid rect")
        try:
            changed = prefs.update(cfg, db, data)
        except ValueError as exc:
            raise HTTPException(400, str(exc))
        rt.save_motion_settings(sensitivity, rects)
        cfg.retention_days = retention
        db.set_setting("retention_days", retention)
        rt.apply_prefs(changed)
        return {"ok": True}

    # ---- events ----

    def event_json(row) -> dict:
        end = row["end_ts"]
        return {
            "id": row["id"],
            "start": row["start_ts"],
            "end": end,
            "duration": (end - row["start_ts"]) if end else None,
            "status": row["status"],
            "score": row["score"],
            "size": row["size_bytes"],
            "kind": row["kind"],
            "keep": bool(row["keep"]),
        }

    @app.get("/api/events")
    def list_events(limit: int = 50, before: float | None = None, _: int = Depends(session)):
        limit = max(1, min(limit, 200))
        if before is None:
            rows = db.query("SELECT * FROM events ORDER BY start_ts DESC LIMIT ?", (limit,))
        else:
            rows = db.query("SELECT * FROM events WHERE start_ts < ? ORDER BY start_ts DESC LIMIT ?", (before, limit))
        return {"events": [event_json(r) for r in rows]}

    def event_file(event_id: int, column: str, media_type: str, filename: str | None = None) -> FileResponse:
        row = db.one("SELECT * FROM events WHERE id=?", (event_id,))
        path = Path(row[column]) if row and row[column] else None
        if path is None or not path.is_file() or cfg.clips_dir.resolve() not in path.resolve().parents:
            raise HTTPException(404, "not found")
        return FileResponse(path, media_type=media_type, filename=filename, headers={"Cache-Control": "private, max-age=3600"})

    @app.get("/api/events/{event_id}/clip")
    def event_clip(event_id: int, download: bool = False, _: int = Depends(session)):
        return event_file(event_id, "clip_path", "video/mp4", f"habitacion-{event_id}.mp4" if download else None)

    @app.get("/api/events/{event_id}/thumb")
    def event_thumb(event_id: int, _: int = Depends(session)):
        return event_file(event_id, "thumb_path", "image/jpeg")

    @app.get("/api/events/count")
    def count_events(since: float = 0, _: int = Depends(session)):
        return {"count": db.one("SELECT COUNT(*) AS n FROM events WHERE start_ts > ?", (since,))["n"]}

    @app.post("/api/events/delete")
    def delete_events(body: IdsBody, _: int = Depends(session)):
        removed = 0
        for event_id in body.ids:
            row = db.one("SELECT id, clip_path, thumb_path FROM events WHERE id=? AND status != 'recording'", (event_id,))
            if row is not None:
                remove_event(db, row)
                removed += 1
        return {"removed": removed}

    @app.delete("/api/events/{event_id}")
    def delete_event(event_id: int, _: int = Depends(session)):
        row = db.one("SELECT id, clip_path, thumb_path FROM events WHERE id=? AND status != 'recording'", (event_id,))
        if row is None:
            raise HTTPException(404, "not found")
        remove_event(db, row)
        return {"ok": True}

    # ---- live ----

    def fresh_frame() -> tuple[float, bytes]:
        latest = rt.grabber.latest
        if latest is None or not rt.grabber.online:
            raise HTTPException(503, "no frame")
        return latest

    @app.get("/api/snapshot")
    def snapshot(download: bool = False, _: int = Depends(session)):
        ts, jpeg = fresh_frame()
        headers = {"Cache-Control": "no-store"}
        if download:
            headers["Content-Disposition"] = f'attachment; filename="habitacion-{int(ts)}.jpg"'
        return Response(jpeg, media_type="image/jpeg", headers=headers)

    pool = ViewerPool(MAX_VIEWERS)

    @app.get("/api/live")
    def live(_: int = Depends(session)):
        if rt.grabber.standby:
            raise HTTPException(503, "camera in standby")
        cancel = pool.join()

        def generate():
            rt.grabber.viewers += 1
            try:
                for jpeg in rt.grabber.frames():
                    if cancel.is_set():
                        return
                    yield b"--frame\r\nContent-Type: image/jpeg\r\nContent-Length: %d\r\n\r\n" % len(jpeg)
                    yield jpeg
                    yield b"\r\n"
            finally:
                rt.grabber.viewers -= 1
                pool.leave(cancel)

        return StreamingResponse(
            generate(), media_type="multipart/x-mixed-replace; boundary=frame", headers={"Cache-Control": "no-store"}
        )

    # ---- manual clips and kept events ----

    @app.post("/api/clips/start")
    def clip_start(body: ClipBody, _: int = Depends(session)):
        if rt.grabber.standby or not rt.grabber.online:
            raise HTTPException(409, "camera not available")
        rt.recorder.start_manual(body.seconds)
        return rt.recorder.manual_state()

    @app.post("/api/clips/stop")
    def clip_stop(_: int = Depends(session)):
        if rt.recorder.stop_manual() is None:
            raise HTTPException(404, "no clip in progress")
        return {"ok": True}

    @app.post("/api/events/{event_id}/keep")
    def keep_event(event_id: int, body: KeepBody, _: int = Depends(session)):
        if db.one("SELECT id FROM events WHERE id=? AND status != 'recording'", (event_id,)) is None:
            raise HTTPException(404, "not found")
        db.execute("UPDATE events SET keep=? WHERE id=?", (int(body.keep), event_id))
        return {"ok": True, "keep": body.keep}

    # ---- photos ----

    @app.post("/api/photos")
    def photo_take(_: int = Depends(session)):
        ts, jpeg = fresh_frame()
        try:
            return rt.photos.save(jpeg, ts)
        except PhotoLimit as exc:
            raise HTTPException(409, str(exc))
        except ValueError as exc:
            raise HTTPException(422, str(exc))

    @app.get("/api/photos")
    def photo_list(limit: int = 60, before: float | None = None, _: int = Depends(session)):
        return {"photos": rt.photos.list(max(1, min(limit, 200)), before)}

    def photo_file(photo_id: int, thumb: bool, download: bool = False) -> FileResponse:
        path = rt.photos.file(photo_id, thumb)
        if path is None:
            raise HTTPException(404, "not found")
        row = rt.photos.get(photo_id)
        return FileResponse(
            path,
            media_type="image/jpeg",
            filename=f"habitacion-{int(row['ts'])}.jpg" if download else None,
            headers={"Cache-Control": "private, max-age=3600"},
        )

    @app.get("/api/photos/{photo_id}/img")
    def photo_img(photo_id: int, download: bool = False, _: int = Depends(session)):
        return photo_file(photo_id, False, download)

    @app.get("/api/photos/{photo_id}/thumb")
    def photo_thumb(photo_id: int, _: int = Depends(session)):
        return photo_file(photo_id, True)

    @app.delete("/api/photos/{photo_id}")
    def photo_delete(photo_id: int, _: int = Depends(session)):
        if not rt.photos.remove(photo_id):
            raise HTTPException(404, "not found")
        return {"ok": True}

    @app.post("/api/photos/delete")
    def photo_delete_many(body: IdsBody, _: int = Depends(session)):
        return {"removed": sum(rt.photos.remove(i) for i in body.ids)}

    # ---- push ----

    @app.get("/api/push/key")
    def push_key(_: int = Depends(session)):
        return {"key": rt.push.public_key}

    @app.post("/api/push/subscribe")
    def push_subscribe(body: PushBody, _: int = Depends(session)):
        if "endpoint" not in body.subscription:
            raise HTTPException(400, "invalid subscription")
        rt.push.subscribe(body.subscription)
        return {"ok": True}

    @app.post("/api/push/unsubscribe")
    def push_unsubscribe(body: EndpointBody, _: int = Depends(session)):
        rt.push.unsubscribe(body.endpoint)
        return {"ok": True}

    @app.post("/api/push/test")
    def push_test(_: int = Depends(session)):
        return {"sent": rt.push.send("Prueba de avisos", "Las notificaciones de la habitación funcionan", "/", "test")}

    @app.get("/sw.js")
    def service_worker():
        return FileResponse(WEB_DIR / "sw.js", media_type="text/javascript", headers={"Cache-Control": "no-cache"})

    app.mount("/", StaticFiles(directory=WEB_DIR, html=True), name="web")
    return app
