mod common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use camd::config::Config;
use camd::db::{Database, EventRow};
use camd::detect::{Detection, FakeDetector, Loader};
use camd::runtime::Runtime;
use common::{FakeAlerts, FakeCamera, scene, wait_for};

const FPS: f64 = 25.0;

fn frames() -> Vec<Vec<u8>> {
    let quiet = scene(0, 90, false);
    let mut frames = vec![quiet.clone(); 30];
    frames.extend((0..40).map(|k| scene(k * 4, 90, true)));
    frames.extend(vec![quiet; 90]);
    frames
}

fn runtime(dir: &Path, camera: &FakeCamera, detection: &str, loader: Loader) -> Arc<Runtime> {
    let mut cfg = Config::with_data_dir(dir);
    cfg.camera_host = "127.0.0.1".into();
    cfg.camera_api_port = camera.api_port as i64;
    cfg.camera_stream_port = camera.stream_port as i64;
    cfg.camera_token = "t".into();
    cfg.detection = detection.into();
    cfg.post_roll_s = 3.0;
    cfg.pre_roll_s = 1.0;
    let db = Arc::new(Database::open(&cfg.db_path()).unwrap());
    let rt =
        Runtime::with_loader(cfg.shared(), db, Arc::new(FakeAlerts::default()), loader).unwrap();
    rt.arming().set_mode("armed").unwrap();
    rt
}

fn finished(rt: &Runtime) -> Option<EventRow> {
    rt.db()
        .list_events(&camd::db::EventFilter {
            limit: 10,
            ..Default::default()
        })
        .unwrap()
        .into_iter()
        .find(|e| e.status == "done")
}

fn run_until_done(rt: &Arc<Runtime>) -> Option<EventRow> {
    rt.start();
    wait_for(30.0, || finished(rt).is_some());
    rt.stop();
    finished(rt)
}

#[test]
fn motion_from_a_real_stream_becomes_a_labelled_clip() {
    let dir = tempfile::tempdir().unwrap();
    let camera = FakeCamera::start(frames(), "t", FPS);
    let person = Detection::new("person", 0.9, [0.3, 0.3, 0.3, 0.5]);
    let rt = runtime(
        dir.path(),
        &camera,
        "alert",
        FakeDetector::loader(vec![person]),
    );
    let row = run_until_done(&rt).expect("an event");
    assert_eq!(row.labels, "[\"person\"]");
    assert!(row.confirmed && row.analyzed);
    assert!(std::fs::metadata(row.clip_path.unwrap()).unwrap().len() > 1000);
    assert!(Path::new(&row.thumb_path.unwrap()).exists());
    let controls = camera.controls.lock().unwrap().clone();
    assert!(
        controls.iter().any(|c| c.starts_with("var=vflip")),
        "{controls:?}"
    );
}

#[test]
fn without_a_detector_every_motion_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let camera = FakeCamera::start(frames(), "t", FPS);
    let rt = runtime(dir.path(), &camera, "off", FakeDetector::loader(Vec::new()));
    let row = run_until_done(&rt).expect("an event");
    assert!(!row.analyzed);
    assert_eq!(row.labels, "[]");
}

#[test]
fn privacy_puts_the_camera_in_standby_and_back() {
    let dir = tempfile::tempdir().unwrap();
    let camera = FakeCamera::start(vec![scene(0, 90, false)], "t", FPS);
    let rt = runtime(dir.path(), &camera, "off", FakeDetector::loader(Vec::new()));
    rt.start();
    assert!(wait_for(10.0, || rt.fresh_frame().is_some()));
    let mut live = rt.frames();
    live.mark_unchanged();
    assert!(wait_for(5.0, || live.has_changed().unwrap_or(false)));
    assert!(rt.set_privacy(true));
    assert!(camera.standby.load(Ordering::SeqCst));
    assert!(rt.fresh_frame().is_none() && !rt.live_available());
    assert_eq!(
        rt.status("3.0.0", "dev")["privacy"],
        serde_json::json!(true)
    );
    assert!(rt.set_privacy(false));
    assert!(wait_for(10.0, || rt.fresh_frame().is_some()));
    rt.stop();
}

#[test]
fn the_grabber_reconnects_after_the_stream_drops() {
    let dir = tempfile::tempdir().unwrap();
    let camera = FakeCamera::start(vec![scene(0, 90, false)], "t", FPS);
    camera.drop_after.store(10, Ordering::SeqCst);
    let rt = runtime(dir.path(), &camera, "off", FakeDetector::loader(Vec::new()));
    rt.start();
    assert!(wait_for(15.0, || camera.streams.load(Ordering::SeqCst)
        >= 2
        && rt.fresh_frame().is_some()));
    let status = rt.status("3.0.0", "dev");
    assert_eq!(status["camera"]["online"], serde_json::json!(true));
    rt.stop();
}

#[test]
fn a_wrong_token_keeps_the_camera_offline() {
    let dir = tempfile::tempdir().unwrap();
    let camera = FakeCamera::start(vec![scene(0, 90, false)], "other", FPS);
    let rt = runtime(dir.path(), &camera, "off", FakeDetector::loader(Vec::new()));
    rt.start();
    assert!(wait_for(5.0, || rt.grabber().failures() >= 1));
    assert!(rt.fresh_frame().is_none());
    let started = std::time::Instant::now();
    rt.stop();
    assert!(started.elapsed().as_secs_f64() < 3.0);
}
