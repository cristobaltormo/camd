use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use toml::{Table, Value};

use crate::error::{Error, Result};

pub const DEFAULT_PATH: &str = "/etc/camd/config.toml";
pub const ENV_PREFIX: &str = "CAMD_";
pub const TRUE_WORDS: [&str; 4] = ["1", "true", "yes", "on"];

pub type SharedConfig = Arc<RwLock<Config>>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(skip)]
    pub integral_floats: BTreeSet<String>,
    pub camera_id: String,
    pub camera_name: String,
    pub camera_host: String,
    pub camera_api_port: i64,
    pub camera_stream_port: i64,
    pub camera_token: String,
    pub data_dir: PathBuf,
    pub listen_host: String,
    pub listen_port: i64,
    pub site_name: String,
    pub language: String,
    pub rp_id: String,
    pub origin: String,
    pub secure_cookies: bool,
    pub trust_proxy: bool,
    pub push_subject: String,
    pub notifich_url: String,
    pub notifich_key: String,
    pub ntfy_url: String,
    pub ntfy_token: String,
    pub mqtt_host: String,
    pub mqtt_port: i64,
    pub mqtt_user: String,
    pub mqtt_password: String,
    pub mqtt_tls: bool,
    pub mqtt_topic: String,
    pub mqtt_discovery: bool,
    pub presence_devices: Vec<String>,
    pub presence_grace_s: f64,
    pub retention_days: i64,
    pub max_disk_gb: f64,
    pub pre_roll_s: f64,
    pub post_roll_s: f64,
    pub max_event_s: f64,
    pub push_cooldown_s: f64,
    pub notify_motion: bool,
    pub notify_offline: bool,
    pub quiet_enabled: bool,
    pub quiet_start: String,
    pub quiet_end: String,
    pub detection: String,
    pub detect_labels: Vec<String>,
    pub detect_confidence: f64,
    pub unconfirmed_keep_h: f64,
    pub camera_enabled: bool,
    pub camera_vflip: bool,
    pub camera_hmirror: bool,
    pub camera_brightness: i64,
    pub camera_contrast: i64,
    pub camera_quality: i64,
    pub camera_framesize: i64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            integral_floats: BTreeSet::new(),
            camera_id: "main".into(),
            camera_name: String::new(),
            camera_host: String::new(),
            camera_api_port: 80,
            camera_stream_port: 81,
            camera_token: String::new(),
            data_dir: PathBuf::from("/var/lib/camd"),
            listen_host: "127.0.0.1".into(),
            listen_port: 8180,
            site_name: "Room camera".into(),
            language: "en".into(),
            rp_id: "localhost".into(),
            origin: "http://localhost:8180".into(),
            secure_cookies: false,
            trust_proxy: false,
            push_subject: "https://localhost".into(),
            notifich_url: String::new(),
            notifich_key: String::new(),
            ntfy_url: String::new(),
            ntfy_token: String::new(),
            mqtt_host: String::new(),
            mqtt_port: 1883,
            mqtt_user: String::new(),
            mqtt_password: String::new(),
            mqtt_tls: false,
            mqtt_topic: "camd".into(),
            mqtt_discovery: true,
            presence_devices: Vec::new(),
            presence_grace_s: 300.0,
            retention_days: 14,
            max_disk_gb: 6.0,
            pre_roll_s: 3.0,
            post_roll_s: 10.0,
            max_event_s: 90.0,
            push_cooldown_s: 60.0,
            notify_motion: true,
            notify_offline: true,
            quiet_enabled: false,
            quiet_start: "23:00".into(),
            quiet_end: "07:00".into(),
            detection: "alert".into(),
            detect_labels: vec!["person".into(), "cat".into(), "dog".into()],
            detect_confidence: 0.5,
            unconfirmed_keep_h: 24.0,
            camera_enabled: true,
            camera_vflip: false,
            camera_hmirror: false,
            camera_brightness: 0,
            camera_contrast: 0,
            camera_quality: 0,
            camera_framesize: 0,
        }
    }
}

impl Config {
    pub fn with_data_dir(data_dir: impl Into<PathBuf>) -> Config {
        Config {
            data_dir: data_dir.into(),
            ..Config::default()
        }
    }

    pub fn shared(self) -> SharedConfig {
        Arc::new(RwLock::new(self))
    }

    pub fn clips_dir(&self) -> PathBuf {
        self.data_dir.join("clips")
    }

    pub fn photos_dir(&self) -> PathBuf {
        self.data_dir.join("photos")
    }

    pub fn spool_dir(&self) -> PathBuf {
        self.data_dir.join("spool")
    }

