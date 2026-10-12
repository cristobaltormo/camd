import threading
import time
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from authenticator import SoftAuthenticator
from camd.app import create_app
from camd.config import Config
from camd.db import Database
from frames import jpeg, scene

ORIGIN = "http://localhost:8180"
H = {"Origin": ORIGIN}


@pytest.fixture
def env(tmp_path):
    cfg = Config(data_dir=tmp_path, rp_id="localhost", origin=ORIGIN, camera_token="t")
    db = Database(cfg.db_path)
    app = create_app(cfg, db, start=False)
    client = TestClient(app, base_url=ORIGIN)
    return cfg, db, app, client


def enroll(app, client, auth: SoftAuthenticator):
    code = app.state.auth.new_enroll_code()
    opts = client.post("/api/auth/register/options", json={"code": code}, headers=H).json()
    r = client.post(
        "/api/auth/register/verify",
        json={"code": code, "credential": auth.create(opts), "name": "test"},
        headers=H,
    )
    assert r.status_code == 200, r.text


def test_requires_login(env):
    _, _, _, client = env
    for path in ("/api/status", "/api/events", "/api/live", "/api/snapshot", "/api/settings", "/api/push/key"):
        assert client.get(path).status_code == 401
    assert client.get("/healthz").status_code == 200
    assert client.get("/").status_code == 200


def test_register_login_logout_flow(env):
    cfg, db, app, client = env
    auth = SoftAuthenticator("localhost", ORIGIN)
    enroll(app, client, auth)
    assert client.get("/api/status").status_code == 200

    other = TestClient(app, base_url=ORIGIN)
    opts = other.post("/api/auth/login/options", headers=H).json()
    r = other.post("/api/auth/login/verify", json={"cid": opts["cid"], "credential": auth.get(opts["options"])}, headers=H)
    assert r.status_code == 200 and other.get("/api/status").status_code == 200

    other.post("/api/auth/logout", headers=H)
    assert other.get("/api/status").status_code == 401
    assert client.get("/api/status").status_code == 200


def test_bad_signature_and_replay_rejected(env):
    _, _, app, client = env
    auth = SoftAuthenticator("localhost", ORIGIN)
    enroll(app, client, auth)
    fresh = TestClient(app, base_url=ORIGIN)

    opts = fresh.post("/api/auth/login/options", headers=H).json()
    bad = fresh.post("/api/auth/login/verify", json={"cid": opts["cid"], "credential": auth.get(opts["options"], corrupt=True)}, headers=H)
    assert bad.status_code == 401

    opts = fresh.post("/api/auth/login/options", headers=H).json()
    cred = auth.get(opts["options"])
    assert fresh.post("/api/auth/login/verify", json={"cid": opts["cid"], "credential": cred}, headers=H).status_code == 200
    again = TestClient(app, base_url=ORIGIN)
    assert again.post("/api/auth/login/verify", json={"cid": opts["cid"], "credential": cred}, headers=H).status_code == 401


def test_enroll_code_is_single_use_and_wrong_code_rejected(env):
    _, _, app, client = env
    r = client.post("/api/auth/register/options", json={"code": "deadbeef"}, headers=H)
    assert r.status_code == 400
    auth = SoftAuthenticator("localhost", ORIGIN)
    code = app.state.auth.new_enroll_code()
    opts = client.post("/api/auth/register/options", json={"code": code}, headers=H).json()
    cred = auth.create(opts)
    assert client.post("/api/auth/register/verify", json={"code": code, "credential": cred}, headers=H).status_code == 200
    assert client.post("/api/auth/register/verify", json={"code": code, "credential": cred}, headers=H).status_code == 400


def test_wrong_origin_blocked_on_writes(env):
    _, _, app, client = env
    enroll(app, client, SoftAuthenticator("localhost", ORIGIN))
    assert client.post("/api/mode", json={"mode": "armed"}, headers={"Origin": "https://evil.example"}).status_code == 403
    assert client.post("/api/mode", json={"mode": "armed"}).status_code == 403
    assert client.post("/api/mode", json={"mode": "armed"}, headers=H).status_code == 200


def test_rate_limit_on_auth(env):
    _, _, _, client = env
    codes = [client.post("/api/auth/register/options", json={"code": f"abcd{i:04d}"}, headers=H).status_code for i in range(14)]
    assert 429 in codes


def test_mode_snooze_settings_status(env):
    _, _, app, client = env
    enroll(app, client, SoftAuthenticator("localhost", ORIGIN))
    assert client.post("/api/mode", json={"mode": "bogus"}, headers=H).status_code == 400
    client.post("/api/mode", json={"mode": "armed"}, headers=H)
    s = client.get("/api/status").json()
    assert s["armed"] and s["mode"] == "armed"
    client.post("/api/snooze", json={"minutes": 30}, headers=H)
    s = client.get("/api/status").json()
    assert not s["armed"] and s["reason"] == "snoozed"
    client.post("/api/snooze", json={"minutes": 0}, headers=H)

    ok = client.post("/api/settings", json={"sensitivity": 8, "ignore_rects": [[0.1, 0.1, 0.2, 0.2]], "retention_days": 7}, headers=H)
    assert ok.status_code == 200
    got = client.get("/api/settings").json()
    assert {k: got[k] for k in ("sensitivity", "ignore_rects", "retention_days")} == {"sensitivity": 8, "ignore_rects": [[0.1, 0.1, 0.2, 0.2]], "retention_days": 7}
    bad = client.post("/api/settings", json={"sensitivity": 8, "ignore_rects": [[2, 0, 0, 0]], "retention_days": 7}, headers=H)
    assert bad.status_code == 400


