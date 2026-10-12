import time

from authenticator import SoftAuthenticator
from camd.grabber import retry_delay
from camd import runtime as runtime_module
from frames import jpeg, scene
from test_api import ORIGIN, H, enroll, env  # noqa: F401


def login(env):
    cfg, db, app, client = env
    enroll(app, client, SoftAuthenticator("localhost", ORIGIN))
    return cfg, db, app, client


def test_retry_backs_off_to_thirty_seconds():
    delays = [retry_delay(n) for n in range(0, 12)]
    assert delays[0] == delays[1] == 1.0
    assert delays == sorted(delays) and max(delays) == 30.0


def test_disabling_the_camera_is_reflected_everywhere(env):
    cfg, db, app, client = login(env)
    rt = app.state.runtime
    rt.grabber._publish(time.time(), jpeg(scene()))
    assert client.get("/api/status").json()["camera"]["enabled"] is True
    assert client.post("/api/settings", json={"camera_enabled": False}, headers=H).status_code == 200
    camera = client.get("/api/status").json()["camera"]
    assert camera["enabled"] is False and camera["online"] is False
    assert not rt.grabber.enabled and rt.grabber.latest is None
    assert client.get("/api/live").status_code == 503
    assert client.get("/api/snapshot").status_code == 503
    assert client.post("/api/photos", headers=H).status_code == 503
    assert client.post("/api/clips/start", json={"seconds": 30}, headers=H).status_code == 409
    assert client.post("/api/privacy", json={"on": True}, headers=H).status_code == 409
    assert client.post("/api/settings", json={"camera_enabled": True}, headers=H).status_code == 200
    assert rt.grabber.enabled


def test_frames_in_flight_are_dropped_while_disabled(env):
    cfg, db, app, client = login(env)
    grabber = app.state.runtime.grabber
    grabber.set_enabled(False)
    grabber._publish(time.time(), jpeg(scene()))
    assert grabber.latest is None and not grabber.online
    grabber.set_enabled(True)
    grabber._publish(time.time(), jpeg(scene()))
    assert grabber.online


def test_no_offline_alert_while_disabled_and_none_right_after_enabling(env, monkeypatch):
    cfg, db, app, client = login(env)
    rt = app.state.runtime
    sent = []
    monkeypatch.setattr(rt.push, "send", lambda *a, **k: sent.append(a[0]) or 1)
    rt._started_at = time.time() - 3600

    rt.cfg.camera_enabled = False
    rt._check_offline()
    assert sent == []

    rt.apply_prefs({"camera_enabled"})
    rt.cfg.camera_enabled = True
    rt.apply_prefs({"camera_enabled"})
    rt._check_offline()
    assert sent == []

    rt._enabled_at = time.time() - 3600
    rt._check_offline()
    rt._check_offline()
    assert sent == ["Cámara sin conexión"]


def test_recovery_notice_after_an_alert(env, monkeypatch):
    cfg, db, app, client = login(env)
    rt = app.state.runtime
    sent = []
    monkeypatch.setattr(rt.push, "send", lambda *a, **k: sent.append(a[0]) or 1)
    rt._started_at = time.time() - 3600
    rt._check_offline()
    rt.grabber._publish(time.time(), jpeg(scene()))
    rt._check_offline()
    rt._check_offline()
    assert sent == ["Cámara sin conexión", "Cámara de vuelta"]


def test_woken_camera_is_put_back_in_standby_when_privacy_is_on(env, monkeypatch):
    cfg, db, app, client = login(env)
    rt = app.state.runtime
    calls = []
    monkeypatch.setattr(rt.grabber, "set_standby", lambda on: calls.append(on) or True)
    rt.grabber._standby = True
    rt.camera_info = {"standby": 0}
    rt._enforce_standby()
    assert calls == [True]
    rt.camera_info = {"standby": 1}
    rt._enforce_standby()
    assert calls == [True]
