use rusqlite::{OptionalExtension, Row, params};
use serde_json::{Value, json};

use super::Database;
use crate::error::Result;

#[derive(Clone, Debug, PartialEq)]
pub struct PhotoRow {
    pub id: i64,
    pub ts: f64,
    pub path: String,
    pub thumb_path: String,
    pub size_bytes: i64,
}

impl PhotoRow {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<PhotoRow> {
        Ok(PhotoRow {
            id: row.get(0)?,
            ts: row.get(1)?,
            path: row.get(2)?,
            thumb_path: row.get(3)?,
            size_bytes: row.get(4)?,
        })
    }

    pub fn to_json(&self) -> Value {
        json!({"id": self.id, "ts": self.ts, "size": self.size_bytes})
    }
}

impl Database {
    pub fn count_photos(&self) -> Result<i64> {
        self.with(|c| c.query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0)))
    }

    pub fn photos_usage(&self) -> Result<(i64, i64)> {
        self.with(|c| {
            c.query_row(
                "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0) FROM photos",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
    }

    pub fn insert_photo(&self, ts: f64) -> Result<i64> {
        self.insert(
            "INSERT INTO photos(ts, path, thumb_path) VALUES(?, '', '')",
            [ts],
        )
    }

    pub fn set_photo_files(
        &self,
        id: i64,
        path: &str,
        thumb_path: &str,
        size_bytes: i64,
    ) -> Result<()> {
        self.execute(
            "UPDATE photos SET path=?, thumb_path=?, size_bytes=? WHERE id=?",
            params![path, thumb_path, size_bytes, id],
        )?;
        Ok(())
    }

    pub fn delete_photo_row(&self, id: i64) -> Result<()> {
        self.execute("DELETE FROM photos WHERE id=?", [id])?;
        Ok(())
    }

    pub fn photo(&self, id: i64) -> Result<Option<PhotoRow>> {
        self.with(|c| {
            c.query_row(
                "SELECT id, ts, path, thumb_path, size_bytes FROM photos WHERE id=?",
                [id],
                PhotoRow::from_row,
            )
            .optional()
        })
    }

    pub fn list_photos(&self, limit: i64, before: Option<f64>) -> Result<Vec<PhotoRow>> {
        self.with(|c| match before {
            None => c
                .prepare("SELECT id, ts, path, thumb_path, size_bytes FROM photos ORDER BY ts DESC LIMIT ?")?
                .query_map([limit], PhotoRow::from_row)?
                .collect(),
            Some(before) => c
                .prepare("SELECT id, ts, path, thumb_path, size_bytes FROM photos WHERE ts < ? ORDER BY ts DESC LIMIT ?")?
                .query_map(params![before, limit], PhotoRow::from_row)?
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photo_rows_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("p.sqlite3")).unwrap();
        let first = db.insert_photo(10.0).unwrap();
        let second = db.insert_photo(20.0).unwrap();
        db.set_photo_files(first, "/p/1.jpg", "/p/1_t.jpg", 100)
            .unwrap();
        db.set_photo_files(second, "/p/2.jpg", "/p/2_t.jpg", 50)
            .unwrap();
        assert_eq!(db.count_photos().unwrap(), 2);
        assert_eq!(db.photos_usage().unwrap(), (2, 150));
        assert_eq!(
            db.list_photos(60, None)
                .unwrap()
                .iter()
                .map(|p| p.id)
                .collect::<Vec<_>>(),
            vec![second, first]
        );
        assert_eq!(
            db.list_photos(60, Some(15.0))
                .unwrap()
                .iter()
                .map(|p| p.id)
                .collect::<Vec<_>>(),
            vec![first]
        );
        assert_eq!(
            db.photo(first).unwrap().unwrap().to_json(),
            json!({"id": first, "ts": 10.0, "size": 100})
        );
        db.delete_photo_row(first).unwrap();
        assert!(db.photo(first).unwrap().is_none());
    }
}
