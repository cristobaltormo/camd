import time

from camd.config import Config
from camd.db import Database
from camd.events import EventRecorder, purge
from frames import jpeg, scene


def make(tmp_path, **kw):
    cfg = Config(data_dir=tmp_path, **kw)
    return cfg, Database(cfg.db_path)


def run(rec, start, n, motion, armed=True, fps=10):
    t = start
    for _ in range(n):
        rec.feed(t, jpeg(scene(blob=motion)), motion, 0.1 if motion else 0.0, armed)
        t += 1 / fps
    return t


def test_event_recorded_with_clip_and_thumb(tmp_path):
    cfg, db = make(tmp_path, post_roll_s=1.0)
    started, finished = [], []
    rec = EventRecorder(cfg, db, lambda *a: started.append(a), finished.append)
    t = run(rec, 1000.0, 20, motion=False)
    t = run(rec, t, 30, motion=True)
    assert rec.recording and len(started) == 1
    run(rec, t, 30, motion=False)
    rec.flush()
    row = db.one("SELECT * FROM events")
    assert row["status"] == "done" and finished
    assert row["end_ts"] > row["start_ts"]
    assert (tmp_path / "clips").exists()
    from pathlib import Path
    assert Path(row["clip_path"]).stat().st_size > 1000
    assert Path(row["thumb_path"]).stat().st_size > 1000


def test_disarmed_never_records(tmp_path):
    cfg, db = make(tmp_path)
    rec = EventRecorder(cfg, db)
    run(rec, 1000.0, 60, motion=True, armed=False)
    assert not rec.recording and db.one("SELECT 1 FROM events") is None


def test_short_blip_does_not_start(tmp_path):
    cfg, db = make(tmp_path)
    rec = EventRecorder(cfg, db)
    run(rec, 1000.0, 2, motion=True)
    run(rec, 1000.5, 20, motion=False)
    assert db.one("SELECT 1 FROM events") is None


def test_disarming_closes_open_event(tmp_path):
    cfg, db = make(tmp_path)
    rec = EventRecorder(cfg, db)
    t = run(rec, 1000.0, 20, motion=True)
    assert rec.recording
    run(rec, t, 2, motion=True, armed=False)
    rec.flush()
    assert not rec.recording and db.one("SELECT status FROM events")["status"] == "done"


def test_max_duration_splits_event(tmp_path):
    cfg, db = make(tmp_path, max_event_s=3.0)
    rec = EventRecorder(cfg, db)
    run(rec, 1000.0, 120, motion=True)
    rec.flush()
    assert db.one("SELECT COUNT(*) AS n FROM events WHERE status='done'")["n"] >= 2


def test_purge_by_age_and_disk(tmp_path):
    cfg, db = make(tmp_path, retention_days=1, max_disk_gb=0.000002)
    old = tmp_path / "old.mp4"
    old.write_bytes(b"x" * 10)
    db.execute("INSERT INTO events(start_ts, status, clip_path, size_bytes) VALUES(?, 'done', ?, 10)", (time.time() - 3 * 86400, str(old)))
    for i in range(3):
        db.execute("INSERT INTO events(start_ts, status, size_bytes) VALUES(?, 'done', 1000)", (time.time() - i,))
    removed = purge(cfg, db)
    assert removed >= 2 and not old.exists()
    assert db.one("SELECT COALESCE(SUM(size_bytes),0) AS s FROM events")["s"] <= cfg.max_disk_gb * 1024**3
