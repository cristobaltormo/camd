import time
from pathlib import Path

from authenticator import SoftAuthenticator
from camd.config import Config
from camd.db import Database
from camd.events import EventRecorder, purge
from frames import jpeg, scene
from test_api import ORIGIN, H, enroll, env  # noqa: F401
from test_events import make, run


def login(env):
    cfg, db, app, client = env
    enroll(app, client, SoftAuthenticator("localhost", ORIGIN))
    return cfg, db, app, client


def publish(app, n=1):
    for _ in range(n):
        app.state.runtime.grabber._publish(time.time(), jpeg(scene()))


def test_manual_clip_is_kept_and_survives_purge(tmp_path):
    cfg, db = make(tmp_path, retention_days=1)
    started = []
    rec = EventRecorder(cfg, db, lambda *a: started.append(a), lambda _id: None)
    t = run(rec, 1000.0, 20, motion=False)
    event_id = rec.start_manual(60, now=t)
    assert rec.manual_state()["active"] and not started
    t = run(rec, t, 30, motion=False, armed=False)
    assert rec.recording
    assert rec.stop_manual() == event_id
    rec.flush()
    row = db.one("SELECT * FROM events")
    assert row["kind"] == "manual" and row["keep"] == 1 and row["status"] == "done"
    db.execute("UPDATE events SET start_ts=1 WHERE id=?", (event_id,))
    assert purge(cfg, db) == 0
    assert Path(row["clip_path"]).exists()


def test_manual_clip_ends_by_itself(tmp_path):
    cfg, db = make(tmp_path)
    rec = EventRecorder(cfg, db)
    t = run(rec, 1000.0, 10, motion=False)
    rec.start_manual(2, now=t)
    run(rec, t, 40, motion=False, armed=False)
    assert not rec.recording
    rec.flush()
    assert db.one("SELECT status FROM events")["status"] == "done"


def test_starting_manual_during_motion_event_keeps_it(tmp_path):
    cfg, db = make(tmp_path, post_roll_s=1.0)
    rec = EventRecorder(cfg, db)
    t = run(rec, 1000.0, 10, motion=False)
    t = run(rec, t, 20, motion=True)
    assert rec.recording
    event_id = rec.start_manual(60, now=t)
    row = db.one("SELECT kind, keep FROM events WHERE id=?", (event_id,))
    assert row["kind"] == "manual" and row["keep"] == 1
    run(rec, t, 40, motion=False)
    assert rec.recording
    rec.stop_manual()
    rec.flush()


def test_stalled_event_is_closed_by_tick(tmp_path):
    cfg, db = make(tmp_path, post_roll_s=1.0)
    rec = EventRecorder(cfg, db)
    t = run(rec, 1000.0, 10, motion=False)
    run(rec, t, 20, motion=True)
    assert rec.recording
    rec.tick(now=t + 60)
    assert not rec.recording
    rec.flush()


def test_stop_without_clip_is_404(env):
    _, _, app, client = login(env)
    assert client.post("/api/clips/stop", headers=H).status_code == 404
    assert client.post("/api/clips/start", json={"seconds": 30}, headers=H).status_code == 409
    publish(app)
    assert client.post("/api/clips/start", json={"seconds": 1}, headers=H).status_code == 422


def test_clip_api_flow(env):
    cfg, db, app, client = login(env)
    publish(app, 3)
    assert client.post("/api/clips/start", json={"seconds": 30}, headers=H).json()["active"] is True
    assert client.get("/api/status").json()["clip"]["active"] is True
    assert client.post("/api/clips/stop", headers=H).status_code == 200
    assert client.get("/api/status").json()["clip"]["active"] is False


