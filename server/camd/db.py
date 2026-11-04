import json
import sqlite3
import threading
from collections.abc import Callable
from pathlib import Path
from typing import Any

BASELINE = """
CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    start_ts REAL NOT NULL,
    end_ts REAL,
    status TEXT NOT NULL,
    score REAL NOT NULL DEFAULT 0,
    clip_path TEXT,
    thumb_path TEXT,
    size_bytes INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS events_start ON events(start_ts);
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS credentials (
    id TEXT PRIMARY KEY,
    public_key BLOB NOT NULL,
    sign_count INTEGER NOT NULL,
    name TEXT NOT NULL,
    created REAL NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    token_hash TEXT NOT NULL UNIQUE,
    created REAL NOT NULL,
    expires REAL NOT NULL,
    last_seen REAL NOT NULL,
    user_agent TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS enroll_codes (
    code_hash TEXT PRIMARY KEY,
    expires REAL NOT NULL
);
CREATE TABLE IF NOT EXISTS push_subs (
    endpoint TEXT PRIMARY KEY,
    data TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS photos (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts REAL NOT NULL,
    path TEXT NOT NULL,
    thumb_path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS photos_ts ON photos(ts);
"""


def add_missing_columns(conn: sqlite3.Connection, table: str, columns: dict[str, str]) -> None:
    present = {row[1] for row in conn.execute(f"PRAGMA table_info({table})")}
    for name, ddl in columns.items():
        if name not in present:
            conn.execute(f"ALTER TABLE {table} ADD COLUMN {name} {ddl}")


def baseline(conn: sqlite3.Connection) -> None:
    conn.executescript(BASELINE)
    add_missing_columns(
        conn,
        "events",
        {"kind": "TEXT NOT NULL DEFAULT 'motion'", "keep": "INTEGER NOT NULL DEFAULT 0"},
    )


def object_detection(conn: sqlite3.Connection) -> None:
    add_missing_columns(
        conn,
        "events",
        {
            "labels": "TEXT NOT NULL DEFAULT '[]'",
            "detections": "TEXT NOT NULL DEFAULT '[]'",
            "analyzed": "INTEGER NOT NULL DEFAULT 0",
            "confirmed": "INTEGER NOT NULL DEFAULT 0",
        },
    )
    conn.execute("CREATE INDEX IF NOT EXISTS events_unconfirmed ON events(analyzed, confirmed, start_ts)")


MIGRATIONS: tuple[Callable[[sqlite3.Connection], None], ...] = (baseline, object_detection)


class Database:
    def __init__(self, path: Path):
        path.parent.mkdir(parents=True, exist_ok=True)
        self._lock = threading.RLock()
        self._conn = sqlite3.connect(str(path), check_same_thread=False)
        self._conn.row_factory = sqlite3.Row
        self._conn.execute("PRAGMA journal_mode=WAL")
        self._conn.execute("PRAGMA synchronous=NORMAL")
        self._migrate()

    def _migrate(self) -> None:
        current = self._conn.execute("PRAGMA user_version").fetchone()[0]
        for version, step in enumerate(MIGRATIONS[current:], start=current + 1):
            step(self._conn)
            self._conn.execute(f"PRAGMA user_version = {version}")
        self._conn.commit()

    def execute(self, sql: str, params: tuple = ()) -> sqlite3.Cursor:
        with self._lock:
            cursor = self._conn.execute(sql, params)
            self._conn.commit()
            return cursor

    def query(self, sql: str, params: tuple = ()) -> list[sqlite3.Row]:
        with self._lock:
            return self._conn.execute(sql, params).fetchall()

    def one(self, sql: str, params: tuple = ()) -> sqlite3.Row | None:
        with self._lock:
            return self._conn.execute(sql, params).fetchone()

    def get_setting(self, key: str, default: Any = None) -> Any:
        row = self.one("SELECT value FROM settings WHERE key=?", (key,))
        return json.loads(row["value"]) if row else default

    def set_setting(self, key: str, value: Any) -> None:
        self.execute(
            "INSERT INTO settings(key, value) VALUES(?, ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            (key, json.dumps(value)),
        )

    def close(self) -> None:
        with self._lock:
            self._conn.close()
