import json
import time
from pathlib import Path

import pytest
from frames import jpeg, scene

from camd.config import Config
from camd.db import Database
from camd.detect import Detection
from camd.events import EventRecorder
from camd.storage import purge

PERSON = Detection("person", 0.9, (0.2, 0.2, 0.3, 0.6))
CAT = Detection("cat", 0.7, (0.5, 0.5, 0.2, 0.2))


@pytest.fixture
def parts(tmp_path):
    def build(analysis=False, **overrides):
        cfg = Config(data_dir=tmp_path, **overrides)
        db = Database(cfg.db_path)
        log = {"start": [], "confirm": [], "finish": []}
        recorder = EventRecorder(
            cfg,
            db,
            lambda: analysis,
            lambda *a: log["start"].append(a),
            lambda *a: log["confirm"].append(a),
            lambda *a: log["finish"].append(a),
        )
        return cfg, db, recorder, log

    return build


def run(recorder, start, count, motion, armed=True, fps=10, score=0.1):
    t = start
    for _ in range(count):
        recorder.feed(t, jpeg(scene(blob=motion)), motion, score if motion else 0.0, armed)
        t += 1 / fps
    return t


def test_event_is_recorded_with_clip_and_thumbnail(parts):
    cfg, db, recorder, log = parts(post_roll_s=1.0)
    t = run(recorder, 1000.0, 20, motion=False)
    t = run(recorder, t, 30, motion=True)
    assert recorder.recording and len(log["start"]) == 1
    run(recorder, t, 30, motion=False)
    recorder.flush()
    row = db.one("SELECT * FROM events")
    assert row["status"] == "done" and log["finish"] and row["end_ts"] > row["start_ts"]
    assert Path(row["clip_path"]).stat().st_size > 1000 and Path(row["thumb_path"]).stat().st_size > 1000
    assert not list(cfg.spool_dir.glob("*"))


def test_disarmed_never_records(parts):
    _, db, recorder, _ = parts()
    run(recorder, 1000.0, 60, motion=True, armed=False)
    assert not recorder.recording and db.one("SELECT 1 FROM events") is None


def test_a_short_blip_does_not_start_an_event(parts):
    _, db, recorder, _ = parts()
    run(recorder, 1000.0, 2, motion=True)
    run(recorder, 1000.5, 20, motion=False)
    assert db.one("SELECT 1 FROM events") is None


def test_disarming_closes_the_open_event(parts):
    _, db, recorder, _ = parts()
    t = run(recorder, 1000.0, 20, motion=True)
    assert recorder.recording
    run(recorder, t, 2, motion=True, armed=False)
    recorder.flush()
    assert not recorder.recording and db.one("SELECT status FROM events")["status"] == "done"


def test_maximum_duration_splits_an_event(parts):
    _, db, recorder, _ = parts(max_event_s=3.0)
    run(recorder, 1000.0, 120, motion=True)
    recorder.flush()
    assert db.one("SELECT COUNT(*) AS n FROM events WHERE status='done'")["n"] >= 2


def test_stalled_event_is_closed_by_tick(parts):
    _, _, recorder, _ = parts(post_roll_s=1.0)
    t = run(recorder, 1000.0, 10, motion=False)
    run(recorder, t, 20, motion=True)
    assert recorder.recording
    recorder.tick(now=t + 60)
    assert not recorder.recording
    recorder.flush()


def test_confirmation_labels_the_event_and_announces_once(parts):
    _, db, recorder, log = parts(analysis=True, post_roll_s=1.0)
    t = run(recorder, 1000.0, 10, motion=False)
    t = run(recorder, t, 10, motion=True)
    assert log["start"][0][2] is True and not log["confirm"]
    recorder.observe(t, jpeg(scene(blob=True)), [PERSON])
    recorder.observe(t + 0.5, jpeg(scene(blob=True)), [PERSON, CAT])
    assert len(log["confirm"]) == 1 and log["confirm"][0][2] == ["person"]
    assert json.loads(db.one("SELECT labels FROM events")["labels"]) == ["person", "cat"]
    run(recorder, t, 40, motion=False)
    recorder.flush()
    row = db.one("SELECT * FROM events")
    assert row["confirmed"] == 1 and row["analyzed"] == 1
    assert [d["label"] for d in json.loads(row["detections"])] == ["person"]


