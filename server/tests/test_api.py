import threading
import time

from frames import jpeg, scene


def test_mode_snooze_and_status(env):
    assert env.post("/api/mode", {"mode": "bogus"}).status_code == 400
    env.post("/api/mode", {"mode": "armed"})
    status = env.get("/api/status").json()
    assert status["armed"] and status["mode"] == "armed"
    env.post("/api/snooze", {"minutes": 30})
    status = env.get("/api/status").json()
    assert not status["armed"] and status["reason"] == "snoozed"
    env.post("/api/snooze", {"minutes": 0})
    assert env.get("/api/status").json()["armed"]


def test_auto_mode_follows_presence(env):
    env.post("/api/mode", {"mode": "auto"})
    assert env.get("/api/status").json()["reason"] == "auto-without-device"
    env.rt.presence.external = True
    assert not env.get("/api/status").json()["armed"]
    env.rt.presence.external = False
    status = env.get("/api/status").json()
    assert status["armed"] and status["reason"] == "away"


def test_status_reports_storage_detection_and_channels(env):
    body = env.get("/api/status").json()
    assert body["channels"] == [] and body["detection"]["state"] == "off"
    assert body["storage"]["events"] == 0 and body["storage"]["cap_bytes"] == int(env.cfg.max_disk_gb * 1024**3)


def test_snapshot_and_live_stream(env):
    assert env.get("/api/snapshot").status_code == 503
    frame = env.frame()
    assert env.get("/api/snapshot").content == frame

    def feed():
        for _ in range(30):
            time.sleep(0.05)
            env.rt.grabber._publish(time.time(), frame)

    threading.Thread(target=feed, daemon=True).start()
    with env.client.stream("GET", "/api/live") as response:
        assert response.status_code == 200 and "multipart/x-mixed-replace" in response.headers["content-type"]
        data = b""
        for chunk in response.iter_bytes():
            data += chunk
            if data.count(b"--frame") >= 3:
                break
    assert data.count(b"Content-Type: image/jpeg") >= 3


def test_snapshot_refuses_a_stale_frame(env):
    env.rt.grabber._publish(time.time() - 60, jpeg(scene()))
    assert env.get("/api/snapshot").status_code == 503


def test_live_is_refused_in_standby(env):
    env.rt.grabber._standby = True
    assert env.get("/api/live").status_code == 503


def test_security_headers_and_caching(anonymous):
    response = anonymous.get("/")
    assert "script-src 'self'" in response.headers["content-security-policy"]
    assert response.headers["x-frame-options"] == "DENY"
    assert anonymous.get("/api/auth/state").headers["cache-control"] == "no-store"
    assert anonymous.get("/style.css").headers["cache-control"] == "private, no-cache"
    assert anonymous.get("/js/main.js").headers["cache-control"] == "private, no-cache"


def test_clip_api_flow(env):
    assert env.post("/api/clips/stop").status_code == 404
    assert env.post("/api/clips/start", {"seconds": 30}).status_code == 409
    env.frame(3)
    assert env.post("/api/clips/start", {"seconds": 1}).status_code == 422
    assert env.post("/api/clips/start", {"seconds": 30}).json()["active"] is True
    assert env.get("/api/status").json()["clip"]["active"] is True
    assert env.post("/api/clips/stop").status_code == 200
    assert env.get("/api/status").json()["clip"]["active"] is False


def test_photo_lifecycle(env):
    assert env.post("/api/photos").status_code == 503
    env.frame()
    shot = env.post("/api/photos").json()
    assert shot["size"] > 1000
    assert [p["id"] for p in env.get("/api/photos").json()["photos"]] == [shot["id"]]
    assert env.get(f"/api/photos/{shot['id']}/img").headers["content-type"] == "image/jpeg"
    assert len(env.get(f"/api/photos/{shot['id']}/thumb").content) > 0
    assert "attachment" in env.get(f"/api/photos/{shot['id']}/img?download=1").headers["content-disposition"]
    stats = env.get("/api/status").json()["storage"]
    assert stats["photos"] == 1 and stats["photo_bytes"] > 0
    assert env.delete(f"/api/photos/{shot['id']}").status_code == 200
    assert env.get(f"/api/photos/{shot['id']}/img").status_code == 404
    assert not list((env.cfg.data_dir / "photos").rglob("*.jpg"))


def test_photo_bulk_delete_and_limit(env, monkeypatch):
    from camd import photos

    env.frame()
    ids = [env.post("/api/photos").json()["id"] for _ in range(3)]
    assert env.post("/api/photos/delete", {"ids": ids[:2] + [999]}).json() == {"removed": 2}
    monkeypatch.setattr(photos, "MAX_PHOTOS", 1)
    assert env.post("/api/photos").status_code == 409


def test_push_test_uses_configured_language(env):
    sent = []
    env.rt.notifier.send = lambda title, body, *rest: sent.append((title, body)) or 1
    env.cfg.language = "es"
    assert env.post("/api/push/test").json() == {"sent": 1}
    assert sent[0][0] == "Prueba de avisos"
