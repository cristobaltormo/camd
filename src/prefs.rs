use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::clock;
use crate::config::Config;
use crate::db::Database;
use crate::error::{Error, Result};
use crate::labels::COCO_LABELS;

pub const DETECTION_MODES: [&str; 3] = ["off", "alert", "strict"];
pub const LANGUAGES: [&str; 2] = ["en", "es"];
pub const FRAMESIZES: [&str; 5] = ["0", "5", "8", "9", "10"];
pub const MAX_LABELS: usize = 12;
pub const MAX_RECTS: usize = 10;
pub const DETECTION_KEYS: [&str; 3] = ["detection", "detect_labels", "detect_confidence"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Float(f64, f64),
    Int(f64, f64),
    Bool,
    Hhmm,
    Choice(&'static [&'static str]),
    Quality,
    ChoiceInt(&'static [&'static str]),
    Labels,
}

pub const SPEC: [(&str, Kind); 23] = [
    ("pre_roll_s", Kind::Float(0.0, 10.0)),
    ("post_roll_s", Kind::Float(3.0, 120.0)),
    ("max_event_s", Kind::Float(15.0, 600.0)),
    ("max_disk_gb", Kind::Float(0.5, 500.0)),
    ("presence_grace_s", Kind::Float(30.0, 3600.0)),
    ("push_cooldown_s", Kind::Float(10.0, 7200.0)),
    ("unconfirmed_keep_h", Kind::Float(1.0, 720.0)),
    ("detect_confidence", Kind::Float(0.2, 0.9)),
    ("notify_motion", Kind::Bool),
    ("notify_offline", Kind::Bool),
    ("quiet_enabled", Kind::Bool),
    ("quiet_start", Kind::Hhmm),
    ("quiet_end", Kind::Hhmm),
    ("camera_enabled", Kind::Bool),
    ("camera_vflip", Kind::Bool),
    ("camera_hmirror", Kind::Bool),
    ("camera_brightness", Kind::Int(-2.0, 2.0)),
    ("camera_contrast", Kind::Int(-2.0, 2.0)),
    ("camera_quality", Kind::Quality),
    ("camera_framesize", Kind::ChoiceInt(&FRAMESIZES)),
    ("detection", Kind::Choice(&DETECTION_MODES)),
    ("language", Kind::Choice(&LANGUAGES)),
    ("detect_labels", Kind::Labels),
];

pub fn kind(key: &str) -> Option<Kind> {
    SPEC.iter().find(|(k, _)| *k == key).map(|(_, kind)| *kind)
}

pub fn is_camera_key(key: &str) -> bool {
    key.starts_with("camera_") && key != "camera_enabled" && kind(key).is_some()
}

pub fn is_detection_key(key: &str) -> bool {
    DETECTION_KEYS.contains(&key)
}