def test_detection_arriving_just_before_the_event_still_counts(parts):
    _, _, recorder, log = parts(analysis=True)
    t = run(recorder, 1000.0, 5, motion=False)
    recorder.observe(t, jpeg(scene(blob=True)), [PERSON])
    run(recorder, t, 6, motion=True)
    assert log["confirm"] and log["confirm"][0][2] == ["person"]


def test_a_still_person_keeps_the_event_alive(parts):
    _, _, recorder, _ = parts(analysis=True, post_roll_s=2.0)
    t = run(recorder, 1000.0, 10, motion=False)
    t = run(recorder, t, 10, motion=True)
    for _ in range(5):
        recorder.observe(t, jpeg(scene()), [PERSON])
        t = run(recorder, t, 10, motion=False)
    assert recorder.recording


def test_strict_mode_discards_unconfirmed_events_without_encoding(parts):
    _, db, recorder, log = parts(analysis=True, detection="strict", post_roll_s=1.0)
    t = run(recorder, 1000.0, 10, motion=False)
    t = run(recorder, t, 20, motion=True)
    run(recorder, t, 30, motion=False)
    recorder.flush()
    assert db.one("SELECT 1 FROM events") is None and log["finish"]


def test_alert_mode_keeps_unconfirmed_events_for_review(parts):
    _, db, recorder, _ = parts(analysis=True, detection="alert", post_roll_s=1.0)
    t = run(recorder, 1000.0, 10, motion=False)
    t = run(recorder, t, 20, motion=True)
    run(recorder, t, 30, motion=False)
    recorder.flush()
    row = db.one("SELECT * FROM events")
    assert row["status"] == "done" and row["analyzed"] == 1 and row["confirmed"] == 0


def test_strict_mode_without_a_ready_detector_keeps_everything(parts):
    _, db, recorder, _ = parts(analysis=False, detection="strict", post_roll_s=1.0)
    t = run(recorder, 1000.0, 10, motion=False)
    t = run(recorder, t, 20, motion=True)
    run(recorder, t, 30, motion=False)
    recorder.flush()
    assert db.one("SELECT status, analyzed FROM events")["status"] == "done"


def test_manual_clip_is_kept_and_survives_purge(parts):
    cfg, db, recorder, log = parts(retention_days=1)
    t = run(recorder, 1000.0, 20, motion=False)
    event_id = recorder.start_manual(60, now=t)
    assert recorder.manual_state()["active"] and not log["start"]
    t = run(recorder, t, 30, motion=False, armed=False)
    assert recorder.recording
    assert recorder.stop_manual() == event_id
    recorder.flush()
    row = db.one("SELECT * FROM events")
    assert row["kind"] == "manual" and row["keep"] == 1 and row["status"] == "done"
    db.execute("UPDATE events SET start_ts=1 WHERE id=?", (event_id,))
    assert purge(cfg, db) == 0 and Path(row["clip_path"]).exists()


def test_manual_clip_ends_by_itself(parts):
    _, db, recorder, _ = parts()
    t = run(recorder, 1000.0, 10, motion=False)
    recorder.start_manual(2, now=t)
    run(recorder, t, 40, motion=False, armed=False)
    assert not recorder.recording
    recorder.flush()
    assert db.one("SELECT status FROM events")["status"] == "done"


def test_manual_clip_ignores_detections(parts):
    _, _, recorder, log = parts(analysis=True)
    t = run(recorder, 1000.0, 10, motion=False)
    recorder.start_manual(30, now=t)
    recorder.observe(t, jpeg(scene()), [PERSON])
    assert not log["confirm"]
    recorder.stop_manual()
    recorder.flush()


