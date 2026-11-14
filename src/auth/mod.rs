mod password;
mod ratelimit;
mod sessions;
pub mod webauthn;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::clock;
use crate::config::Config;
use crate::db::Database;
use crate::error::{Error, Result};

pub use password::{SALT_LEN, check_password, hash_password};
pub use ratelimit::RateLimiter;

pub const SESSION_COOKIE: &str = "cam_session";
pub const SESSION_TTL: f64 = 30.0 * 86400.0;
pub const ENROLL_TTL: f64 = 600.0;
pub const CHALLENGE_TTL: f64 = 300.0;
pub const SEEN_REFRESH_S: f64 = 60.0;
pub const USER_ID: &[u8] = b"owner";
pub const MIN_PASSWORD_LENGTH: usize = 10;
pub const RATE_LIMIT: usize = 10;
pub const RATE_WINDOW_S: f64 = 60.0;
pub const PASSWORD_CHECKS_AT_ONCE: usize = 2;
pub const PASSWORD_ATTEMPTS_PER_WINDOW: usize = 60;
pub const MAX_CHALLENGES: usize = 1024;
pub const DEFAULT_DEVICE_NAME: &str = "dispositivo";
pub const MAX_DEVICE_NAME: usize = 40;
pub const MAX_USER_AGENT: usize = 200;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CredentialInfo {
    pub id: String,
    pub name: String,
    pub created: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SessionInfo {
    pub id: i64,
    pub created: f64,
    pub last_seen: f64,
    pub user_agent: String,
}

pub fn sha256_hex(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).expect("system randomness");
    out
}

pub fn token_urlsafe(bytes: usize) -> String {
    use base64::Engine;
    let mut raw = vec![0u8; bytes];
    getrandom::fill(&mut raw).expect("system randomness");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
}

pub fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

pub struct PasswordSlot<'a>(&'a AtomicUsize);

impl Drop for PasswordSlot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct Auth {
    pub rp_id: String,
    pub origin: String,
    pub site_name: String,
    pub limiter: RateLimiter,
    password_limiter: RateLimiter,
    password_checks: AtomicUsize,
    db: Arc<Database>,
    challenges: Mutex<HashMap<String, (Vec<u8>, f64)>>,
}

impl Auth {
    pub fn new(cfg: &Config, db: Arc<Database>) -> Auth {
        Auth {
            rp_id: cfg.rp_id.clone(),
            origin: cfg.origin.clone(),
            site_name: cfg.site_name.clone(),
            limiter: RateLimiter::new(RATE_LIMIT, RATE_WINDOW_S),
            password_limiter: RateLimiter::new(PASSWORD_ATTEMPTS_PER_WINDOW, RATE_WINDOW_S),
            password_checks: AtomicUsize::new(0),
            db,
            challenges: Mutex::new(HashMap::new()),
        }
    }

    pub fn db(&self) -> &Arc<Database> {
        &self.db
    }

    pub fn password_slot(&self) -> Option<PasswordSlot<'_>> {
        if !self.password_limiter.allow("*") {
            return None;
        }
        let taken = self.password_checks.fetch_add(1, Ordering::SeqCst);
        let slot = PasswordSlot(&self.password_checks);
        (taken < PASSWORD_CHECKS_AT_ONCE).then_some(slot)
    }

    pub fn put_challenge(&self, key: &str, challenge: Vec<u8>) {
        let now = clock::now();
        let mut map = self.challenges.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, (_, expires)| *expires > now);
        if map.len() >= MAX_CHALLENGES {
            map.clear();
        }
        map.insert(key.to_string(), (challenge, now + CHALLENGE_TTL));
    }

    pub fn pop_challenge(&self, key: &str) -> Result<Vec<u8>> {
        let entry = self
            .challenges
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
        match entry {
            Some((challenge, expires)) if expires >= clock::now() => Ok(challenge),
            _ => Err(Error::invalid("challenge expired")),
        }
    }

    pub fn has_credentials(&self) -> Result<bool> {
        self.db.has_credentials()
    }

    pub fn has_password(&self) -> Result<bool> {
        Ok(self
            .db
            .get_setting_value("password_hash")?
            .is_some_and(|v| !v.is_null()))
    }

    pub fn set_password(&self, password: &str) -> Result<()> {
        if password.chars().count() < MIN_PASSWORD_LENGTH {
            return Err(Error::invalid(format!(
                "password must have at least {MIN_PASSWORD_LENGTH} characters"
            )));
        }
        self.db
            .set_setting("password_hash", &hash_password(password, None))
    }

    pub fn login_password(&self, password: &str, user_agent: &str) -> Result<String> {
        let stored: Option<String> = self.db.get_setting("password_hash").unwrap_or(None);
        if !check_password(password, stored.as_deref()) {
            return Err(Error::invalid("wrong password"));
        }
        self.create_session(user_agent)
    }

    pub fn credentials(&self) -> Result<Vec<CredentialInfo>> {
        Ok(self
            .db
            .list_credentials()?
            .into_iter()
            .map(|c| CredentialInfo {
                id: c.id,
                name: c.name,
                created: c.created,
            })
            .collect())
    }

    pub fn delete_credential(&self, id: &str) -> Result<()> {
        self.db.delete_credential(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn auth() -> (tempfile::TempDir, Auth) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::with_data_dir(dir.path());
        let db = Arc::new(Database::open(&cfg.db_path()).unwrap());
        (dir, Auth::new(&cfg, db))
    }

    #[test]
    fn password_login() {
        let (_d, auth) = auth();
        assert!(!auth.has_password().unwrap());
        assert_eq!(
            auth.set_password("short").unwrap_err().message(),
            "password must have at least 10 characters"
        );
        auth.set_password("correct horse battery").unwrap();
        assert!(auth.has_password().unwrap());
        assert!(auth.login_password("wrong", "ua").is_err());
        let token = auth.login_password("correct horse battery", "ua").unwrap();
        assert!(auth.check_session(Some(&token)).unwrap().is_some());
    }

    #[test]
    fn password_checks_are_limited_in_parallel_and_in_total() {
        let (_d, auth) = auth();
        let first = auth.password_slot().unwrap();
        let second = auth.password_slot().unwrap();
        assert!(auth.password_slot().is_none());
        drop(first);
        assert!(auth.password_slot().is_some());
        drop(second);
        let used = 4;
        let left = (0..PASSWORD_ATTEMPTS_PER_WINDOW)
            .filter(|_| auth.password_slot().is_some())
            .count();
        assert_eq!(left, PASSWORD_ATTEMPTS_PER_WINDOW - used);
    }

    #[test]
    fn pending_challenges_are_bounded() {
        let (_d, auth) = auth();
        for i in 0..3 * MAX_CHALLENGES {
            auth.put_challenge(&format!("login:{i}"), vec![1]);
        }
        assert!(auth.challenges.lock().unwrap().len() <= MAX_CHALLENGES);
    }

    #[test]
    fn challenges_are_single_use() {
        let (_d, auth) = auth();
        auth.put_challenge("login:x", vec![1, 2, 3]);
        assert_eq!(auth.pop_challenge("login:x").unwrap(), vec![1, 2, 3]);
        assert_eq!(
            auth.pop_challenge("login:x").unwrap_err().message(),
            "challenge expired"
        );
    }

    #[test]
    fn tokens_have_the_python_lengths() {
        assert_eq!(token_urlsafe(32).len(), 43);
        assert_eq!(token_urlsafe(16).len(), 22);
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
