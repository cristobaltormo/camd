import time

import pytest

from camd import prefs
from camd.config import Config


def test_settings_apply_live(env):
    response = env.post("/api/settings", {"post_roll_s": 25, "notify_motion": False, "camera_brightness": 1, "quiet_start": "22:30"})
    assert response.status_code == 200, response.text
    got = env.get("/api/settings").json()
    assert got["post_roll_s"] == 25 and got["notify_motion"] is False and got["camera_brightness"] == 1
    assert env.cfg.post_roll_s == 25 and got["sensitivity"] == 5


def test_motion_settings_roundtrip_and_validation(env):
    ok = env.post("/api/settings", {"sensitivity": 8, "ignore_rects": [[0.1, 0.1, 0.2, 0.2]], "retention_days": 7})
    assert ok.status_code == 200
    got = env.get("/api/settings").json()
    assert (got["sensitivity"], got["ignore_rects"], got["retention_days"]) == (8, [[0.1, 0.1, 0.2, 0.2]], 7)
    assert env.post("/api/settings", {"ignore_rects": [[2, 0, 0, 0]]}).status_code == 400


def test_partial_update_keeps_other_settings(env):
    env.post("/api/settings", {"sensitivity": 8, "ignore_rects": [[0.1, 0.1, 0.2, 0.2]], "retention_days": 30})
    env.post("/api/settings", {"pre_roll_s": 5})
    got = env.get("/api/settings").json()
    assert got["sensitivity"] == 8 and got["ignore_rects"] == [[0.1, 0.1, 0.2, 0.2]] and got["retention_days"] == 30


@pytest.mark.parametrize(
    "body",
    [
        {"post_roll_s": 9999},
        {"quiet_start": "25:00"},
        {"notify_motion": "yes"},
        {"camera_brightness": 3},
        {"nope": 1},
        {"detection": "maybe"},
        {"detect_labels": ["unicorn"]},
        {"detect_labels": []},
        {"language": "xx"},
        {"detect_confidence": 0.99},
    ],
)
def test_invalid_settings_are_rejected(env, body):
    assert env.post("/api/settings", body).status_code in (400, 422)


def test_detection_settings_are_stored_and_deduplicated(env):
    ok = env.post("/api/settings", {"detection": "strict", "detect_labels": ["person", "dog", "person"], "detect_confidence": 0.6})
    assert ok.status_code == 200
    got = env.get("/api/settings").json()
    assert got["detection"] == "strict" and got["detect_labels"] == ["person", "dog"] and got["detect_confidence"] == 0.6


def test_prefs_persist_across_restarts(env):
    env.post("/api/settings", {"max_disk_gb": 3, "quiet_enabled": True, "detection": "alert"})
    fresh = Config(data_dir=env.cfg.data_dir)
    prefs.load(fresh, env.db)
    assert fresh.max_disk_gb == 3 and fresh.quiet_enabled is True and fresh.detection == "alert"


def test_quiet_hours_wrap_midnight():
    cfg = Config(quiet_enabled=True, quiet_start="23:00", quiet_end="07:00")

    def at(hour, minute):
        return time.mktime((2026, 10, 12, hour, minute, 0, 0, 0, -1))

    assert prefs.in_quiet_hours(cfg, at(23, 30)) and prefs.in_quiet_hours(cfg, at(3, 0)) and prefs.in_quiet_hours(cfg, at(6, 59))
    assert not prefs.in_quiet_hours(cfg, at(7, 0)) and not prefs.in_quiet_hours(cfg, at(12, 0))
    cfg.quiet_enabled = False
    assert not prefs.in_quiet_hours(cfg, at(3, 0))


def test_quiet_hours_within_one_day():
    cfg = Config(quiet_enabled=True, quiet_start="13:00", quiet_end="15:00")

    def at(hour):
        return time.mktime((2026, 10, 12, hour, 0, 0, 0, 0, -1))

    assert prefs.in_quiet_hours(cfg, at(14)) and not prefs.in_quiet_hours(cfg, at(16))


def test_resolution_is_a_known_size_and_is_sent_to_the_camera(env, monkeypatch):
    sent = []
    monkeypatch.setattr(env.rt.camera, "apply_controls", lambda controls: sent.append(controls) or {})
    assert env.post("/api/settings", {"camera_framesize": 640}).status_code == 400
    assert env.post("/api/settings", {"camera_framesize": 8}).status_code == 200
    env.rt.apply_camera()
    assert sent[-1]["framesize"] == 8
    env.post("/api/settings", {"camera_framesize": 0})
    env.rt.apply_camera()
    assert "framesize" not in sent[-1]


@pytest.mark.parametrize("value", [1, 3, 64, True])
def test_jpeg_quality_must_be_zero_or_within_the_camera_range(env, value):
    assert env.post("/api/settings", {"camera_quality": value}).status_code in (400, 422)
    assert env.post("/api/settings", {"camera_quality": 0}).status_code == 200
    assert env.post("/api/settings", {"camera_quality": 12}).status_code == 200


def test_alert_zones_roundtrip_and_validate(env):
    assert env.post("/api/settings", {"alert_rects": [[0.1, 0.2, 0.3, 0.4]]}).status_code == 200
    assert env.get("/api/settings").json()["alert_rects"] == [[0.1, 0.2, 0.3, 0.4]]
    assert env.post("/api/settings", {"alert_rects": [[0.1, 0.2, 0.3, 2]]}).status_code == 400
    assert env.get("/api/settings").json()["alert_rects"] == [[0.1, 0.2, 0.3, 0.4]]
