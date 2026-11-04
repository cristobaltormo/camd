import sqlite3

from camd.db import MIGRATIONS, Database


def test_fresh_database_is_at_the_latest_version(tmp_path):
    db = Database(tmp_path / "new.sqlite3")
    assert db.one("PRAGMA user_version")[0] == len(MIGRATIONS)
    row = db.execute("INSERT INTO events(start_ts, status) VALUES(1, 'done')")
    assert db.one("SELECT kind, keep, labels, detections, analyzed, confirmed FROM events WHERE id=?", (row.lastrowid,))["labels"] == "[]"


def test_an_old_schema_is_migrated_without_losing_events(tmp_path):
    path = tmp_path / "old.sqlite3"
    conn = sqlite3.connect(path)
    conn.execute(
        "CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, start_ts REAL NOT NULL, end_ts REAL, status TEXT NOT NULL, "
        "score REAL NOT NULL DEFAULT 0, clip_path TEXT, thumb_path TEXT, size_bytes INTEGER NOT NULL DEFAULT 0)"
    )
    conn.execute("INSERT INTO events(start_ts, status) VALUES(1, 'done')")
    conn.commit()
    conn.close()
    row = Database(path).one("SELECT kind, keep, analyzed, confirmed, labels FROM events")
    assert (row["kind"], row["keep"], row["analyzed"], row["confirmed"], row["labels"]) == ("motion", 0, 0, 0, "[]")


def test_migrations_are_idempotent(tmp_path):
    path = tmp_path / "again.sqlite3"
    Database(path).close()
    assert Database(path).one("PRAGMA user_version")[0] == len(MIGRATIONS)


def test_settings_roundtrip(tmp_path):
    db = Database(tmp_path / "s.sqlite3")
    assert db.get_setting("missing", 5) == 5
    db.set_setting("k", {"a": [1, 2]})
    db.set_setting("k", {"a": [3]})
    assert db.get_setting("k") == {"a": [3]}
