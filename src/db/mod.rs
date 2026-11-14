mod auth;
mod events;
mod photos;
mod push;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::{Error, Result};

pub use auth::{CredentialRow, SessionRow};
pub use events::{EventDone, EventFilter, EventRow, FINISHED, VISIBLE};
pub use photos::PhotoRow;

pub const BASELINE: &str = "
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
";

type Migration = fn(&Connection) -> rusqlite::Result<()>;

pub const MIGRATIONS: [Migration; 2] = [baseline, object_detection];

pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

fn add_missing_columns(
    conn: &Connection,
    table: &str,
    columns: &[(&str, &str)],
) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let present: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<_>>()?;
    for (name, ddl) in columns {
        if !present.iter().any(|p| p == name) {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {name} {ddl}"))?;
        }
    }
    Ok(())
}

fn baseline(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(BASELINE)?;
    add_missing_columns(
        conn,
        "events",
        &[
            ("kind", "TEXT NOT NULL DEFAULT 'motion'"),
            ("keep", "INTEGER NOT NULL DEFAULT 0"),
        ],
    )
}

fn object_detection(conn: &Connection) -> rusqlite::Result<()> {
    add_missing_columns(
        conn,
        "events",
        &[
            ("labels", "TEXT NOT NULL DEFAULT '[]'"),
            ("detections", "TEXT NOT NULL DEFAULT '[]'"),
            ("analyzed", "INTEGER NOT NULL DEFAULT 0"),
            ("confirmed", "INTEGER NOT NULL DEFAULT 0"),
        ],
    )?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS events_unconfirmed ON events(analyzed, confirmed, start_ts)",
    )
}

pub struct Database {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl Database {
    pub fn open(path: &Path) -> Result<Database> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrate(&conn)?;
        Ok(Database {
            conn: Mutex::new(conn),
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T> {
        Ok(f(&self.conn())?)
    }

    pub fn execute(&self, sql: &str, params: impl rusqlite::Params) -> Result<usize> {
        Ok(self.conn().execute(sql, params)?)
    }

    pub fn insert(&self, sql: &str, params: impl rusqlite::Params) -> Result<i64> {
        let conn = self.conn();
        conn.execute(sql, params)?;
        Ok(conn.last_insert_rowid())
    }

    pub fn user_version(&self) -> Result<i64> {
        self.with(|c| c.query_row("PRAGMA user_version", [], |r| r.get(0)))
    }

    pub fn get_setting_value(&self, key: &str) -> Result<Option<Value>> {
        let raw: Option<String> = self.with(|c| {
            c.query_row("SELECT value FROM settings WHERE key=?", [key], |r| {
                r.get(0)
            })
            .optional()
        })?;
        match raw {
            None => Ok(None),
            Some(text) => Ok(Some(serde_json::from_str(&text)?)),
        }
    }

    pub fn get_setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        match self.get_setting_value(key)? {
            None => Ok(None),
            Some(value) => serde_json::from_value(value)
                .map(Some)
                .map_err(|e| Error::internal(format!("setting {key}: {e}"))),
        }
    }

    pub fn get_setting_or<T: DeserializeOwned>(&self, key: &str, default: T) -> T {
        self.get_setting(key).ok().flatten().unwrap_or(default)
    }

    pub fn set_setting<T: Serialize + ?Sized>(&self, key: &str, value: &T) -> Result<()> {
        let text = serde_json::to_string(value)?;
        self.execute(
            "INSERT INTO settings(key, value) VALUES(?, ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, text],
        )?;
        Ok(())
    }
}

fn migrate(conn: &Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (index, step) in MIGRATIONS.iter().enumerate().skip(current.max(0) as usize) {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let applied = step(conn)
            .and_then(|_| conn.execute_batch(&format!("PRAGMA user_version = {}", index + 1)));
        match applied {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e.into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fresh_database_is_at_the_latest_version() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("new.sqlite3")).unwrap();
        assert_eq!(db.user_version().unwrap(), SCHEMA_VERSION);
        let id = db
            .insert("INSERT INTO events(start_ts, status) VALUES(1, 'done')", [])
            .unwrap();
        let row = db.event(id).unwrap().unwrap();
        assert_eq!(
            (
                row.kind.as_str(),
                row.keep,
                row.labels.as_str(),
                row.detections.as_str()
            ),
            ("motion", false, "[]", "[]")
        );
        assert!(!row.analyzed && !row.confirmed);
    }

    #[test]
    fn an_old_schema_is_migrated_without_losing_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.sqlite3");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, start_ts REAL NOT NULL, end_ts REAL, status TEXT NOT NULL, \
                 score REAL NOT NULL DEFAULT 0, clip_path TEXT, thumb_path TEXT, size_bytes INTEGER NOT NULL DEFAULT 0);
                 INSERT INTO events(start_ts, status) VALUES(1, 'done');",
            )
            .unwrap();
        }
        let db = Database::open(&path).unwrap();
        let row = db.event(1).unwrap().unwrap();
        assert_eq!(
            (
                row.kind.as_str(),
                row.keep,
                row.analyzed,
                row.confirmed,
                row.labels.as_str()
            ),
            ("motion", false, false, false, "[]")
        );
        assert_eq!(db.user_version().unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn a_version_one_database_only_runs_the_second_step() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v1.sqlite3");
        {
            let conn = Connection::open(&path).unwrap();
            baseline(&conn).unwrap();
            conn.execute_batch("PRAGMA user_version = 1; INSERT INTO events(start_ts, status, kind) VALUES(5, 'done', 'manual');")
                .unwrap();
        }
        let db = Database::open(&path).unwrap();
        let row = db.event(1).unwrap().unwrap();
        assert_eq!((row.kind.as_str(), row.labels.as_str()), ("manual", "[]"));
        let indexes: i64 = db
            .with(|c| {
                c.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name='events_unconfirmed'",
                    [],
                    |r| r.get(0),
                )
            })
            .unwrap();
        assert_eq!(indexes, 1);
    }

    #[test]
    fn migrations_are_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("again.sqlite3");
        drop(Database::open(&path).unwrap());
        assert_eq!(
            Database::open(&path).unwrap().user_version().unwrap(),
            SCHEMA_VERSION
        );
    }

    #[test]
    fn the_journal_is_wal() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("w.sqlite3")).unwrap();
        let mode: String = db
            .with(|c| c.query_row("PRAGMA journal_mode", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(mode, "wal");
    }

    #[test]
    fn settings_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("s.sqlite3")).unwrap();
        assert_eq!(db.get_setting_or("missing", 5), 5);
        db.set_setting("k", &json!({"a": [1, 2]})).unwrap();
        db.set_setting("k", &json!({"a": [3]})).unwrap();
        assert_eq!(db.get_setting_value("k").unwrap(), Some(json!({"a": [3]})));
        assert_eq!(db.get_setting::<String>("missing").unwrap(), None);
    }

    #[test]
    fn settings_written_by_python_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("p.sqlite3")).unwrap();
        db.execute(
            "INSERT INTO settings(key, value) VALUES('prefs', '{\"post_roll_s\": 25.0, \"notify_motion\": false}')",
            [],
        )
        .unwrap();
        assert_eq!(
            db.get_setting_value("prefs").unwrap(),
            Some(json!({"post_roll_s": 25.0, "notify_motion": false}))
        );
    }
}
