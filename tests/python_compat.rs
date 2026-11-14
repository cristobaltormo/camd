use std::path::{Path, PathBuf};
use std::sync::Arc;

use camd::arming::Arming;
use camd::auth::Auth;
use camd::config::Config;
use camd::db::{Database, EventDone, EventFilter, SCHEMA_VERSION};
use camd::prefs;
use camd::presence::Presence;
use serde_json::{Value, json};

fn fixture() -> Value {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python-2.2.0.json"),
    )
    .unwrap();
    serde_json::from_str(&text).unwrap()
}

fn data_dir() -> (Option<tempfile::TempDir>, PathBuf) {
    match std::env::var_os("CAMD_COMPAT_OUT") {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            (None, dir)
        }
        None => {
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().to_path_buf();
            (Some(tmp), path)
        }
    }
}

fn python_database(dir: &Path) -> (Config, Arc<Database>) {
    let cfg = Config {
        rp_id: "localhost".into(),
        origin: "http://localhost:8180".into(),
        ..Config::with_data_dir(dir)
    };
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python-2.2.0.sqlite3"),
        cfg.db_path(),
    )
    .unwrap();
    let db = Arc::new(Database::open(&cfg.db_path()).unwrap());
    (cfg, db)
}

#[test]
fn a_database_written_by_python_is_read_in_full() {
    let fixture = fixture();
    let (_keep, dir) = data_dir();
    let (mut cfg, db) = python_database(&dir);
    assert_eq!(db.user_version().unwrap(), SCHEMA_VERSION);

    let auth = Auth::new(&cfg, Arc::clone(&db));
    assert!(auth.has_password().unwrap() && auth.has_credentials().unwrap());
    assert!(
        auth.login_password(fixture["password"].as_str().unwrap(), "rust")
            .is_ok()
    );
    assert!(auth.login_password("wrong password!", "rust").is_err());
    let session = auth
        .check_session(fixture["session_token"].as_str())
        .unwrap();
    assert_eq!(session, Some(2));
    auth.check_code(fixture["enroll_code"].as_str().unwrap())
        .unwrap();
    let credentials = auth.credentials().unwrap();
    assert_eq!(credentials.len(), 1);
    assert_eq!(
        credentials[0].id,
        fixture["credential_id"].as_str().unwrap()
    );
    assert_eq!(credentials[0].name, "python phone");
    let stored = db.credential(&credentials[0].id).unwrap().unwrap();
    assert_eq!(stored.sign_count, 0);
    assert_eq!(stored.public_key[0], 0xa5);
    assert!(
        auth.sessions()
            .unwrap()
            .iter()
            .any(|s| s.user_agent == "fixture agent")
    );

    prefs::load_stored(&mut cfg, &db).unwrap();
    assert_eq!(
        (cfg.retention_days, cfg.post_roll_s, cfg.language.as_str()),
        (9, 25.0, "es")
    );
    assert_eq!(cfg.detect_labels, vec!["person", "dog"]);
    let motion = prefs::load_motion(&db);
    assert_eq!(
        (motion.sensitivity, motion.ignore_rects.clone()),
        (7, vec![vec![0.1, 0.1, 0.2, 0.2]])
    );
    let arming = Arming::new(Arc::clone(&db), Presence::new(vec![], 300.0));
    assert_eq!(arming.mode(), "armed");
    assert_eq!(arming.decision(), (true, "manual"));

    let events = db
        .list_events(&EventFilter {
            limit: 50,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[1].to_json(),
        json!({"id": 1, "start": 1700000000.5, "end": 1700000012.25, "duration": 11.75, "status": "done", "score": 0.042,
               "size": 123456, "kind": "motion", "keep": false, "analyzed": true, "labels": ["person", "cat"],
               "detections": [{"label": "person", "confidence": 0.912, "box": [0.1, 0.2, 0.3, 0.4]}]})
    );
    assert_eq!((events[0].kind.as_str(), events[0].keep), ("manual", true));
    let by_label = db
        .list_events(&EventFilter {
            limit: 50,
            label: Some("cat".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(by_label.len(), 1);
    assert_eq!(
        db.list_photos(10, None).unwrap()[0].to_json(),
        json!({"id": 1, "ts": 1700000200.75, "size": 2048})
    );
    assert_eq!(db.push_count().unwrap(), 1);
}

#[test]
fn rust_writes_what_python_reads() {
    let (_keep, dir) = data_dir();
    let (mut cfg, db) = python_database(&dir);
    let auth = Auth::new(&cfg, Arc::clone(&db));
    auth.set_password("rust made this password").unwrap();
    let token = auth.create_session("rust agent").unwrap();
    let code = auth.new_enroll_code().unwrap();
    let applied = prefs::apply_settings(
        &mut cfg,
        &db,
        &prefs::parse_settings_body(&json!({"pre_roll_s": 2, "quiet_enabled": true, "sensitivity": 3, "alert_rects": [[0.0, 0.0, 0.5, 0.5]]}))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(applied.changed.len(), 2);
    let arming = Arming::new(Arc::clone(&db), Presence::new(vec![], 300.0));
    arming.set_mode("disarmed").unwrap();
    arming.snooze(10.0).unwrap();
    let id = db
        .insert_event(1700001000.0, "motion", false, true)
        .unwrap();
    db.set_event_labels(id, &["dog".to_string()]).unwrap();
    db.finish_event(
        id,
        &EventDone {
            end_ts: 1700001009.5,
            score: 0.5,
            clip_path: "/c/x.mp4".into(),
            thumb_path: "/c/x.jpg".into(),
            size_bytes: 99,
            labels: vec!["dog".into()],
            detections: json!([{"label": "dog", "confidence": 0.7, "box": [0.0, 0.0, 1.0, 1.0]}]),
        },
    )
    .unwrap();
    drop(auth);
    std::fs::write(
        dir.join("rust-written.json"),
        serde_json::to_string_pretty(
            &json!({"session_token": token, "enroll_code": code, "event_id": id}),
        )
        .unwrap(),
    )
    .unwrap();
    let reopened = Database::open(&cfg.db_path()).unwrap();
    assert_eq!(
        reopened.event(id).unwrap().unwrap().label_list(),
        vec!["dog"]
    );
}
