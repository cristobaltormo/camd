use rusqlite::{OptionalExtension, Row, params, params_from_iter};
use serde_json::{Value, json};

use super::Database;
use crate::error::Result;

pub const FINISHED: &str = "status != 'recording'";
pub const VISIBLE: &str = "(analyzed=0 OR confirmed=1)";
const COLUMNS: &str = "id, start_ts, end_ts, status, score, clip_path, thumb_path, size_bytes, kind, keep, labels, detections, analyzed, confirmed";

#[derive(Clone, Debug, PartialEq)]
pub struct EventRow {
    pub id: i64,
    pub start_ts: f64,
    pub end_ts: Option<f64>,
    pub status: String,
    pub score: f64,
    pub clip_path: Option<String>,
    pub thumb_path: Option<String>,
    pub size_bytes: i64,
    pub kind: String,
    pub keep: bool,
    pub labels: String,
    pub detections: String,
    pub analyzed: bool,
    pub confirmed: bool,
}

impl EventRow {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<EventRow> {
        Ok(EventRow {
            id: row.get(0)?,
            start_ts: row.get(1)?,
            end_ts: row.get(2)?,
            status: row.get(3)?,
            score: row.get(4)?,
            clip_path: row.get(5)?,
            thumb_path: row.get(6)?,
            size_bytes: row.get(7)?,
            kind: row.get(8)?,
            keep: row.get::<_, i64>(9)? != 0,
            labels: row.get(10)?,
            detections: row.get(11)?,
            analyzed: row.get::<_, i64>(12)? != 0,
            confirmed: row.get::<_, i64>(13)? != 0,
        })
    }

    pub fn label_list(&self) -> Vec<String> {
        serde_json::from_str(&self.labels).unwrap_or_default()
    }

