use super::{
    Auth, ENROLL_TTL, MAX_USER_AGENT, SEEN_REFRESH_S, SESSION_TTL, SessionInfo, sha256_hex,
    token_urlsafe, truncate_chars,
};
use crate::clock;
use crate::error::{Error, Result};

impl Auth {
    pub fn create_session(&self, user_agent: &str) -> Result<String> {
        let token = token_urlsafe(32);
        let now = clock::now();
        self.db().insert_session(
            &sha256_hex(&token),
            now,
            now + SESSION_TTL,
            &truncate_chars(user_agent, MAX_USER_AGENT),
        )?;
        Ok(token)
    }

    pub fn check_session(&self, token: Option<&str>) -> Result<Option<i64>> {
        let Some(token) = token.filter(|t| !t.is_empty()) else {
            return Ok(None);
        };
        let Some(row) = self.db().session_by_hash(&sha256_hex(token))? else {
            return Ok(None);
        };
        let now = clock::now();
        if row.expires < now {
            return Ok(None);
        }
        if now - row.last_seen > SEEN_REFRESH_S {
            self.db().touch_session(row.id, now)?;
        }
        Ok(Some(row.id))
    }

    pub fn revoke(&self, session_id: i64) -> Result<()> {
        self.db().delete_session(session_id)
    }

    pub fn revoke_all(&self) -> Result<()> {
        self.db().delete_all_sessions()
    }

    pub fn sessions(&self) -> Result<Vec<SessionInfo>> {
        Ok(self
            .db()
            .active_sessions(clock::now())?
            .into_iter()
            .map(|s| SessionInfo {
                id: s.id,
                created: s.created,
                last_seen: s.last_seen,
                user_agent: s.user_agent,
            })
            .collect())
    }

    pub fn new_enroll_code(&self) -> Result<String> {
        let code = hex::encode(super::random_bytes::<4>());
        let now = clock::now();
        self.db().delete_expired_enroll_codes(now)?;
        self.db()
            .insert_enroll_code(&sha256_hex(&code), now + ENROLL_TTL)?;
        Ok(code)
    }

    pub fn check_code(&self, code: &str) -> Result<()> {
        match self.db().enroll_code_expiry(&sha256_hex(code))? {
            Some(expires) if expires >= clock::now() => Ok(()),
            _ => Err(Error::invalid("invalid or expired code")),
        }
    }

    pub fn consume_code(&self, code: &str) -> Result<()> {
        self.db().delete_enroll_code(&sha256_hex(code))
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::auth;

    #[test]
    fn sessions_are_created_checked_listed_and_revoked() {
        let (_d, auth) = auth();
        let first = auth.create_session(&"x".repeat(300)).unwrap();
        let second = auth.create_session("phone").unwrap();
        let first_id = auth.check_session(Some(&first)).unwrap().unwrap();
        assert!(auth.check_session(Some("nope")).unwrap().is_none());
        assert!(auth.check_session(Some("")).unwrap().is_none());
        assert!(auth.check_session(None).unwrap().is_none());
        let listed = auth.sessions().unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().any(|s| s.user_agent.len() == 200));
        auth.revoke(first_id).unwrap();
        assert!(auth.check_session(Some(&first)).unwrap().is_none());
        assert!(auth.check_session(Some(&second)).unwrap().is_some());
        auth.revoke_all().unwrap();
        assert!(auth.check_session(Some(&second)).unwrap().is_none());
    }

    #[test]
    fn expired_sessions_are_refused_and_stale_ones_refreshed() {
        let (_d, auth) = auth();
        let token = auth.create_session("ua").unwrap();
        let id = auth.check_session(Some(&token)).unwrap().unwrap();
        auth.db()
            .execute("UPDATE sessions SET last_seen=0 WHERE id=?", [id])
            .unwrap();
        auth.check_session(Some(&token)).unwrap();
        assert!(auth.sessions().unwrap()[0].last_seen > 1.0);
        auth.db()
            .execute("UPDATE sessions SET expires=1 WHERE id=?", [id])
            .unwrap();
        assert!(auth.check_session(Some(&token)).unwrap().is_none());
        assert!(auth.sessions().unwrap().is_empty());
    }

    #[test]
    fn enroll_codes_are_hex_hashed_and_expire() {
        let (_d, auth) = auth();
        let code = auth.new_enroll_code().unwrap();
        assert_eq!(code.len(), 8);
        assert!(code.chars().all(|c| c.is_ascii_hexdigit()));
        auth.check_code(&code).unwrap();
        assert_eq!(
            auth.check_code("deadbeef").unwrap_err().message(),
            "invalid or expired code"
        );
        auth.consume_code(&code).unwrap();
        assert!(auth.check_code(&code).is_err());
        let late = auth.new_enroll_code().unwrap();
        auth.db()
            .execute("UPDATE enroll_codes SET expires=1", [])
            .unwrap();
        assert!(auth.check_code(&late).is_err());
        auth.new_enroll_code().unwrap();
        let left: i64 = auth
            .db()
            .with(|c| c.query_row("SELECT COUNT(*) FROM enroll_codes", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(left, 1);
    }
}