def test_keep_toggle_and_listing(env):
    cfg, db, app, client = login(env)
    db.execute("INSERT INTO events(start_ts, end_ts, status) VALUES(1, 2, 'done')")
    event_id = db.one("SELECT id FROM events")["id"]
    assert client.post(f"/api/events/{event_id}/keep", json={"keep": True}, headers=H).json()["keep"] is True
    listed = client.get("/api/events").json()["events"][0]
    assert listed["keep"] is True and listed["kind"] == "motion"
    assert client.post("/api/events/999/keep", json={"keep": True}, headers=H).status_code == 404


def test_photo_lifecycle(env):
    cfg, db, app, client = login(env)
    assert client.post("/api/photos", headers=H).status_code == 503
    publish(app)
    shot = client.post("/api/photos", headers=H).json()
    assert shot["size"] > 1000
    listed = client.get("/api/photos").json()["photos"]
    assert [p["id"] for p in listed] == [shot["id"]]
    assert client.get(f"/api/photos/{shot['id']}/img").headers["content-type"] == "image/jpeg"
    thumb = client.get(f"/api/photos/{shot['id']}/thumb").content
    assert 0 < len(thumb) < client.get(f"/api/photos/{shot['id']}/img").num_bytes_downloaded + 10**6
    assert "attachment" in client.get(f"/api/photos/{shot['id']}/img?download=1").headers["content-disposition"]
    stats = client.get("/api/status").json()["storage"]
    assert stats["photos"] == 1 and stats["photo_bytes"] > 0
    assert client.delete(f"/api/photos/{shot['id']}", headers=H).status_code == 200
    assert client.get(f"/api/photos/{shot['id']}/img").status_code == 404
    assert not list((cfg.data_dir / "photos").rglob("*.jpg"))


def test_photo_bulk_delete_and_limit(env, monkeypatch):
    from camd import photos

    cfg, db, app, client = login(env)
    publish(app)
    ids = [client.post("/api/photos", headers=H).json()["id"] for _ in range(3)]
    assert client.post("/api/photos/delete", json={"ids": ids[:2] + [999]}, headers=H).json() == {"removed": 2}
    monkeypatch.setattr(photos, "MAX_PHOTOS", 1)
    assert client.post("/api/photos", headers=H).status_code == 409


def test_photo_endpoints_require_login(env):
    _, _, _, client = env
    for method, path in (("get", "/api/photos"), ("post", "/api/photos"), ("get", "/api/photos/1/img"), ("post", "/api/clips/start")):
        assert getattr(client, method)(path, headers=H).status_code == 401


def test_live_rejected_in_standby(env):
    cfg, db, app, client = login(env)
    app.state.runtime.grabber._standby = True
    assert client.get("/api/live").status_code == 503


def test_viewer_pool_evicts_oldest():
    from camd.viewers import ViewerPool

    pool = ViewerPool(2)
    first, second = pool.join(), pool.join()
    assert not first.is_set() and len(pool) == 2
    third = pool.join()
    assert first.is_set() and not second.is_set() and not third.is_set() and len(pool) == 2
    pool.leave(second)
    pool.leave(second)
    assert len(pool) == 1


def test_snapshot_refuses_stale_frame(env):
    cfg, db, app, client = login(env)
    app.state.runtime.grabber._publish(time.time() - 60, jpeg(scene()))
    assert client.get("/api/snapshot").status_code == 503


def test_events_table_migrates_old_schema(tmp_path):
    import sqlite3

    path = tmp_path / "old.sqlite3"
    conn = sqlite3.connect(path)
    conn.execute("CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, start_ts REAL NOT NULL, end_ts REAL, status TEXT NOT NULL, score REAL NOT NULL DEFAULT 0, clip_path TEXT, thumb_path TEXT, size_bytes INTEGER NOT NULL DEFAULT 0)")
    conn.execute("INSERT INTO events(start_ts, status) VALUES(1, 'done')")
    conn.commit()
    conn.close()
    db = Database(path)
    row = db.one("SELECT kind, keep FROM events")
    assert row["kind"] == "motion" and row["keep"] == 0