fn number_text(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

pub fn valid_hhmm(text: &str) -> bool {
    let b = text.as_bytes();
    if b.len() != 5
        || b[2] != b':'
        || !b
            .iter()
            .enumerate()
            .all(|(i, c)| i == 2 || c.is_ascii_digit())
    {
        return false;
    }
    let hour = (b[0] - b'0') * 10 + (b[1] - b'0');
    hour <= 23 && b[3] <= b'5'
}

fn python_repr(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub fn validate(key: &str, value: &Value) -> Result<Value> {
    let Some(spec) = kind(key) else {
        return Err(Error::invalid(format!("unknown setting: {key}")));
    };
    match spec {
        Kind::Bool => match value {
            Value::Bool(_) => Ok(value.clone()),
            _ => Err(Error::invalid(format!("{key} must be true or false"))),
        },
        Kind::Hhmm => match value {
            Value::String(s) if valid_hhmm(s) => Ok(value.clone()),
            _ => Err(Error::invalid(format!("{key} must be HH:MM"))),
        },
        Kind::Choice(options) => match value {
            Value::String(s) if options.contains(&s.as_str()) => Ok(value.clone()),
            _ => Err(Error::invalid(format!(
                "{key} must be one of {}",
                options.join(", ")
            ))),
        },
        Kind::Quality => match value.as_i64() {
            Some(q) if value.is_i64() && (q == 0 || (4..=63).contains(&q)) => Ok(json!(q)),
            _ => Err(Error::invalid(format!(
                "{key} must be 0 or between 4 and 63"
            ))),
        },
        Kind::ChoiceInt(options) => {
            if !value.is_boolean() && options.contains(&python_repr(value).as_str()) {
                Ok(json!(python_repr(value).parse::<i64>().unwrap_or(0)))
            } else {
                Err(Error::invalid(format!(
                    "{key} must be one of {}",
                    options.join(", ")
                )))
            }
        }
        Kind::Labels => {
            let items = value
                .as_array()
                .filter(|items| (1..=MAX_LABELS).contains(&items.len()));
            let names: Option<Vec<&str>> = items.and_then(|items| {
                items
                    .iter()
                    .map(|v| v.as_str().filter(|s| COCO_LABELS.contains(s)))
                    .collect()
            });
            match names {
                Some(names) => {
                    let mut unique: Vec<&str> = Vec::new();
                    for name in names {
                        if !unique.contains(&name) {
                            unique.push(name);
                        }
                    }
                    Ok(json!(unique))
                }
                None => Err(Error::invalid(format!(
                    "{key} must list between 1 and {MAX_LABELS} known object names"
                ))),
            }
        }
        Kind::Float(low, high) | Kind::Int(low, high) => {
            let Some(number) = value.as_f64().filter(|_| value.is_number()) else {
                return Err(Error::invalid(format!("{key} must be a number")));
            };
            if !(low <= number && number <= high) {
                return Err(Error::invalid(format!(
                    "{key} must be between {} and {}",
                    number_text(low),
                    number_text(high)
                )));
            }
            if matches!(spec, Kind::Int(..)) {
                Ok(json!(number.trunc() as i64))
            } else {
                Ok(json!(number))
            }
        }
    }
}

fn number(cfg: &Config, key: &str, value: f64) -> Value {
    if cfg.integral_floats.contains(key) && value.fract() == 0.0 && value.abs() < 9e15 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

pub fn get(cfg: &Config, key: &str) -> Option<Value> {
    Some(match key {
        "pre_roll_s" => number(cfg, key, cfg.pre_roll_s),
        "post_roll_s" => number(cfg, key, cfg.post_roll_s),
        "max_event_s" => number(cfg, key, cfg.max_event_s),
        "max_disk_gb" => number(cfg, key, cfg.max_disk_gb),
        "presence_grace_s" => number(cfg, key, cfg.presence_grace_s),
        "push_cooldown_s" => number(cfg, key, cfg.push_cooldown_s),
        "unconfirmed_keep_h" => number(cfg, key, cfg.unconfirmed_keep_h),
        "detect_confidence" => number(cfg, key, cfg.detect_confidence),
        "notify_motion" => json!(cfg.notify_motion),
        "notify_offline" => json!(cfg.notify_offline),
        "quiet_enabled" => json!(cfg.quiet_enabled),
        "quiet_start" => json!(cfg.quiet_start),
        "quiet_end" => json!(cfg.quiet_end),
        "camera_enabled" => json!(cfg.camera_enabled),
        "camera_vflip" => json!(cfg.camera_vflip),
        "camera_hmirror" => json!(cfg.camera_hmirror),
        "camera_brightness" => json!(cfg.camera_brightness),
        "camera_contrast" => json!(cfg.camera_contrast),
        "camera_quality" => json!(cfg.camera_quality),
        "camera_framesize" => json!(cfg.camera_framesize),
        "detection" => json!(cfg.detection),
        "language" => json!(cfg.language),
        "detect_labels" => json!(cfg.detect_labels),
        _ => return None,
    })
}

pub fn set(cfg: &mut Config, key: &str, value: &Value) -> Result<()> {
    cfg.integral_floats.remove(key);
    let bad = || Error::invalid(format!("{key} has the wrong type"));
    let float = || value.as_f64().ok_or_else(bad);
    let int = || value.as_i64().ok_or_else(bad);
    let flag = || value.as_bool().ok_or_else(bad);
    let text = || value.as_str().map(str::to_string).ok_or_else(bad);
    match key {
        "pre_roll_s" => cfg.pre_roll_s = float()?,
        "post_roll_s" => cfg.post_roll_s = float()?,
        "max_event_s" => cfg.max_event_s = float()?,
        "max_disk_gb" => cfg.max_disk_gb = float()?,
        "presence_grace_s" => cfg.presence_grace_s = float()?,
        "push_cooldown_s" => cfg.push_cooldown_s = float()?,
        "unconfirmed_keep_h" => cfg.unconfirmed_keep_h = float()?,
        "detect_confidence" => cfg.detect_confidence = float()?,
        "notify_motion" => cfg.notify_motion = flag()?,
        "notify_offline" => cfg.notify_offline = flag()?,
        "quiet_enabled" => cfg.quiet_enabled = flag()?,
        "quiet_start" => cfg.quiet_start = text()?,
        "quiet_end" => cfg.quiet_end = text()?,
        "camera_enabled" => cfg.camera_enabled = flag()?,
        "camera_vflip" => cfg.camera_vflip = flag()?,
        "camera_hmirror" => cfg.camera_hmirror = flag()?,
        "camera_brightness" => cfg.camera_brightness = int()?,
        "camera_contrast" => cfg.camera_contrast = int()?,
        "camera_quality" => cfg.camera_quality = int()?,
        "camera_framesize" => cfg.camera_framesize = int()?,
        "detection" => cfg.detection = text()?,
        "language" => cfg.language = text()?,
        "detect_labels" => {
            cfg.detect_labels = serde_json::from_value(value.clone()).map_err(|_| bad())?
        }
        _ => return Err(Error::invalid(format!("unknown setting: {key}"))),
    }
    Ok(())
}

pub fn load(cfg: &mut Config, db: &Database) -> Result<()> {
    let stored = db.get_setting_value("prefs")?;
    let Some(Value::Object(stored)) = stored else {
        return Ok(());
    };
    for (key, value) in &stored {
        if kind(key).is_none() {
            continue;
        }
        if let Ok(clean) = validate(key, value) {
            set(cfg, key, &clean)?;
        }
    }
    Ok(())
}

pub fn load_stored(cfg: &mut Config, db: &Database) -> Result<()> {
    if let Some(days) = db.get_setting_value("retention_days")?
        && let Some(days) = days.as_f64().filter(|d| *d != 0.0)
    {
        cfg.retention_days = days.trunc() as i64;
    }
    load(cfg, db)
}

pub fn update(
    cfg: &mut Config,
    db: &Database,
    patch: &Map<String, Value>,
) -> Result<BTreeSet<String>> {
    let mut clean: Vec<(String, Value)> = Vec::new();
    for (key, value) in patch {
        if kind(key).is_none() {
            return Err(Error::invalid(format!("unknown setting: {key}")));
        }
        clean.push((key.clone(), validate(key, value)?));
    }
    let mut stored = match db.get_setting_value("prefs")? {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    for (key, value) in &clean {
        stored.insert(key.clone(), value.clone());
    }
    db.set_setting("prefs", &stored)?;
    for (key, value) in &clean {
        set(cfg, key, value)?;
    }
    Ok(clean.into_iter().map(|(k, _)| k).collect())
}

pub fn snapshot(cfg: &Config) -> Map<String, Value> {
    SPEC.iter()
        .filter_map(|(key, _)| get(cfg, key).map(|v| (key.to_string(), v)))
        .collect()
}

pub fn minutes(hhmm: &str) -> u32 {
    let hours: u32 = hhmm.get(..2).and_then(|h| h.parse().ok()).unwrap_or(0);
    let mins: u32 = hhmm.get(3..5).and_then(|m| m.parse().ok()).unwrap_or(0);
    hours * 60 + mins
}

pub fn in_quiet_hours(cfg: &Config, now: f64) -> bool {
    if !cfg.quiet_enabled {
        return false;
    }
    let current = clock::local(now).minutes_of_day();
    let (start, end) = (minutes(&cfg.quiet_start), minutes(&cfg.quiet_end));
    if start == end {
        return false;
    }
    if start < end {
        start <= current && current < end
    } else {
        current >= start || current < end
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionSettings {
    pub sensitivity: i64,
    pub ignore_rects: Vec<Vec<f64>>,
    pub alert_rects: Vec<Vec<f64>>,
}

impl Default for MotionSettings {
    fn default() -> Self {
        MotionSettings {
            sensitivity: 5,
            ignore_rects: Vec::new(),
            alert_rects: Vec::new(),
        }
    }
}

fn rects(value: Option<&Value>) -> Vec<Vec<f64>> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_array)
                .map(|rect| rect.iter().filter_map(Value::as_f64).collect())
                .collect()
        })
        .unwrap_or_default()
}

pub fn load_motion(db: &Database) -> MotionSettings {
    let raw = db
        .get_setting_value("motion")
        .ok()
        .flatten()
        .unwrap_or_else(|| json!({}));
    MotionSettings {
        sensitivity: raw
            .get("sensitivity")
            .and_then(Value::as_f64)
            .map(|s| s.trunc() as i64)
            .unwrap_or(5),
        ignore_rects: rects(raw.get("ignore_rects")),
        alert_rects: rects(raw.get("alert_rects")),
    }
}

pub fn save_motion(db: &Database, settings: &MotionSettings) -> Result<()> {
    db.set_setting("motion", settings)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsBody {
    pub sensitivity: Option<i64>,
    pub ignore_rects: Option<Vec<Vec<f64>>>,
    pub alert_rects: Option<Vec<Vec<f64>>>,
    pub retention_days: Option<i64>,
    pub prefs: Map<String, Value>,
}

#[derive(Clone, Copy, PartialEq)]
enum Field {
    Int(Option<(i64, i64)>),
    Float,
    StrictBool,
    Str,
    StrList,
    Rects,
}

const BODY_FIELDS: [(&str, Field); 27] = [
    ("sensitivity", Field::Int(Some((1, 10)))),
    ("ignore_rects", Field::Rects),
    ("alert_rects", Field::Rects),
    ("retention_days", Field::Int(Some((1, 365)))),
    ("pre_roll_s", Field::Float),
    ("post_roll_s", Field::Float),
    ("max_event_s", Field::Float),
    ("max_disk_gb", Field::Float),
    ("presence_grace_s", Field::Float),
    ("push_cooldown_s", Field::Float),
    ("unconfirmed_keep_h", Field::Float),
    ("detect_confidence", Field::Float),
    ("detection", Field::Str),
    ("detect_labels", Field::StrList),
    ("language", Field::Str),
    ("notify_motion", Field::StrictBool),
    ("notify_offline", Field::StrictBool),
    ("quiet_enabled", Field::StrictBool),
    ("quiet_start", Field::Str),
    ("quiet_end", Field::Str),
    ("camera_enabled", Field::StrictBool),
    ("camera_vflip", Field::StrictBool),
    ("camera_hmirror", Field::StrictBool),
    ("camera_brightness", Field::Int(None)),
    ("camera_contrast", Field::Int(None)),
    ("camera_quality", Field::Int(None)),
    ("camera_framesize", Field::Int(None)),
];

pub fn field_error(kind: &str, loc: &[Value], msg: &str, input: &Value) -> Value {
    let mut full = vec![json!("body")];
    full.extend_from_slice(loc);
    json!({"type": kind, "loc": full, "msg": msg, "input": input})
}

fn lax_float(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().replace('_', "").parse::<f64>().ok(),
        _ => None,
    }
}

fn lax_int(value: &Value) -> std::result::Result<i64, (&'static str, &'static str)> {
    let from_float = |f: f64| {
        if f.fract() != 0.0 {
            Err((
                "int_from_float",
                "Input should be a valid integer, got a number with a fractional part",
            ))
        } else if f.abs() > 9.0e18 {
            Err((
                "int_parsing_size",
                "Unable to parse input string as an integer, exceeded maximum size",
            ))
        } else {
            Ok(f as i64)
        }
    };
    match value {
        Value::Bool(b) => Ok(*b as i64),
        Value::Number(n) => match n.as_i64() {
            Some(i) => Ok(i),
            None => from_float(n.as_f64().unwrap_or(f64::NAN)),
        },
        Value::String(s) => {
            let clean = s.trim().replace('_', "");
            match clean.parse::<i64>() {
                Ok(i) => Ok(i),
                Err(_) => match clean.parse::<f64>() {
                    Ok(f) if f.is_finite() && f.fract() == 0.0 => Ok(f as i64),
                    _ => Err((
                        "int_parsing",
                        "Input should be a valid integer, unable to parse string as an integer",
                    )),
                },
            }
        }
        _ => Err(("int_type", "Input should be a valid integer")),
    }
}

fn float_rect_list(key: &str, value: &Value, errors: &mut Vec<Value>) -> Option<Vec<Vec<f64>>> {
    let Some(items) = value.as_array() else {
        errors.push(field_error(
            "list_type",
            &[json!(key)],
            "Input should be a valid list",
            value,
        ));
        return None;
    };
    let mut out = Vec::new();
    let mut ok = true;
    for (i, item) in items.iter().enumerate() {
        let Some(inner) = item.as_array() else {
            errors.push(field_error(
                "list_type",
                &[json!(key), json!(i)],
                "Input should be a valid list",
                item,
            ));
            ok = false;
            continue;
        };
        let mut rect = Vec::new();
        for (j, v) in inner.iter().enumerate() {
            match lax_float(v) {
                Some(f) => rect.push(f),
                None => {
                    errors.push(field_error(
                        "float_parsing",
                        &[json!(key), json!(i), json!(j)],
                        "Input should be a valid number, unable to parse string as a number",
                        v,
                    ));
                    ok = false;
                }
            }
        }
        out.push(rect);
    }
    if ok && out.len() > MAX_RECTS {
        errors.push(field_error(
            "too_long",
            &[json!(key)],
            &format!(
                "List should have at most {MAX_RECTS} items after validation, not {}",
                out.len()
            ),
            value,
        ));
        return None;
    }
    ok.then_some(out)
}

pub fn parse_settings_body(body: &Value) -> std::result::Result<SettingsBody, Vec<Value>> {
    let Some(map) = body.as_object() else {
        return Err(vec![
            json!({"type": "model_type", "loc": ["body"], "msg": "Input should be an object", "input": body}),
        ]);
    };
    let mut errors = Vec::new();
    let mut out = SettingsBody::default();
    for (key, value) in map {
        let Some((_, field)) = BODY_FIELDS.iter().find(|(name, _)| name == key) else {
            errors.push(field_error(
                "extra_forbidden",
                &[json!(key)],
                "Extra inputs are not permitted",
                value,
            ));
            continue;
        };
        if value.is_null() {
            continue;
        }
        let loc = [json!(key)];
        let parsed: Option<Value> = match field {
            Field::Int(bounds) => match lax_int(value) {
                Ok(n) => match bounds {
                    Some((low, _)) if n < *low => {
                        errors.push(json!({"type": "greater_than_equal", "loc": ["body", key],
                            "msg": format!("Input should be greater than or equal to {low}"), "input": value, "ctx": {"ge": low}}));
                        None
                    }
                    Some((_, high)) if n > *high => {
                        errors.push(json!({"type": "less_than_equal", "loc": ["body", key],
                            "msg": format!("Input should be less than or equal to {high}"), "input": value, "ctx": {"le": high}}));
                        None
                    }
                    _ => Some(json!(n)),
                },
                Err((kind, msg)) => {
                    errors.push(field_error(kind, &loc, msg, value));
                    None
                }
            },
            Field::Float => match lax_float(value) {
                Some(f) => Some(
                    serde_json::Number::from_f64(f)
                        .map(Value::Number)
                        .unwrap_or(json!(f64::MAX)),
                ),
                None => {
                    let (kind, msg) = if value.is_string() {
                        (
                            "float_parsing",
                            "Input should be a valid number, unable to parse string as a number",
                        )
                    } else {
                        ("float_type", "Input should be a valid number")
                    };
                    errors.push(field_error(kind, &loc, msg, value));
                    None
                }
            },
            Field::StrictBool => {
                if value.is_boolean() {
                    Some(value.clone())
                } else {
                    errors.push(field_error(
                        "bool_type",
                        &loc,
                        "Input should be a valid boolean",
                        value,
                    ));
                    None
                }
            }
            Field::Str => {
                if value.is_string() {
                    Some(value.clone())
                } else {
                    errors.push(field_error(
                        "string_type",
                        &loc,
                        "Input should be a valid string",
                        value,
                    ));
                    None
                }
            }
            Field::StrList => match value.as_array() {
                None => {
                    errors.push(field_error(
                        "list_type",
                        &loc,
                        "Input should be a valid list",
                        value,
                    ));
                    None
                }
                Some(items) => {
                    let mut ok = true;
                    for (i, item) in items.iter().enumerate() {
                        if !item.is_string() {
                            errors.push(field_error(
                                "string_type",
                                &[json!(key), json!(i)],
                                "Input should be a valid string",
                                item,
                            ));
                            ok = false;
                        }
                    }
                    ok.then(|| value.clone())
                }
            },
            Field::Rects => float_rect_list(key, value, &mut errors).map(|r| json!(r)),
        };
        let Some(parsed) = parsed else { continue };
        match key.as_str() {
            "sensitivity" => out.sensitivity = parsed.as_i64(),
            "retention_days" => out.retention_days = parsed.as_i64(),
            "ignore_rects" => out.ignore_rects = serde_json::from_value(parsed).ok(),
            "alert_rects" => out.alert_rects = serde_json::from_value(parsed).ok(),
            _ => {
                out.prefs.insert(key.clone(), parsed);
            }
        }
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

pub fn valid_rect(rect: &[f64]) -> bool {
    rect.len() == 4 && rect.iter().all(|v| (0.0..=1.0).contains(v))
}

pub struct Applied {
    pub changed: BTreeSet<String>,
    pub motion: MotionSettings,
}

pub fn apply_settings(cfg: &mut Config, db: &Database, body: &SettingsBody) -> Result<Applied> {
    let current = load_motion(db);
    let motion = MotionSettings {
        sensitivity: body.sensitivity.unwrap_or(current.sensitivity),
        ignore_rects: body.ignore_rects.clone().unwrap_or(current.ignore_rects),
        alert_rects: body.alert_rects.clone().unwrap_or(current.alert_rects),
    };
    let retention = body.retention_days.unwrap_or(cfg.retention_days);
    if !motion
        .ignore_rects
        .iter()
        .chain(&motion.alert_rects)
        .all(|r| valid_rect(r))
    {
        return Err(Error::invalid("invalid rect"));
    }
    let changed = update(cfg, db, &body.prefs)?;
    save_motion(db, &motion)?;
    cfg.retention_days = retention;
    db.set_setting("retention_days", &retention)?;
    Ok(Applied { changed, motion })
}

pub fn settings_json(cfg: &Config, motion: &MotionSettings) -> Value {
    let mut out = Map::new();
    out.insert("sensitivity".into(), json!(motion.sensitivity));
    out.insert("ignore_rects".into(), json!(motion.ignore_rects));
    out.insert("alert_rects".into(), json!(motion.alert_rects));
    out.insert("retention_days".into(), json!(cfg.retention_days));
    out.insert("available_labels".into(), json!(COCO_LABELS.as_slice()));
    out.extend(snapshot(cfg));
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, Config, Database) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::with_data_dir(dir.path());
        let db = Database::open(&cfg.db_path()).unwrap();
        (dir, cfg, db)
    }

    fn post(cfg: &mut Config, db: &Database, body: Value) -> std::result::Result<Applied, u16> {
        let parsed = parse_settings_body(&body).map_err(|_| 422u16)?;
        apply_settings(cfg, db, &parsed).map_err(|_| 400u16)
    }

    #[test]
    fn settings_apply_live() {
        let (_d, mut cfg, db) = setup();
        let applied = post(
            &mut cfg,
            &db,
            json!({"post_roll_s": 25, "notify_motion": false, "camera_brightness": 1, "quiet_start": "22:30"}),
        )
        .unwrap();
        assert_eq!(applied.changed.len(), 4);
        let got = settings_json(&cfg, &load_motion(&db));
        assert_eq!(got["post_roll_s"], json!(25.0));
        assert_eq!(got["notify_motion"], json!(false));
        assert_eq!(got["camera_brightness"], json!(1));
        assert_eq!(got["sensitivity"], json!(5));
        assert_eq!(cfg.post_roll_s, 25.0);
        assert_eq!(cfg.quiet_start, "22:30");
    }

    #[test]
    fn motion_settings_roundtrip_and_validation() {
        let (_d, mut cfg, db) = setup();
        post(
            &mut cfg,
            &db,
            json!({"sensitivity": 8, "ignore_rects": [[0.1, 0.1, 0.2, 0.2]], "retention_days": 7}),
        )
        .unwrap();
        let got = settings_json(&cfg, &load_motion(&db));
        assert_eq!(
            (
                got["sensitivity"].clone(),
                got["ignore_rects"].clone(),
                got["retention_days"].clone()
            ),
            (json!(8), json!([[0.1, 0.1, 0.2, 0.2]]), json!(7))
        );
        assert_eq!(
            post(&mut cfg, &db, json!({"ignore_rects": [[2, 0, 0, 0]]})).err(),
            Some(400)
        );
        assert_eq!(
            post(&mut cfg, &db, json!({"ignore_rects": [[0.1, 0.1, 0.1]]})).err(),
            Some(400)
        );
    }

    #[test]
    fn partial_update_keeps_other_settings() {
        let (_d, mut cfg, db) = setup();
        post(
            &mut cfg,
            &db,
            json!({"sensitivity": 8, "ignore_rects": [[0.1, 0.1, 0.2, 0.2]], "retention_days": 30}),
        )
        .unwrap();
        post(&mut cfg, &db, json!({"pre_roll_s": 5})).unwrap();
        let got = settings_json(&cfg, &load_motion(&db));
        assert_eq!(got["sensitivity"], json!(8));
        assert_eq!(got["ignore_rects"], json!([[0.1, 0.1, 0.2, 0.2]]));
        assert_eq!(got["retention_days"], json!(30));
        assert_eq!(
            db.get_setting_value("retention_days").unwrap(),
            Some(json!(30))
        );
    }

    #[test]
    fn invalid_settings_are_rejected() {
        for body in [
            json!({"post_roll_s": 9999}),
            json!({"quiet_start": "25:00"}),
            json!({"notify_motion": "yes"}),
            json!({"camera_brightness": 3}),
            json!({"nope": 1}),
            json!({"detection": "maybe"}),
            json!({"detect_labels": ["unicorn"]}),
            json!({"detect_labels": []}),
            json!({"language": "xx"}),
            json!({"detect_confidence": 0.99}),
            json!({"camera_framesize": 640}),
        ] {
            let (_d, mut cfg, db) = setup();
            assert!(post(&mut cfg, &db, body.clone()).is_err(), "{body}");
            assert_eq!(cfg, Config::with_data_dir(cfg.data_dir.clone()));
        }
    }

    #[test]
    fn status_codes_follow_pydantic_then_prefs() {
        let (_d, mut cfg, db) = setup();
        for (body, code) in [
            (json!({"sensitivity": 11}), 422),
            (json!({"nope": 1}), 422),
            (json!({"camera_brightness": 1.5}), 422),
            (json!({"language": 5}), 422),
            (json!({"detect_labels": "person"}), 422),
            (json!({"retention_days": 0}), 422),
            (json!({"camera_quality": true}), 400),
            (json!({"quiet_start": "7:00"}), 400),
            (json!({"post_roll_s": "nan"}), 400),
        ] {
            assert_eq!(
                post(&mut cfg, &db, body.clone()).err(),
                Some(code),
                "{body}"
            );
        }
        for body in [
            json!({"camera_brightness": 1.0}),
            json!({"camera_brightness": "1"}),
            json!({"post_roll_s": "25"}),
            json!({"detection": null}),
            json!({"ignore_rects": [["0.1", "0.1", "0.1", "0.1"]]}),
            json!({"sensitivity": "5"}),
            json!({"sensitivity": 5.0}),
            json!({"sensitivity": true}),
            json!({"camera_framesize": "8"}),
        ] {
            assert!(post(&mut cfg, &db, body.clone()).is_ok(), "{body}");
        }
        assert_eq!(cfg.camera_framesize, 8);
        assert_eq!(load_motion(&db).sensitivity, 1);
    }

    #[test]
    fn pydantic_error_details_are_a_list() {
        let errors = parse_settings_body(&json!({"sensitivity": 11})).unwrap_err();
        assert_eq!(
            errors,
            vec![
                json!({"type": "less_than_equal", "loc": ["body", "sensitivity"], "msg": "Input should be less than or equal to 10",
                        "input": 11, "ctx": {"le": 10}})
            ]
        );
        assert!(parse_settings_body(&json!([])).is_err());
        let too_many = json!({"alert_rects": vec![vec![0, 0, 0, 0]; 11]});
        assert_eq!(
            parse_settings_body(&too_many).unwrap_err()[0]["type"],
            "too_long"
        );
    }

    #[test]
    fn detection_settings_are_stored_and_deduplicated() {
        let (_d, mut cfg, db) = setup();
        post(&mut cfg, &db, json!({"detection": "strict", "detect_labels": ["person", "dog", "person"], "detect_confidence": 0.6}))
            .unwrap();
        let got = settings_json(&cfg, &load_motion(&db));
        assert_eq!(got["detection"], "strict");
        assert_eq!(got["detect_labels"], json!(["person", "dog"]));
        assert_eq!(got["detect_confidence"], json!(0.6));
    }

    #[test]
    fn prefs_persist_across_restarts() {
        let (_d, mut cfg, db) = setup();
        post(&mut cfg, &db, json!({"max_disk_gb": 3, "quiet_enabled": true, "detection": "alert", "retention_days": 9})).unwrap();
        let mut fresh = Config::with_data_dir(cfg.data_dir.clone());
        load_stored(&mut fresh, &db).unwrap();
        assert_eq!(
            (
                fresh.max_disk_gb,
                fresh.quiet_enabled,
                fresh.detection.as_str(),
                fresh.retention_days
            ),
            (3.0, true, "alert", 9)
        );
    }

    #[test]
    fn invalid_stored_prefs_are_skipped() {
        let (_d, mut cfg, db) = setup();
        db.set_setting(
            "prefs",
            &json!({"post_roll_s": 9999, "pre_roll_s": 2, "gone": 1, "language": "es"}),
        )
        .unwrap();
        load(&mut cfg, &db).unwrap();
        assert_eq!(
            (cfg.post_roll_s, cfg.pre_roll_s, cfg.language.as_str()),
            (10.0, 2.0, "es")
        );
    }

    #[test]
    fn resolution_is_a_known_size() {
        let (_d, mut cfg, db) = setup();
        assert_eq!(
            post(&mut cfg, &db, json!({"camera_framesize": 640})).err(),
            Some(400)
        );
        post(&mut cfg, &db, json!({"camera_framesize": 8})).unwrap();
        assert_eq!(cfg.camera_framesize, 8);
        post(&mut cfg, &db, json!({"camera_framesize": 0})).unwrap();
        assert_eq!(cfg.camera_framesize, 0);
    }

    #[test]
    fn jpeg_quality_must_be_zero_or_within_the_camera_range() {
        let (_d, mut cfg, db) = setup();
        for value in [json!(1), json!(3), json!(64), json!(true)] {
            assert!(post(&mut cfg, &db, json!({"camera_quality": value})).is_err());
        }
        post(&mut cfg, &db, json!({"camera_quality": 0})).unwrap();
        post(&mut cfg, &db, json!({"camera_quality": 12})).unwrap();
        assert_eq!(cfg.camera_quality, 12);
    }

    #[test]
    fn alert_zones_roundtrip_and_validate() {
        let (_d, mut cfg, db) = setup();
        post(
            &mut cfg,
            &db,
            json!({"alert_rects": [[0.1, 0.2, 0.3, 0.4]]}),
        )
        .unwrap();
        assert_eq!(load_motion(&db).alert_rects, vec![vec![0.1, 0.2, 0.3, 0.4]]);
        assert_eq!(
            post(&mut cfg, &db, json!({"alert_rects": [[0.1, 0.2, 0.3, 2]]})).err(),
            Some(400)
        );
        assert_eq!(load_motion(&db).alert_rects, vec![vec![0.1, 0.2, 0.3, 0.4]]);
    }

    #[test]
    fn validation_messages_match_python() {
        let cases = [
            (
                "post_roll_s",
                json!(9999),
                "post_roll_s must be between 3 and 120",
            ),
            (
                "max_disk_gb",
                json!(0.1),
                "max_disk_gb must be between 0.5 and 500",
            ),
            (
                "detect_confidence",
                json!(0.99),
                "detect_confidence must be between 0.2 and 0.9",
            ),
            ("pre_roll_s", json!("x"), "pre_roll_s must be a number"),
            ("pre_roll_s", json!(true), "pre_roll_s must be a number"),
            (
                "detection",
                json!("maybe"),
                "detection must be one of off, alert, strict",
            ),
            (
                "camera_framesize",
                json!(640),
                "camera_framesize must be one of 0, 5, 8, 9, 10",
            ),
            (
                "detect_labels",
                json!([]),
                "detect_labels must list between 1 and 12 known object names",
            ),
            ("quiet_end", json!("24:00"), "quiet_end must be HH:MM"),
            (
                "notify_offline",
                json!(1),
                "notify_offline must be true or false",
            ),
            ("nope", json!(1), "unknown setting: nope"),
        ];
        for (key, value, message) in cases {
            assert_eq!(validate(key, &value).unwrap_err().message(), message);
        }
        assert_eq!(
            validate("camera_brightness", &json!(1.9)).unwrap(),
            json!(1)
        );
        assert_eq!(validate("camera_framesize", &json!("9")).unwrap(), json!(9));
    }

    #[test]
    fn quiet_hours_wrap_midnight() {
        let mut cfg = Config {
            quiet_enabled: true,
            quiet_start: "23:00".into(),
            quiet_end: "07:00".into(),
            ..Config::default()
        };
        let at = |h, m| clock::mktime(2026, 10, 12, h, m, 0);
        assert!(
            in_quiet_hours(&cfg, at(23, 30))
                && in_quiet_hours(&cfg, at(3, 0))
                && in_quiet_hours(&cfg, at(6, 59))
        );
        assert!(!in_quiet_hours(&cfg, at(7, 0)) && !in_quiet_hours(&cfg, at(12, 0)));
        cfg.quiet_enabled = false;
        assert!(!in_quiet_hours(&cfg, at(3, 0)));
    }

    #[test]
    fn quiet_hours_within_one_day_and_empty_window() {
        let mut cfg = Config {
            quiet_enabled: true,
            quiet_start: "13:00".into(),
            quiet_end: "15:00".into(),
            ..Config::default()
        };
        let at = |h| clock::mktime(2026, 10, 12, h, 0, 0);
        assert!(in_quiet_hours(&cfg, at(14)) && !in_quiet_hours(&cfg, at(16)));
        cfg.quiet_end = "13:00".into();
        assert!(!in_quiet_hours(&cfg, at(13)));
    }

    #[test]
    fn camera_and_detection_keys() {
        assert!(is_camera_key("camera_vflip") && is_camera_key("camera_framesize"));
        assert!(!is_camera_key("camera_enabled") && !is_camera_key("camera_host"));
        assert!(is_detection_key("detect_labels") && !is_detection_key("language"));
    }

    #[test]
    fn integers_from_the_toml_keep_their_type_until_changed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("camd.toml");
        std::fs::write(&path, "push_cooldown_s = 300\nmax_disk_gb = 2.0\n").unwrap();
        let mut cfg = crate::config::load_with_env(&path, &|_| None).unwrap();
        assert_eq!(get(&cfg, "push_cooldown_s").unwrap().to_string(), "300");
        assert_eq!(get(&cfg, "max_disk_gb").unwrap().to_string(), "2.0");
        assert_eq!(get(&cfg, "pre_roll_s").unwrap().to_string(), "3.0");
        set(&mut cfg, "push_cooldown_s", &json!(300.0)).unwrap();
        assert_eq!(get(&cfg, "push_cooldown_s").unwrap().to_string(), "300.0");
        let env = |name: &str| (name == "CAMD_PUSH_COOLDOWN_S").then(|| "120".to_string());
        let cfg = crate::config::load_with_env(&path, &env).unwrap();
        assert_eq!(get(&cfg, "push_cooldown_s").unwrap().to_string(), "120");
        let env = |name: &str| (name == "CAMD_PUSH_COOLDOWN_S").then(|| "90.5".to_string());
        let cfg = crate::config::load_with_env(&path, &env).unwrap();
        assert_eq!(get(&cfg, "push_cooldown_s").unwrap().to_string(), "90.5");
    }

    #[test]
    fn snapshot_lists_every_live_key() {
        let snap = snapshot(&Config::default());
        assert_eq!(snap.len(), SPEC.len());
        assert_eq!(snap["detect_labels"], json!(["person", "cat", "dog"]));
        assert_eq!(snap["quiet_end"], json!("07:00"));
    }
}
