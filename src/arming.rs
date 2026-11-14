use std::sync::Arc;

use serde_json::json;

use crate::clock;
use crate::db::Database;
use crate::error::{Error, Result};
use crate::presence::Presence;

pub const MODES: [&str; 3] = ["auto", "armed", "disarmed"];

pub struct Arming {
    db: Arc<Database>,
    presence: Arc<Presence>,
}

impl Arming {
    pub fn new(db: Arc<Database>, presence: Arc<Presence>) -> Arming {
        Arming { db, presence }
    }

    pub fn presence(&self) -> &Arc<Presence> {
        &self.presence
    }

    pub fn mode(&self) -> String {
        self.db.get_setting_or("mode", "auto".to_string())
    }

    pub fn set_mode(&self, mode: &str) -> Result<()> {
        if !MODES.contains(&mode) {
            return Err(Error::invalid("invalid mode"));
        }
        self.db.set_setting("mode", mode)
    }

    pub fn snooze_until(&self) -> f64 {
        self.db.get_setting_or("snooze_until", 0.0)
    }

    pub fn snooze(&self, minutes: f64) -> Result<()> {
        if minutes > 0.0 {
            self.db
                .set_setting("snooze_until", &(clock::now() + minutes * 60.0))
        } else {
            self.db.set_setting("snooze_until", &json!(0))
        }
    }

    pub fn privacy(&self) -> bool {
        match self.db.get_setting_value("privacy").ok().flatten() {
            Some(serde_json::Value::Bool(on)) => on,
            Some(serde_json::Value::Number(n)) => n.as_f64().is_some_and(|v| v != 0.0),
            _ => false,
        }
    }

    pub fn set_privacy(&self, on: bool) -> Result<()> {
        self.db.set_setting("privacy", &on)
    }

    pub fn decision(&self) -> (bool, &'static str) {
        if self.privacy() {
            return (false, "privacy");
        }
        if self.snooze_until() > clock::now() {
            return (false, "snoozed");
        }
        let mode = self.mode();
        if mode != "auto" {
            return (mode == "armed", "manual");
        }
        if !self.presence.configured() {
            return (false, "auto-without-device");
        }
        if self.presence.is_home() {
            return (false, "home");
        }
        (true, "away")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arming() -> (tempfile::TempDir, Arming) {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open(&dir.path().join("a.sqlite3")).unwrap());
        (dir, Arming::new(db, Presence::new(vec![], 300.0)))
    }

    #[test]
    fn mode_snooze_and_decision() {
        let (_d, arming) = arming();
        assert_eq!(arming.mode(), "auto");
        assert_eq!(arming.decision(), (false, "auto-without-device"));
        assert_eq!(
            arming.set_mode("bogus").unwrap_err().message(),
            "invalid mode"
        );
        arming.set_mode("armed").unwrap();
        assert_eq!(arming.decision(), (true, "manual"));
        arming.snooze(30.0).unwrap();
        assert_eq!(arming.decision(), (false, "snoozed"));
        assert!(arming.snooze_until() > clock::now());
        arming.snooze(0.0).unwrap();
        assert_eq!(arming.snooze_until(), 0.0);
        assert_eq!(arming.decision(), (true, "manual"));
        arming.set_mode("disarmed").unwrap();
        assert_eq!(arming.decision(), (false, "manual"));
    }

    #[test]
    fn auto_mode_follows_presence() {
        let (_d, arming) = arming();
        arming.presence().set_external(Some(true));
        assert_eq!(arming.decision(), (false, "home"));
        arming.presence().set_external(Some(false));
        assert_eq!(arming.decision(), (true, "away"));
    }

    #[test]
    fn privacy_wins_over_everything() {
        let (_d, arming) = arming();
        arming.set_mode("armed").unwrap();
        arming.set_privacy(true).unwrap();
        assert!(arming.privacy());
        assert_eq!(arming.decision(), (false, "privacy"));
        arming.set_privacy(false).unwrap();
        assert_eq!(arming.decision(), (true, "manual"));
    }

    #[test]
    fn values_stored_by_python_are_understood() {
        let (_d, arming) = arming();
        arming.db.execute("INSERT INTO settings(key, value) VALUES('mode', '\"disarmed\"'), ('snooze_until', '0'), ('privacy', 'true')", [])
            .unwrap();
        assert_eq!(
            (
                arming.mode().as_str(),
                arming.snooze_until(),
                arming.privacy()
            ),
            ("disarmed", 0.0, true)
        );
    }
}
