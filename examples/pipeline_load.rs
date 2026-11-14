use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use camd::config::Config;
use camd::db::Database;
use camd::detect::{Detector, NanoDet};
use camd::notify::Alerts;
use camd::runtime::Runtime;

struct Quiet;

impl Alerts for Quiet {
    fn send(&self, _: &str, _: &str, _: &str, _: &str) -> usize {
        0
    }
    fn push_subscriptions(&self) -> i64 {
        0
    }
    fn channel_names(&self) -> Vec<String> {
        Vec::new()
    }
}

fn status_kb(field: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find(|l| l.starts_with(field))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn cpu_ticks() -> u64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let fields: Vec<&str> = stat
        .rsplit(')')
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect();
    fields[11].parse::<u64>().unwrap_or(0) + fields[12].parse::<u64>().unwrap_or(0)
}

fn bench(model: &str, image: &str, rounds: usize) {
    let mut detector = NanoDet::load(&PathBuf::from(model)).unwrap();
    let jpeg = std::fs::read(image).unwrap();
    let labels = vec!["person".to_string(), "cat".to_string(), "dog".to_string()];
    let found = detector.detect(&jpeg, &labels, 0.5).unwrap();
    let started = Instant::now();
    for _ in 0..rounds {
        detector.detect(&jpeg, &labels, 0.5).unwrap();
    }
    let ms = started.elapsed().as_secs_f64() * 1000.0 / rounds as f64;
    println!(
        "{{\"per_detection_ms\": {ms:.1}, \"rss_kb\": {}, \"found\": {}}}",
        status_kb("VmRSS:"),
        found.len()
    );
}

fn motion_bench(folder: &str) {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(folder)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    paths.sort();
    let frames: Vec<Vec<u8>> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
    let kernel = camd::imaging::gaussian_kernel(21);
    let rounds = 5;
    let mut timings = [0f64; 3];
    let mut detector = camd::motion::MotionDetector::new(Default::default());
    for _ in 0..rounds {
        for frame in &frames {
            let started = Instant::now();
            let gray = camd::imaging::decode_gray_half(frame).unwrap();
            timings[0] += started.elapsed().as_secs_f64();
            let started = Instant::now();
            let _ = camd::imaging::gaussian_blur(&gray, &kernel);
            timings[1] += started.elapsed().as_secs_f64();
            let started = Instant::now();
            detector.process(frame);
            timings[2] += started.elapsed().as_secs_f64();
        }
    }
    let gray = camd::imaging::gaussian_blur(
        &camd::imaging::decode_gray_half(&frames[10]).unwrap(),
        &kernel,
    );
    let background: Vec<f32> = camd::imaging::gaussian_blur(
        &camd::imaging::decode_gray_half(&frames[0]).unwrap(),
        &kernel,
    )
    .pixels
    .iter()
    .map(|v| *v as f32)
    .collect();
    let started = Instant::now();
    let mut gain = 0.0;
    for _ in 0..100 {
        gain = camd::motion::median_difference(&gray.pixels, &background);
    }
    let median_ms = started.elapsed().as_secs_f64() * 10.0;
    let changed: Vec<bool> = gray
        .pixels
        .iter()
        .zip(&background)
        .map(|(v, b)| v.abs_diff((b + gain).clamp(0.0, 255.0) as u8) > 25)
        .collect();
    let started = Instant::now();
    let mut grown = Vec::new();
    for _ in 0..100 {
        grown = camd::motion::dilate(&changed, gray.width, gray.height, 2);
    }
    let dilate_ms = started.elapsed().as_secs_f64() * 10.0;
    let started = Instant::now();
    let mut area = 0.0;
    for _ in 0..100 {
        area = camd::motion::largest_contour_area(&grown, gray.width, gray.height);
    }
    let contour_ms = started.elapsed().as_secs_f64() * 10.0;
    println!(
        "{{\"median_ms\": {median_ms:.2}, \"dilate_ms\": {dilate_ms:.2}, \"contour_ms\": {contour_ms:.2}, \"area\": {area}}}"
    );
    let n = (rounds * frames.len()) as f64;
    println!(
        "{{\"decode_ms\": {:.2}, \"blur_ms\": {:.2}, \"process_ms\": {:.2}}}",
        timings[0] * 1000.0 / n,
        timings[1] * 1000.0 / n,
        timings[2] * 1000.0 / n
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("gray") {
        let gray = camd::imaging::decode_gray_half(&std::fs::read(&args[2]).unwrap()).unwrap();
        let blurred = camd::imaging::gaussian_blur(&gray, &camd::imaging::gaussian_kernel(21));
        let mut out = gray.pixels.clone();
        out.extend_from_slice(&blurred.pixels);
        std::fs::write(&args[3], out).unwrap();
        println!("{} {}", gray.width, gray.height);
        return;
    }
    if args.get(1).map(String::as_str) == Some("motion") {
        return motion_bench(&args[2]);
    }
    if args.get(1).map(String::as_str) == Some("bench") {
        return bench(&args[2], &args[3], args[4].parse().unwrap());
    }
    let [
        _,
        data_dir,
        stream_port,
        api_port,
        detection,
        warmup,
        seconds,
    ] = args.as_slice()
    else {
        eprintln!("usage: pipeline_load DATA_DIR STREAM_PORT API_PORT off|alert WARMUP_S SECONDS");
        std::process::exit(2);
    };
    let mut cfg = Config::with_data_dir(data_dir);
    cfg.camera_host = "127.0.0.1".into();
    cfg.camera_stream_port = stream_port.parse().unwrap();
    cfg.camera_api_port = api_port.parse().unwrap();
    cfg.camera_token = "test".into();
    cfg.detection = detection.clone();
    let db = Arc::new(Database::open(&cfg.db_path()).unwrap());
    let rt = Runtime::new(cfg.shared(), db, Arc::new(Quiet)).unwrap();
    rt.arming().set_mode("armed").unwrap();
    rt.start();
    std::thread::sleep(Duration::from_secs_f64(warmup.parse().unwrap()));
    let (ticks, started) = (cpu_ticks(), Instant::now());
    let mut peak = 0;
    let window: f64 = seconds.parse().unwrap();
    while started.elapsed().as_secs_f64() < window {
        std::thread::sleep(Duration::from_millis(500));
        peak = peak.max(status_kb("VmRSS:"));
    }
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    let cpu = (cpu_ticks() - ticks) as f64 / hz / started.elapsed().as_secs_f64() * 100.0;
    let status = rt.status("", "");
    println!(
        "{{\"detection\": \"{detection}\", \"rss_kb\": {}, \"rss_peak_kb\": {peak}, \"cpu_percent\": {cpu:.2}, \"fps\": {}, \"inference_ms\": {}, \"state\": {}, \"events\": {}}}",
        status_kb("VmRSS:"),
        status["camera"]["fps"],
        status["detection"]["inference_ms"],
        status["detection"]["state"],
        status["storage"]["events"],
    );
    rt.stop();
}
