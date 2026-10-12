import time

import pytest

from camd import prefs
from camd.config import Config
from camd.db import Database
from test_api import ORIGIN, H, enroll, env  # noqa: F401
from authenticator import SoftAuthenticator


def login(env):
    cfg, db, app, client = env
    enroll(app, client, SoftAuthenticator("localhost", ORIGIN))
    return cfg, db, app, client


def test_settings_roundtrip_applies_live(env):
    cfg, db, app, client = login(env)
    r = client.post("/api/settings", json={"post_roll_s": 25, "notify_motion": False, "camera_brightness": 1, "quiet_start": "22:30"}, headers=H)
    assert r.status_code == 200, r.text
    got = client.get("/api/settings").json()
    assert got["post_roll_s"] == 25 and got["notify_motion"] is False and got["camera_brightness"] == 1
    assert cfg.post_roll_s == 25 and cfg.notify_motion is False
    assert got["sensitivity"] == 5


def test_partial_update_keeps_motion_settings(env):
    cfg, db, app, client = login(env)
    client.post("/api/settings", json={"sensitivity": 8, "ignore_rects": [[0.1, 0.1, 0.2, 0.2]], "retention_days": 30}, headers=H)
    client.post("/api/settings", json={"pre_roll_s": 5}, headers=H)
    got = client.get("/api/settings").json()
    assert got["sensitivity"] == 8 and got["ignore_rects"] == [[0.1, 0.1, 0.2, 0.2]] and got["retention_days"] == 30


@pytest.mark.parametrize("body", [{"post_roll_s": 9999}, {"quiet_start": "25:00"}, {"notify_motion": "yes"}, {"camera_brightness": 3}, {"nope": 1}])
def test_invalid_settings_rejected(env, body):
    _, _, _, client = login(env)
    assert client.post("/api/settings", json=body, headers=H).status_code in (400, 422)


def test_prefs_persist_across_restart(env, tmp_path):
    cfg, db, app, client = login(env)
    client.post("/api/settings", json={"max_disk_gb": 3, "quiet_enabled": True}, headers=H)
    fresh = Config(data_dir=cfg.data_dir)
    prefs.load(fresh, db)
    assert fresh.max_disk_gb == 3 and fresh.quiet_enabled is True


def test_quiet_hours_wrap_midnight():
    cfg = Config(quiet_enabled=True, quiet_start="23:00", quiet_end="07:00")
    at = lambda h, m: time.mktime((2026, 10, 12, h, m, 0, 0, 0, -1))
    assert prefs.in_quiet_hours(cfg, at(23, 30)) and prefs.in_quiet_hours(cfg, at(3, 0)) and prefs.in_quiet_hours(cfg, at(6, 59))
    assert not prefs.in_quiet_hours(cfg, at(7, 0)) and not prefs.in_quiet_hours(cfg, at(12, 0))
    cfg.quiet_enabled = False
    assert not prefs.in_quiet_hours(cfg, at(3, 0))


def test_quiet_hours_same_day_window():
    cfg = Config(quiet_enabled=True, quiet_start="13:00", quiet_end="15:00")
    at = lambda h: time.mktime((2026, 10, 12, h, 0, 0, 0, 0, -1))
    assert prefs.in_quiet_hours(cfg, at(14)) and not prefs.in_quiet_hours(cfg, at(16))


def test_bulk_delete_and_count(env):
    cfg, db, app, client = login(env)
    now = time.time()
    for i in range(3):
        db.execute("INSERT INTO events(start_ts, end_ts, status) VALUES(?, ?, 'done')", (now - i * 100, now - i * 100 + 5))
    assert client.get("/api/events/count", params={"since": now - 150}).json()["count"] == 2
    ids = [r["id"] for r in db.query("SELECT id FROM events")]
    r = client.post("/api/events/delete", json={"ids": ids[:2] + [9999]}, headers=H)
    assert r.json() == {"removed": 2}
    assert len(client.get("/api/events").json()["events"]) == 1
