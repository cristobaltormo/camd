use rusqlite::params;

use super::Database;
use crate::error::Result;

impl Database {
    pub fn push_count(&self) -> Result<i64> {
        self.with(|c| c.query_row("SELECT COUNT(*) FROM push_subs", [], |r| r.get(0)))
    }

    pub fn upsert_push_sub(&self, endpoint: &str, data: &str) -> Result<()> {
        self.execute(
            "INSERT INTO push_subs(endpoint, data) VALUES(?, ?) ON CONFLICT(endpoint) DO UPDATE SET data=excluded.data",
            params![endpoint, data],
        )?;
        Ok(())
    }

    pub fn delete_push_sub(&self, endpoint: &str) -> Result<()> {
        self.execute("DELETE FROM push_subs WHERE endpoint=?", [endpoint])?;
        Ok(())
    }

    pub fn push_subs(&self) -> Result<Vec<(String, String)>> {
        self.with(|c| {
            c.prepare("SELECT endpoint, data FROM push_subs")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscriptions_are_upserted_by_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("p.sqlite3")).unwrap();
        db.upsert_push_sub("https://a/1", "{\"endpoint\": \"https://a/1\"}")
            .unwrap();
        db.upsert_push_sub(
            "https://a/1",
            "{\"endpoint\": \"https://a/1\", \"keys\": {}}",
        )
        .unwrap();
        db.upsert_push_sub("https://a/2", "{}").unwrap();
        assert_eq!(db.push_count().unwrap(), 2);
        assert!(
            db.push_subs()
                .unwrap()
                .iter()
                .any(|(e, d)| e == "https://a/1" && d.contains("keys"))
        );
        db.delete_push_sub("https://a/1").unwrap();
        assert_eq!(db.push_count().unwrap(), 1);
    }
}