def test_starting_a_manual_clip_during_motion_keeps_the_event(parts):
    _, db, recorder, _ = parts(post_roll_s=1.0)
    t = run(recorder, 1000.0, 10, motion=False)
    t = run(recorder, t, 20, motion=True)
    event_id = recorder.start_manual(60, now=t)
    row = db.one("SELECT kind, keep FROM events WHERE id=?", (event_id,))
    assert row["kind"] == "manual" and row["keep"] == 1
    run(recorder, t, 40, motion=False)
    assert recorder.recording
    recorder.stop_manual()
    recorder.flush()


def test_interrupted_recordings_are_recovered_from_the_spool(parts):
    cfg, db, recorder, _ = parts()
    run(recorder, 1000.0, 10, motion=False)
    run(recorder, 1001.0, 25, motion=True)
    assert recorder.recording
    event_id = db.one("SELECT id FROM events")["id"]
    recorder._current.spool.close()

    cfg2, db2, restarted, _ = parts()
    restarted.recover()
    restarted.flush()
    row = db2.one("SELECT * FROM events WHERE id=?", (event_id,))
    assert row["status"] == "done" and Path(row["clip_path"]).exists()
    assert not list(cfg2.spool_dir.glob("*"))


def test_recovery_drops_empty_spools_and_orphans(parts):
    cfg, db, recorder, _ = parts()
    db.execute("INSERT INTO events(start_ts, status) VALUES(1, 'recording')")
    cfg.spool_dir.mkdir(parents=True)
    (cfg.spool_dir / "99.spool").write_bytes(b"")
    recorder.recover()
    assert db.one("SELECT 1 FROM events") is None and not list(cfg.spool_dir.glob("*"))


def test_purge_by_age_and_disk_cap(tmp_path):
    cfg = Config(data_dir=tmp_path, retention_days=1, max_disk_gb=0.000002)
    db = Database(cfg.db_path)
    old = tmp_path / "old.mp4"
    old.write_bytes(b"x" * 10)
    db.execute("INSERT INTO events(start_ts, status, clip_path, size_bytes) VALUES(?, 'done', ?, 10)", (time.time() - 3 * 86400, str(old)))
    for i in range(3):
        db.execute("INSERT INTO events(start_ts, status, size_bytes) VALUES(?, 'done', 1000)", (time.time() - i,))
    assert purge(cfg, db) >= 2 and not old.exists()
    assert db.one("SELECT COALESCE(SUM(size_bytes), 0) AS s FROM events")["s"] <= cfg.max_disk_gb * 1024**3


def test_unconfirmed_events_expire_sooner_but_only_when_analysed(tmp_path):
    cfg = Config(data_dir=tmp_path, detection="alert", unconfirmed_keep_h=1.0)
    db = Database(cfg.db_path)
    long_ago = time.time() - 5 * 3600
    db.execute("INSERT INTO events(start_ts, status, analyzed, confirmed) VALUES(?, 'done', 1, 0)", (long_ago,))
    db.execute("INSERT INTO events(start_ts, status, analyzed, confirmed) VALUES(?, 'done', 1, 1)", (long_ago,))
    db.execute("INSERT INTO events(start_ts, status, analyzed, confirmed) VALUES(?, 'done', 0, 0)", (long_ago,))
    assert purge(cfg, db) == 1
    assert db.one("SELECT COUNT(*) AS n FROM events")["n"] == 2
    cfg.detection = "off"
    db.execute("UPDATE events SET analyzed=1, confirmed=0")
    assert purge(cfg, db) == 0


def test_a_full_disk_closes_the_event_instead_of_wedging_it(parts, monkeypatch):
    _, db, recorder, _ = parts()
    t = run(recorder, 1000.0, 10, motion=False)
    t = run(recorder, t, 10, motion=True)
    assert recorder.recording

    def full(*_):
        raise OSError("no space left on device")

    monkeypatch.setattr(recorder._current.spool, "append", full)
    run(recorder, t, 2, motion=True)
    assert not recorder.recording
    recorder.flush()