    pub fn to_json(&self) -> Value {
        let end = self.end_ts.filter(|e| *e != 0.0);
        json!({
            "id": self.id,
            "start": self.start_ts,
            "end": self.end_ts,
            "duration": end.map(|e| e - self.start_ts),
            "status": self.status,
            "score": self.score,
            "size": self.size_bytes,
            "kind": self.kind,
            "keep": self.keep,
            "analyzed": self.analyzed,
            "labels": serde_json::from_str::<Value>(&self.labels).unwrap_or_else(|_| json!([])),
            "detections": serde_json::from_str::<Value>(&self.detections).unwrap_or_else(|_| json!([])),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EventFilter {
    pub limit: i64,
    pub before: Option<f64>,
    pub after: Option<f64>,
    pub label: Option<String>,
    pub hide_unconfirmed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EventDone {
    pub end_ts: f64,
    pub score: f64,
    pub clip_path: String,
    pub thumb_path: String,
    pub size_bytes: i64,
    pub labels: Vec<String>,
    pub detections: Value,
}

impl Database {
    fn events_where(
        &self,
        clause: &str,
        values: Vec<rusqlite::types::Value>,
    ) -> Result<Vec<EventRow>> {
        self.with(|c| {
            let mut stmt = c.prepare(&format!("SELECT {COLUMNS} FROM events {clause}"))?;
            stmt.query_map(params_from_iter(values), EventRow::from_row)?
                .collect()
        })
    }

    fn event_where(
        &self,
        clause: &str,
        values: Vec<rusqlite::types::Value>,
    ) -> Result<Option<EventRow>> {
        self.with(|c| {
            c.query_row(
                &format!("SELECT {COLUMNS} FROM events {clause}"),
                params_from_iter(values),
                EventRow::from_row,
            )
            .optional()
        })
    }

    pub fn event(&self, id: i64) -> Result<Option<EventRow>> {
        self.event_where("WHERE id=?", vec![id.into()])
    }

    pub fn finished_event(&self, id: i64) -> Result<Option<EventRow>> {
        self.event_where(&format!("WHERE id=? AND {FINISHED}"), vec![id.into()])
    }

    pub fn list_events(&self, filter: &EventFilter) -> Result<Vec<EventRow>> {
        let mut clauses: Vec<String> = Vec::new();
        let mut values: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(before) = filter.before {
            clauses.push("start_ts < ?".into());
            values.push(before.into());
        }
        if let Some(after) = filter.after {
            clauses.push("start_ts >= ?".into());
            values.push(after.into());
        }
        if let Some(label) = filter.label.as_deref().filter(|l| !l.is_empty()) {
            clauses.push("labels LIKE ?".into());
            values.push(format!("%\"{}\"%", label.replace('"', "")).into());
        }
        if filter.hide_unconfirmed {
            clauses.push(VISIBLE.into());
        }
        let clause = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        values.push(filter.limit.clamp(1, 200).into());
        self.events_where(&format!("{clause} ORDER BY start_ts DESC LIMIT ?"), values)
    }

    pub fn count_visible_events_since(&self, since: f64) -> Result<i64> {
        self.with(|c| {
            c.query_row(
                &format!("SELECT COUNT(*) FROM events WHERE start_ts > ? AND {VISIBLE}"),
                [since],
                |r| r.get(0),
            )
        })
    }

    pub fn last_visible_event(&self) -> Result<Option<EventRow>> {
        self.event_where(
            "WHERE status='done' AND (analyzed=0 OR confirmed=1) ORDER BY start_ts DESC LIMIT 1",
            vec![],
        )
    }

    pub fn events_usage(&self) -> Result<(i64, i64)> {
        self.with(|c| {
            c.query_row(
                "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0) FROM events",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
    }

    pub fn events_total_bytes(&self) -> Result<i64> {
        Ok(self.events_usage()?.1)
    }

    pub fn set_event_keep(&self, id: i64, keep: bool) -> Result<()> {
        self.execute(
            "UPDATE events SET keep=? WHERE id=?",
            params![keep as i64, id],
        )?;
        Ok(())
    }

    pub fn delete_event_row(&self, id: i64) -> Result<()> {
        self.execute("DELETE FROM events WHERE id=?", [id])?;
        Ok(())
    }

    pub fn insert_event(
        &self,
        start_ts: f64,
        kind: &str,
        keep: bool,
        analyzed: bool,
    ) -> Result<i64> {
        self.insert(
            "INSERT INTO events(start_ts, status, kind, keep, analyzed) VALUES(?, 'recording', ?, ?, ?)",
            params![start_ts, kind, keep as i64, analyzed as i64],
        )
    }

    pub fn mark_event_manual(&self, id: i64) -> Result<()> {
        self.execute("UPDATE events SET kind='manual', keep=1 WHERE id=?", [id])?;
        Ok(())
    }

    pub fn set_event_labels(&self, id: i64, labels: &[String]) -> Result<()> {
        self.execute(
            "UPDATE events SET labels=?, confirmed=1 WHERE id=?",
            params![serde_json::to_string(labels)?, id],
        )?;
        Ok(())
    }

    pub fn finish_event(&self, id: i64, done: &EventDone) -> Result<()> {
        self.execute(
            "UPDATE events SET end_ts=?, status='done', score=?, clip_path=?, thumb_path=?, size_bytes=?, labels=?, detections=?, confirmed=? WHERE id=?",
            params![
                done.end_ts,
                done.score,
                done.clip_path,
                done.thumb_path,
                done.size_bytes,
                serde_json::to_string(&done.labels)?,
                serde_json::to_string(&done.detections)?,
                (!done.labels.is_empty()) as i64,
                id
            ],
        )?;
        Ok(())
    }

    pub fn fail_event(&self, id: i64, end_ts: f64) -> Result<()> {
        self.execute(
            "UPDATE events SET status='failed', end_ts=? WHERE id=?",
            params![end_ts, id],
        )?;
        Ok(())
    }

    pub fn recording_events(&self) -> Result<Vec<EventRow>> {
        self.events_where("WHERE status='recording'", vec![])
    }

    pub fn expired_events(&self, started_before: f64) -> Result<Vec<EventRow>> {
        self.events_where(
            "WHERE start_ts < ? AND status != 'recording' AND keep=0",
            vec![started_before.into()],
        )
    }

    pub fn expired_unconfirmed_events(&self, started_before: f64) -> Result<Vec<EventRow>> {
        self.events_where(
            "WHERE analyzed=1 AND confirmed=0 AND kind='motion' AND status != 'recording' AND keep=0 AND start_ts < ?",
            vec![started_before.into()],
        )
    }

    pub fn oldest_removable_event(&self) -> Result<Option<EventRow>> {
        self.event_where(
            "WHERE status != 'recording' AND keep=0 ORDER BY start_ts LIMIT 1",
            vec![],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("e.sqlite3")).unwrap();
        (dir, db)
    }

    fn insert_done(db: &Database, start: f64, labels: &[&str], analyzed: bool) -> i64 {
        db.insert(
            "INSERT INTO events(start_ts, end_ts, status, size_bytes, labels, analyzed, confirmed) VALUES(?, ?, 'done', 1, ?, ?, ?)",
            params![
                start,
                start + 10.0,
                serde_json::to_string(labels).unwrap(),
                analyzed as i64,
                (!labels.is_empty()) as i64
            ],
        )
        .unwrap()
    }

    #[test]
    fn event_json_has_the_python_shape() {
        let (_d, db) = db();
        let id = insert_done(&db, 100.0, &[], false);
        let value = db.event(id).unwrap().unwrap().to_json();
        assert_eq!(
            value,
            json!({"id": id, "start": 100.0, "end": 110.0, "duration": 10.0, "status": "done", "score": 0.0, "size": 1,
                   "kind": "motion", "keep": false, "analyzed": false, "labels": [], "detections": []})
        );
        let recording = db.insert_event(5.0, "motion", false, true).unwrap();
        let value = db.event(recording).unwrap().unwrap().to_json();
        assert_eq!(
            (
                value["end"].clone(),
                value["duration"].clone(),
                value["analyzed"].clone()
            ),
            (Value::Null, Value::Null, json!(true))
        );
    }

    #[test]
    fn label_filter_hidden_noise_and_count() {
        let (_d, db) = db();
        let person = insert_done(&db, 300.0, &["person"], true);
        insert_done(&db, 200.0, &["cat"], true);
        let noise = insert_done(&db, 100.0, &[], true);
        let by_label = db
            .list_events(&EventFilter {
                limit: 50,
                label: Some("person".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            by_label.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![person]
        );
        let visible = db
            .list_events(&EventFilter {
                limit: 50,
                hide_unconfirmed: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(visible.len(), 2);
        assert!(!visible.iter().any(|e| e.id == noise));
        assert_eq!(db.count_visible_events_since(0.0).unwrap(), 2);
        let quoted = db
            .list_events(&EventFilter {
                limit: 50,
                label: Some("per\"son".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(quoted.len(), 1);
    }

    #[test]
    fn time_range_and_limit_are_applied() {
        let (_d, db) = db();
        let now = 1_000_000.0;
        for offset in [0.0, 3600.0, 7200.0] {
            insert_done(&db, now + offset, &[], false);
        }
        let got = db
            .list_events(&EventFilter {
                limit: 50,
                after: Some(now + 3000.0),
                before: Some(now + 7000.0),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            got.iter().map(|e| e.start_ts).collect::<Vec<_>>(),
            vec![now + 3600.0]
        );
        assert_eq!(
            db.list_events(&EventFilter {
                limit: 0,
                ..Default::default()
            })
            .unwrap()
            .len(),
            1
        );
        let all = db
            .list_events(&EventFilter {
                limit: 500,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            all.iter().map(|e| e.start_ts).collect::<Vec<_>>(),
            vec![now + 7200.0, now + 3600.0, now]
        );
    }

    #[test]
    fn recorder_lifecycle_queries() {
        let (_d, db) = db();
        let id = db.insert_event(10.0, "motion", false, false).unwrap();
        assert!(db.finished_event(id).unwrap().is_none());
        assert_eq!(db.recording_events().unwrap().len(), 1);
        db.set_event_labels(id, &["person".to_string(), "cat".to_string()])
            .unwrap();
        let row = db.event(id).unwrap().unwrap();
        assert!(row.confirmed);
        assert_eq!(row.label_list(), vec!["person", "cat"]);
        db.mark_event_manual(id).unwrap();
        db.finish_event(
            id,
            &EventDone {
                end_ts: 20.0,
                score: 0.25,
                clip_path: "/c/1.mp4".into(),
                thumb_path: "/c/1.jpg".into(),
                size_bytes: 1234,
                labels: vec![],
                detections: json!([{"label": "person", "confidence": 0.9, "box": [0.1, 0.2, 0.3, 0.4]}]),
            },
        )
        .unwrap();
        let row = db.finished_event(id).unwrap().unwrap();
        assert_eq!(
            (
                row.status.as_str(),
                row.kind.as_str(),
                row.keep,
                row.confirmed
            ),
            ("done", "manual", true, false)
        );
        assert_eq!(row.to_json()["detections"][0]["label"], "person");
        assert_eq!(db.events_usage().unwrap(), (1, 1234));
        assert_eq!(db.last_visible_event().unwrap().unwrap().id, id);
        db.fail_event(id, 30.0).unwrap();
        assert_eq!(db.event(id).unwrap().unwrap().status, "failed");
        db.set_event_keep(id, false).unwrap();
        assert_eq!(db.oldest_removable_event().unwrap().unwrap().id, id);
        db.delete_event_row(id).unwrap();
        assert!(db.event(id).unwrap().is_none());
    }

    #[test]
    fn the_last_event_card_skips_unfinished_and_unconfirmed_events() {
        let (_d, db) = db();
        db.execute("INSERT INTO events(start_ts, end_ts, status, analyzed, confirmed) VALUES(100, 110, 'done', 1, 1)", [])
            .unwrap();
        db.execute("INSERT INTO events(start_ts, end_ts, status, analyzed, confirmed) VALUES(200, 210, 'done', 1, 0)", [])
            .unwrap();
        db.execute(
            "INSERT INTO events(start_ts, status) VALUES(300, 'recording')",
            [],
        )
        .unwrap();
        assert_eq!(db.last_visible_event().unwrap().unwrap().start_ts, 100.0);
    }
}
