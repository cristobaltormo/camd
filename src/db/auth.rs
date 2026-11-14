use rusqlite::{OptionalExtension, Row, params};

use super::Database;
use crate::error::Result;

#[derive(Clone, Debug, PartialEq)]
pub struct CredentialRow {
    pub id: String,
    pub public_key: Vec<u8>,
    pub sign_count: i64,
    pub name: String,
    pub created: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionRow {
    pub id: i64,
    pub created: f64,
    pub expires: f64,
    pub last_seen: f64,
    pub user_agent: String,
}

impl CredentialRow {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<CredentialRow> {
        Ok(CredentialRow {
            id: row.get(0)?,
            public_key: row.get(1)?,
            sign_count: row.get(2)?,
            name: row.get(3)?,
            created: row.get(4)?,
        })
    }
}

impl SessionRow {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<SessionRow> {
        Ok(SessionRow {
            id: row.get(0)?,
            created: row.get(1)?,
            expires: row.get(2)?,
            last_seen: row.get(3)?,
            user_agent: row.get(4)?,
        })
    }
}

impl Database {
    pub fn has_credentials(&self) -> Result<bool> {
        self.with(|c| {
            c.query_row("SELECT 1 FROM credentials LIMIT 1", [], |_| Ok(()))
                .optional()
        })
        .map(|found| found.is_some())
    }

    pub fn credential_ids(&self) -> Result<Vec<String>> {
        self.with(|c| {
            c.prepare("SELECT id FROM credentials")?
                .query_map([], |r| r.get(0))?
                .collect()
        })
    }

    pub fn credential(&self, id: &str) -> Result<Option<CredentialRow>> {
        self.with(|c| {
            c.query_row(
                "SELECT id, public_key, sign_count, name, created FROM credentials WHERE id=?",
                [id],
                CredentialRow::from_row,
            )
            .optional()
        })
    }

    pub fn list_credentials(&self) -> Result<Vec<CredentialRow>> {
        self.with(|c| {
            c.prepare("SELECT id, public_key, sign_count, name, created FROM credentials ORDER BY created")?
                .query_map([], CredentialRow::from_row)?
                .collect()
        })
    }

    pub fn insert_credential(&self, row: &CredentialRow) -> Result<()> {
        self.execute(
            "INSERT INTO credentials(id, public_key, sign_count, name, created) VALUES(?, ?, ?, ?, ?)",
            params![row.id, row.public_key, row.sign_count, row.name, row.created],
        )?;
        Ok(())
    }

    pub fn update_sign_count(&self, id: &str, sign_count: i64) -> Result<()> {
        self.execute(
            "UPDATE credentials SET sign_count=? WHERE id=?",
            params![sign_count, id],
        )?;
        Ok(())
    }

    pub fn delete_credential(&self, id: &str) -> Result<()> {
        self.execute("DELETE FROM credentials WHERE id=?", [id])?;
        Ok(())
    }

    pub fn insert_session(
        &self,
        token_hash: &str,
        created: f64,
        expires: f64,
        user_agent: &str,
    ) -> Result<i64> {
        self.insert(
            "INSERT INTO sessions(token_hash, created, expires, last_seen, user_agent) VALUES(?, ?, ?, ?, ?)",
            params![token_hash, created, expires, created, user_agent],
        )
    }

    pub fn session_by_hash(&self, token_hash: &str) -> Result<Option<SessionRow>> {
        self.with(|c| {
            c.query_row(
                "SELECT id, created, expires, last_seen, user_agent FROM sessions WHERE token_hash=?",
                [token_hash],
                SessionRow::from_row,
            )
            .optional()
        })
    }

    pub fn touch_session(&self, id: i64, now: f64) -> Result<()> {
        self.execute(
            "UPDATE sessions SET last_seen=? WHERE id=?",
            params![now, id],
        )?;
        Ok(())
    }

    pub fn delete_session(&self, id: i64) -> Result<()> {
        self.execute("DELETE FROM sessions WHERE id=?", [id])?;
        Ok(())
    }

    pub fn delete_all_sessions(&self) -> Result<()> {
        self.execute("DELETE FROM sessions", [])?;
        Ok(())
    }

    pub fn active_sessions(&self, now: f64) -> Result<Vec<SessionRow>> {
        self.with(|c| {
            c.prepare("SELECT id, created, expires, last_seen, user_agent FROM sessions WHERE expires > ? ORDER BY last_seen DESC")?
                .query_map([now], SessionRow::from_row)?
                .collect()
        })
    }

    pub fn delete_expired_enroll_codes(&self, now: f64) -> Result<()> {
        self.execute("DELETE FROM enroll_codes WHERE expires < ?", [now])?;
        Ok(())
    }

    pub fn insert_enroll_code(&self, code_hash: &str, expires: f64) -> Result<()> {
        self.execute(
            "INSERT INTO enroll_codes(code_hash, expires) VALUES(?, ?)",
            params![code_hash, expires],
        )?;
        Ok(())
    }

    pub fn enroll_code_expiry(&self, code_hash: &str) -> Result<Option<f64>> {
        self.with(|c| {
            c.query_row(
                "SELECT expires FROM enroll_codes WHERE code_hash=?",
                [code_hash],
                |r| r.get(0),
            )
            .optional()
        })
    }

    pub fn delete_enroll_code(&self, code_hash: &str) -> Result<()> {
        self.execute("DELETE FROM enroll_codes WHERE code_hash=?", [code_hash])?;
        Ok(())
    }
}
