use std::collections::BTreeMap;
use std::path::Path;

use crate::clock;
use crate::config::Config;
use crate::db::{Database, EventRow};
use crate::error::Result;
use crate::warning;

pub const PURGE_INTERVAL_S: f64 = 3600.0;

pub fn unlink(path: Option<&str>) {
    let Some(path) = path.filter(|p| !p.is_empty()) else {
        return;
    };
    if let Err(e) = std::fs::remove_file(Path::new(path))
        && e.kind() != std::io::ErrorKind::NotFound
    {
        warning!("camd.storage", "could not delete {}", path);
    }
}

pub fn remove_event(db: &Database, row: &EventRow) -> Result<()> {
    unlink(row.clip_path.as_deref());
    unlink(row.thumb_path.as_deref());
    db.delete_event_row(row.id)
}

pub fn cap_bytes(cfg: &Config) -> i64 {
    (cfg.max_disk_gb * 1024f64.powi(3)) as i64
}

pub fn purge(cfg: &Config, db: &Database) -> Result<usize> {
    purge_at(cfg, db, clock::now())
}

pub fn purge_at(cfg: &Config, db: &Database, now: f64) -> Result<usize> {
    let mut rows = db.expired_events(now - cfg.retention_days as f64 * 86400.0)?;
    if cfg.detection != "off" {
        rows.extend(db.expired_unconfirmed_events(now - cfg.unconfirmed_keep_h * 3600.0)?);
    }
    let unique: BTreeMap<i64, EventRow> = rows.into_iter().map(|r| (r.id, r)).collect();
    let mut removed = 0;
    for row in unique.values() {
        remove_event(db, row)?;
        removed += 1;
    }
    let cap = cap_bytes(cfg);
    while db.events_total_bytes()? > cap {
        let Some(oldest) = db.oldest_removable_event()? else {
            break;
        };
        remove_event(db, &oldest)?;
        removed += 1;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn setup(cfg: impl FnOnce(&mut Config)) -> (tempfile::TempDir, Config, Database) {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::with_data_dir(dir.path());
        cfg(&mut config);
        let db = Database::open(&config.db_path()).unwrap();
        (dir, config, db)
    }

    #[test]
    fn purge_by_age_and_disk_cap() {
        let (dir, cfg, db) = setup(|c| {
            c.retention_days = 1;
            c.max_disk_gb = 0.000002;
        });
        let old = dir.path().join("old.mp4");
        std::fs::write(&old, b"x".repeat(10)).unwrap();
        let now = clock::now();
        db.execute(
            "INSERT INTO events(start_ts, status, clip_path, size_bytes) VALUES(?, 'done', ?, 10)",
            params![now - 3.0 * 86400.0, old.to_string_lossy()],
        )
        .unwrap();
        for i in 0..3 {
            db.execute(
                "INSERT INTO events(start_ts, status, size_bytes) VALUES(?, 'done', 1000)",
                [now - i as f64],
            )
            .unwrap();
        }
        assert!(purge(&cfg, &db).unwrap() >= 2);
        assert!(!old.exists());
        assert!(db.events_total_bytes().unwrap() as f64 <= cfg.max_disk_gb * 1024f64.powi(3));
    }

    #[test]
    fn unconfirmed_events_expire_sooner_but_only_when_analysed() {
        let (_dir, mut cfg, db) = setup(|c| {
            c.detection = "alert".into();
            c.unconfirmed_keep_h = 1.0;
        });
        let long_ago = clock::now() - 5.0 * 3600.0;
        for (analyzed, confirmed) in [(1, 0), (1, 1), (0, 0)] {
            db.execute(
                "INSERT INTO events(start_ts, status, analyzed, confirmed) VALUES(?, 'done', ?, ?)",
                params![long_ago, analyzed, confirmed],
            )
            .unwrap();
        }
        assert_eq!(purge(&cfg, &db).unwrap(), 1);
        assert_eq!(db.events_usage().unwrap().0, 2);
        cfg.detection = "off".into();
        db.execute("UPDATE events SET analyzed=1, confirmed=0", [])
            .unwrap();
        assert_eq!(purge(&cfg, &db).unwrap(), 0);
    }

    #[test]
    fn kept_recording_and_manual_events_survive() {
        let (_dir, cfg, db) = setup(|c| {
            c.retention_days = 1;
            c.max_disk_gb = 0.0;
        });
        db.execute(
            "INSERT INTO events(start_ts, status, keep, size_bytes) VALUES(1, 'done', 1, 50)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO events(start_ts, status, size_bytes) VALUES(1, 'recording', 50)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO events(start_ts, status, kind, analyzed, confirmed) VALUES(1, 'done', 'manual', 1, 0)",
            [],
        )
        .unwrap();
        db.execute("UPDATE events SET keep=1 WHERE kind='manual'", [])
            .unwrap();
        assert_eq!(purge(&cfg, &db).unwrap(), 0);
        assert_eq!(db.events_usage().unwrap().0, 3);
    }

    #[test]
    fn an_event_matched_by_both_rules_counts_once() {
        let (_dir, cfg, db) = setup(|c| {
            c.retention_days = 1;
            c.unconfirmed_keep_h = 1.0;
        });
        db.execute(
            "INSERT INTO events(start_ts, status, analyzed, confirmed) VALUES(1, 'done', 1, 0)",
            [],
        )
        .unwrap();
        assert_eq!(purge(&cfg, &db).unwrap(), 1);
    }

    #[test]
    fn unlink_ignores_missing_and_empty_paths() {
        unlink(None);
        unlink(Some(""));
        unlink(Some("/nonexistent/camd/file"));
    }
}