    pub fn models_dir(&self) -> PathBuf {
        self.data_dir.join("models")
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("camd.sqlite3")
    }

    pub fn vapid_path(&self) -> PathBuf {
        self.data_dir.join("vapid_private.pem")
    }

    pub fn keys() -> Vec<String> {
        Config::default().to_table().keys().cloned().collect()
    }

    pub fn to_table(&self) -> Table {
        Table::try_from(self).expect("config serialises to a table")
    }

    pub fn from_table(table: Table) -> Result<Config> {
        Value::Table(table)
            .try_into()
            .map_err(|e: toml::de::Error| {
                Error::invalid(format!("invalid config: {}", e.message()))
            })
    }

    pub fn apply_env(&mut self, lookup: &dyn Fn(&str) -> Option<String>) -> Result<()> {
        let mut table = self.to_table();
        let mut changed = false;
        for (key, value) in table.iter_mut() {
            if let Some(raw) = lookup(&format!("{ENV_PREFIX}{}", key.to_uppercase())) {
                *value = coerce(key, &raw, value)?;
                changed = true;
            }
        }
        if changed {
            let mut integral = std::mem::take(&mut self.integral_floats);
            integral.retain(|key| {
                lookup(&format!("{ENV_PREFIX}{}", key.to_uppercase()))
                    .is_none_or(|raw| raw.trim().parse::<i64>().is_ok())
            });
            *self = Config::from_table(table)?;
            self.integral_floats = integral;
        }
        Ok(())
    }
}

pub fn coerce(key: &str, raw: &str, template: &Value) -> Result<Value> {
    let bad = || {
        Error::invalid(format!(
            "invalid value for {ENV_PREFIX}{}: {raw:?}",
            key.to_uppercase()
        ))
    };
    Ok(match template {
        Value::Boolean(_) => {
            Value::Boolean(TRUE_WORDS.contains(&raw.trim().to_lowercase().as_str()))
        }
        Value::Integer(_) => {
            Value::Integer(raw.trim().replace('_', "").parse().map_err(|_| bad())?)
        }
        Value::Float(_) => Value::Float(raw.trim().replace('_', "").parse().map_err(|_| bad())?),
        Value::Array(_) => Value::Array(
            raw.split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| Value::String(item.to_string()))
                .collect(),
        ),
        _ => Value::String(raw.to_string()),
    })
}

fn env_lookup(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

pub fn config_path(path: Option<&Path>) -> PathBuf {
    match path {
        Some(p) => p.to_path_buf(),
        None => std::env::var_os("CAMD_CONFIG")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_PATH)),
    }
}

fn read_file(path: &Path) -> Result<Table> {
    if !path.exists() {
        return Ok(Table::new());
    }
    let text = std::fs::read_to_string(path)?;
    text.parse::<Table>().map_err(|e| {
        Error::invalid(format!(
            "invalid config file {}: {}",
            path.display(),
            e.message()
        ))
    })
}

fn known_keys() -> BTreeSet<String> {
    Config::keys().into_iter().collect()
}

pub fn load(path: Option<&Path>) -> Result<Config> {
    load_with_env(&config_path(path), &env_lookup)
}

pub fn load_with_env(path: &Path, lookup: &dyn Fn(&str) -> Option<String>) -> Result<Config> {
    let mut raw = read_file(path)?;
    raw.remove("cameras");
    base_from_table(raw, lookup)
}

fn base_from_table(raw: Table, lookup: &dyn Fn(&str) -> Option<String>) -> Result<Config> {
    let known = known_keys();
    let unknown: Vec<&str> = raw
        .keys()
        .filter(|k| !known.contains(*k))
        .map(String::as_str)
        .collect();
    if !unknown.is_empty() {
        return Err(Error::invalid(format!(
            "unknown config keys: {}",
            unknown.join(", ")
        )));
    }
    let mut integral = BTreeSet::new();
    integral_floats(&raw, &mut integral);
    let mut table = Config::default().to_table();
    table.extend(raw);
    let mut cfg = Config::from_table(table)?;
    cfg.integral_floats = integral;
    cfg.apply_env(lookup)?;
    Ok(cfg)
}

fn integral_floats(raw: &Table, into: &mut BTreeSet<String>) {
    let template = Config::default().to_table();
    for (key, value) in raw {
        if matches!(template.get(key), Some(Value::Float(_))) {
            if matches!(value, Value::Integer(_)) {
                into.insert(key.clone());
            } else {
                into.remove(key);
            }
        }
    }
}