def test_auto_mode_follows_presence(env):
    _, _, app, client = env
    enroll(app, client, SoftAuthenticator("localhost", ORIGIN))
    rt = app.state.runtime
    client.post("/api/mode", json={"mode": "auto"}, headers=H)
    assert client.get("/api/status").json()["reason"] == "auto-without-device"
    rt.presence.external = True
    assert not client.get("/api/status").json()["armed"]
    rt.presence.external = False
    s = client.get("/api/status").json()
    assert s["armed"] and s["reason"] == "away"


def test_events_clip_range_thumb_delete(env):
    cfg, db, app, client = env
    enroll(app, client, SoftAuthenticator("localhost", ORIGIN))
    folder = cfg.clips_dir / "2026" / "10" / "12"
    folder.mkdir(parents=True)
    clip, thumb = folder / "1.mp4", folder / "1.jpg"
    clip.write_bytes(bytes(range(256)) * 40)
    thumb.write_bytes(jpeg(scene()))
    db.execute("INSERT INTO events(start_ts, end_ts, status, clip_path, thumb_path, size_bytes) VALUES(?,?, 'done', ?, ?, 1)", (100.0, 110.0, str(clip), str(thumb)))

    ev = client.get("/api/events").json()["events"][0]
    assert ev["duration"] == 10.0
    full = client.get(f"/api/events/{ev['id']}/clip")
    assert full.status_code == 200 and full.headers["content-type"] == "video/mp4"
    part = client.get(f"/api/events/{ev['id']}/clip", headers={"Range": "bytes=0-99"})
    assert part.status_code == 206 and len(part.content) == 100
    assert client.get(f"/api/events/{ev['id']}/thumb").headers["content-type"] == "image/jpeg"

    assert client.delete(f"/api/events/{ev['id']}", headers=H).status_code == 200
    assert not clip.exists() and client.get(f"/api/events/{ev['id']}/clip").status_code == 404


def test_clip_path_outside_clips_dir_is_refused(env, tmp_path_factory):
    cfg, db, app, client = env
    enroll(app, client, SoftAuthenticator("localhost", ORIGIN))
    secret = tmp_path_factory.mktemp("outside") / "secret.mp4"
    secret.write_bytes(b"secret")
    db.execute("INSERT INTO events(start_ts, status, clip_path) VALUES(1, 'done', ?)", (str(secret),))
    assert client.get("/api/events/1/clip").status_code == 404


def test_cannot_remove_last_passkey(env):
    _, _, app, client = env
    auth = SoftAuthenticator("localhost", ORIGIN)
    enroll(app, client, auth)
    cred_id = client.get("/api/auth/devices").json()["credentials"][0]["id"]
    assert client.delete(f"/api/auth/credentials/{cred_id}", headers=H).status_code == 400


def test_snapshot_and_live_stream(env):
    cfg, _, app, client = env
    enroll(app, client, SoftAuthenticator("localhost", ORIGIN))
    rt = app.state.runtime
    assert client.get("/api/snapshot").status_code == 503
    frame = jpeg(scene())
    rt.grabber._publish(time.time(), frame)
    assert client.get("/api/snapshot").content == frame

    def feed():
        for _ in range(30):
            time.sleep(0.05)
            rt.grabber._publish(time.time(), frame)

    threading.Thread(target=feed, daemon=True).start()
    with client.stream("GET", "/api/live") as r:
        assert r.status_code == 200 and "multipart/x-mixed-replace" in r.headers["content-type"]
        data = b""
        for chunk in r.iter_bytes():
            data += chunk
            if data.count(b"--frame") >= 3:
                break
    assert data.count(b"Content-Type: image/jpeg") >= 3


def test_security_headers(env):
    _, _, _, client = env
    r = client.get("/")
    assert "script-src 'self'" in r.headers["content-security-policy"]
    assert r.headers["x-frame-options"] == "DENY"
    assert client.get("/api/status").headers["cache-control"] == "no-store"


def test_status_reports_storage_and_notifich(env):
    cfg, db, app, client = env
    enroll(app, client, SoftAuthenticator("localhost", ORIGIN))
    body = client.get("/api/status").json()
    assert body["notifich"] is False
    assert body["storage"]["events"] == 0 and body["storage"]["cap_bytes"] == int(cfg.max_disk_gb * 1024**3)


def test_notifich_channel_counts_as_sent(tmp_path, monkeypatch):
    from camd import push as push_module

    calls = []

    class Answer:
        ok = True
        status_code = 200

    monkeypatch.setattr(push_module.requests, "post", lambda url, **kw: calls.append((url, kw)) or Answer())
    db = Database(tmp_path / "t.sqlite3")
    channel = push_module.Push(db, tmp_path / "vapid.pem", "https://localhost", "https://notifich.example/", "secret")
    assert channel.notifich_enabled
    assert channel.send("Titulo", "Cuerpo") == 1
    url, kw = calls[0]
    assert url == "https://notifich.example/api/notify"
    assert kw["headers"] == {"X-API-Key": "secret"} and kw["json"] == {"title": "Titulo", "body": "Cuerpo"}


def test_notifich_failure_does_not_raise(tmp_path, monkeypatch):
    from camd import push as push_module

    def boom(*_a, **_k):
        raise push_module.requests.ConnectionError("down")

    monkeypatch.setattr(push_module.requests, "post", boom)
    channel = push_module.Push(Database(tmp_path / "t.sqlite3"), tmp_path / "vapid.pem", "https://localhost", "https://x", "k")
    assert channel.send("a", "b") == 0


def test_static_files_revalidate_and_api_is_never_cached(env):
    _, _, _, client = env
    assert client.get("/style.css").headers["cache-control"] == "no-cache"
    assert client.get("/js/main.js").headers["cache-control"] == "no-cache"
    assert client.get("/api/auth/state").headers["cache-control"] == "no-store"