pub fn valid_camera_id(id: &str) -> bool {
    (1..=32).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub fn load_all(path: Option<&Path>) -> Result<Vec<Config>> {
    load_all_with_env(&config_path(path), &env_lookup)
}

pub fn load_all_with_env(
    path: &Path,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<Config>> {
    let mut raw = read_file(path)?;
    let extra = raw.remove("cameras");
    let base = base_from_table(raw, lookup)?;
    let entries = match extra {
        None => Vec::new(),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(Error::invalid("cameras must be a list of tables")),
    };
    let known = known_keys();
    let mut configs = vec![base.clone()];
    for entry in entries {
        let Value::Table(mut entry) = entry else {
            return Err(Error::invalid("invalid camera entry: ?"));
        };
        let id = entry.remove("id");
        let name = entry.remove("name");
        let id_text = match &id {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Integer(i)) => i.to_string(),
            Some(_) => "?".to_string(),
            None => "?".to_string(),
        };
        let unknown: Vec<&str> = entry
            .keys()
            .filter(|k| {
                !known.contains(*k) || k.as_str() == "camera_id" || k.as_str() == "camera_name"
            })
            .map(String::as_str)
            .collect();
        let id_ok = matches!(&id, Some(Value::String(s)) if valid_camera_id(s));
        if !unknown.is_empty() || !id_ok {
            let message = format!("invalid camera entry: {id_text} {}", unknown.join(", "));
            return Err(Error::invalid(message.trim().to_string()));
        }
        let name = match name {
            Some(Value::String(s)) => s,
            Some(_) => {
                return Err(Error::invalid(format!(
                    "invalid camera entry: {id_text} name"
                )));
            }
            None => id_text.clone(),
        };
        let mut table = base.to_table();
        table.insert(
            "data_dir".into(),
            Value::String(
                base.data_dir
                    .join("cameras")
                    .join(&id_text)
                    .to_string_lossy()
                    .into_owned(),
            ),
        );
        table.insert(
            "mqtt_topic".into(),
            Value::String(format!("{}_{}", base.mqtt_topic, id_text)),
        );
        let mut integral = base.integral_floats.clone();
        integral_floats(&entry, &mut integral);
        table.extend(entry);
        table.insert("camera_id".into(), Value::String(id_text.clone()));
        table.insert("camera_name".into(), Value::String(name));
        let mut camera = Config::from_table(table)?;
        camera.integral_floats = integral;
        configs.push(camera);
    }
    let ids: BTreeSet<&str> = configs.iter().map(|c| c.camera_id.as_str()).collect();
    if ids.len() != configs.len() {
        return Err(Error::invalid("camera ids must be unique"));
    }
    Ok(configs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name: &str| map.get(name).cloned()
    }

    fn none(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn environment_overrides_the_file_and_is_typed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.toml");
        std::fs::write(&path, "camera_host = \"10.0.0.5\"\nlisten_port = 9000\n").unwrap();
        let data = dir.path().join("data");
        let data_text = data.to_string_lossy().into_owned();
        let lookup = env(&[
            ("CAMD_CAMERA_HOST", "10.0.0.9"),
            ("CAMD_LISTEN_PORT", "9100"),
            ("CAMD_SECURE_COOKIES", "true"),
            ("CAMD_PRESENCE_DEVICES", "192.168.1.5, 192.168.1.6"),
            ("CAMD_DATA_DIR", &data_text),
        ]);
        let cfg = load_with_env(&path, &lookup).unwrap();
        assert_eq!(cfg.camera_host, "10.0.0.9");
        assert_eq!(cfg.listen_port, 9100);
        assert!(cfg.secure_cookies);
        assert_eq!(cfg.presence_devices, vec!["192.168.1.5", "192.168.1.6"]);
        assert_eq!(cfg.data_dir, data);
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = load_with_env(&dir.path().join("nope.toml"), &none).unwrap();
        assert_eq!(cfg.listen_port, 8180);
        assert_eq!(cfg.detection, "alert");
        assert_eq!(cfg.mqtt_host, "");
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn unknown_keys_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.toml");
        std::fs::write(&path, "listen_prt = 1\n").unwrap();
        let err = load_with_env(&path, &none).unwrap_err();
        assert_eq!(err.message(), "unknown config keys: listen_prt");
    }

    #[test]
    fn booleans_accept_the_python_words_and_anything_else_is_false() {
        for (raw, expected) in [
            ("1", true),
            ("YES", true),
            (" on ", true),
            ("true", true),
            ("no", false),
            ("2", false),
        ] {
            let cfg = {
                let mut c = Config::default();
                c.apply_env(&env(&[("CAMD_TRUST_PROXY", raw)])).unwrap();
                c
            };
            assert_eq!(cfg.trust_proxy, expected, "{raw}");
        }
    }

    #[test]
    fn floats_and_bad_numbers_in_the_environment() {
        let mut cfg = Config::default();
        cfg.apply_env(&env(&[
            ("CAMD_MAX_DISK_GB", "2.5"),
            ("CAMD_RETENTION_DAYS", " 7 "),
        ]))
        .unwrap();
        assert_eq!((cfg.max_disk_gb, cfg.retention_days), (2.5, 7));
        assert!(cfg.apply_env(&env(&[("CAMD_LISTEN_PORT", "x")])).is_err());
    }

    #[test]
    fn integers_in_the_file_are_accepted_for_float_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.toml");
        std::fs::write(&path, "max_disk_gb = 3\npre_roll_s = 1.5\n").unwrap();
        let cfg = load_with_env(&path, &none).unwrap();
        assert_eq!((cfg.max_disk_gb, cfg.pre_roll_s), (3.0, 1.5));
    }

    #[test]
    fn the_keys_match_the_python_dataclass() {
        let keys = Config::keys();
        assert_eq!(keys.len(), 51);
        for key in [
            "camera_id",
            "data_dir",
            "presence_devices",
            "camera_framesize",
            "unconfirmed_keep_h",
        ] {
            assert!(keys.contains(&key.to_string()));
        }
    }

    #[test]
    fn derived_paths() {
        let cfg = Config::with_data_dir("/d");
        assert_eq!(cfg.db_path(), PathBuf::from("/d/camd.sqlite3"));
        assert_eq!(cfg.clips_dir(), PathBuf::from("/d/clips"));
        assert_eq!(cfg.photos_dir(), PathBuf::from("/d/photos"));
        assert_eq!(cfg.spool_dir(), PathBuf::from("/d/spool"));
        assert_eq!(cfg.models_dir(), PathBuf::from("/d/models"));
        assert_eq!(cfg.vapid_path(), PathBuf::from("/d/vapid_private.pem"));
    }

    #[test]
    fn extra_cameras_come_from_the_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let data = dir.path().join("data");
        std::fs::write(
            &path,
            format!(
                "data_dir = \"{}\"\nmqtt_topic = \"home\"\n\n[[cameras]]\nid = \"porch\"\nname = \"Porch\"\ncamera_host = \"10.0.0.9\"\n",
                data.display()
            ),
        )
        .unwrap();
        let all = load_all_with_env(&path, &none).unwrap();
        let (main, porch) = (&all[0], &all[1]);
        assert_eq!(main.camera_id, "main");
        assert_eq!(porch.camera_id, "porch");
        assert_eq!(porch.camera_name, "Porch");
        assert_eq!(porch.camera_host, "10.0.0.9");
        assert_eq!(porch.data_dir, data.join("cameras").join("porch"));
        assert_eq!(porch.mqtt_topic, "home_porch");
        assert_eq!(main.camera_host, "");
    }

    #[test]
    fn extra_cameras_inherit_the_environment_but_their_own_keys_win() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[[cameras]]\nid = \"porch\"\ncamera_token = \"u\"\n").unwrap();
        let lookup = env(&[("CAMD_CAMERA_TOKEN", "t"), ("CAMD_SITE_NAME", "Home")]);
        let all = load_all_with_env(&path, &lookup).unwrap();
        assert_eq!(all[0].camera_token, "t");
        assert_eq!(all[1].camera_token, "u");
        assert_eq!(all[1].site_name, "Home");
        assert_eq!(all[1].camera_name, "porch");
    }

    #[test]
    fn duplicate_or_unknown_camera_entries_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for (body, message) in [
            ("[[cameras]]\nid = \"main\"\n", "camera ids must be unique"),
            ("[[cameras]]\nid = \"../x\"\n", "invalid camera entry: ../x"),
            (
                "[[cameras]]\nid = \"x\"\nbogus = 1\n",
                "invalid camera entry: x bogus",
            ),
            ("[[cameras]]\nname = \"x\"\n", "invalid camera entry: ?"),
        ] {
            std::fs::write(&path, body).unwrap();
            assert_eq!(
                load_all_with_env(&path, &none).unwrap_err().message(),
                message
            );
        }
    }

    #[test]
    fn a_single_camera_file_loads_one_config() {
        let dir = tempfile::tempdir().unwrap();
        let all = load_all_with_env(&dir.path().join("none.toml"), &none).unwrap();
        assert_eq!(all.len(), 1);
    }
}
