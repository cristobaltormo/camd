#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use camd::imaging::{self, Image};
use camd::notify::Alerts;

pub fn scene(shift: usize, brightness: u8, blob: bool) -> Vec<u8> {
    let mut img = Image::filled(800, 600, 3, brightness);
    img.fill_rect(50, 100, 251, 501, brightness.saturating_add(30));
    for k in 0..4 {
        img.fill_rect(
            400 + k * 40,
            255,
            425 + k * 40,
            305,
            brightness.saturating_sub(40),
        );
    }
    if blob {
        img.fill_rect(300 + shift, 200, 521 + shift, 521, 20);
    }
    imaging::encode_jpeg(&img, 90).unwrap()
}

#[derive(Default)]
pub struct FakeAlerts {
    pub sent: Mutex<Vec<String>>,
}

impl Alerts for FakeAlerts {
    fn send(&self, title: &str, _: &str, _: &str, _: &str) -> usize {
        self.sent.lock().unwrap().push(title.to_string());
        1
    }
    fn push_subscriptions(&self) -> i64 {
        0
    }
    fn channel_names(&self) -> Vec<String> {
        Vec::new()
    }
}

pub struct FakeCamera {
    pub api_port: u16,
    pub stream_port: u16,
    pub standby: Arc<AtomicBool>,
    pub streams: Arc<AtomicUsize>,
    pub controls: Arc<Mutex<Vec<String>>>,
    pub drop_after: Arc<AtomicUsize>,
}

struct Shared {
    frames: Vec<Vec<u8>>,
    token: String,
    fps: f64,
    index: AtomicUsize,
    standby: Arc<AtomicBool>,
    streams: Arc<AtomicUsize>,
    controls: Arc<Mutex<Vec<String>>>,
    drop_after: Arc<AtomicUsize>,
}

fn reply(stream: &mut TcpStream, code: u16, body: &[u8], kind: &str) {
    let head = format!(
        "HTTP/1.1 {code} X\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn handle(mut stream: TcpStream, shared: Arc<Shared>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
    let mut authorised = false;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 || header.trim().is_empty() {
            break;
        }
        if header.trim() == format!("Authorization: Bearer {}", shared.token) {
            authorised = true;
        }
    }
    if !authorised {
        return reply(&mut stream, 401, b"unauthorized", "text/plain");
    }
    let (route, query) = path.split_once('?').unwrap_or((&path, ""));
    match route {
        "/status" => {
            let body = format!(
                "{{\"fw\": \"fake\", \"rssi\": -50, \"uptime_s\": 5, \"standby\": {}}}",
                shared.standby.load(Ordering::SeqCst) as i32
            );
            reply(&mut stream, 200, body.as_bytes(), "application/json")
        }
        "/control" => {
            shared.controls.lock().unwrap().push(query.to_string());
            if query.starts_with("var=standby&val=") {
                shared.standby.store(query.ends_with('1'), Ordering::SeqCst);
            }
            reply(&mut stream, 200, b"ok", "text/plain")
        }
        "/stream" => {
            if shared.standby.load(Ordering::SeqCst) {
                return reply(&mut stream, 503, b"standby", "text/plain");
            }
            shared.streams.fetch_add(1, Ordering::SeqCst);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace;boundary=frame\r\nTransfer-Encoding: chunked\r\n\r\n",
            );
            let mut sent = 0usize;
            while !shared.standby.load(Ordering::SeqCst) {
                let limit = shared.drop_after.load(Ordering::SeqCst);
                if limit > 0 && sent >= limit {
                    shared.drop_after.store(0, Ordering::SeqCst);
                    return;
                }
                let i = shared.index.fetch_add(1, Ordering::SeqCst);
                let frame = &shared.frames[i % shared.frames.len()];
                let mut part = format!(
                    "\r\n--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                    frame.len()
                )
                .into_bytes();
                part.extend_from_slice(frame);
                let mut chunk = format!("{:x}\r\n", part.len()).into_bytes();
                chunk.extend_from_slice(&part);
                chunk.extend_from_slice(b"\r\n");
                if stream.write_all(&chunk).is_err() {
                    return;
                }
                sent += 1;
                std::thread::sleep(Duration::from_secs_f64(1.0 / shared.fps));
            }
        }
        _ => reply(&mut stream, 404, b"not found", "text/plain"),
    }
}

fn serve(listener: TcpListener, shared: Arc<Shared>) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || handle(stream, shared));
        }
    });
}

impl FakeCamera {
    pub fn start(frames: Vec<Vec<u8>>, token: &str, fps: f64) -> FakeCamera {
        let standby = Arc::new(AtomicBool::new(false));
        let streams = Arc::new(AtomicUsize::new(0));
        let controls = Arc::new(Mutex::new(Vec::new()));
        let drop_after = Arc::new(AtomicUsize::new(0));
        let shared = Arc::new(Shared {
            frames,
            token: token.to_string(),
            fps,
            index: AtomicUsize::new(0),
            standby: Arc::clone(&standby),
            streams: Arc::clone(&streams),
            controls: Arc::clone(&controls),
            drop_after: Arc::clone(&drop_after),
        });
        let api = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpListener::bind("127.0.0.1:0").unwrap();
        let (api_port, stream_port) = (
            api.local_addr().unwrap().port(),
            stream.local_addr().unwrap().port(),
        );
        serve(api, Arc::clone(&shared));
        serve(stream, shared);
        FakeCamera {
            api_port,
            stream_port,
            standby,
            streams,
            controls,
            drop_after,
        }
    }
}

pub fn wait_for(seconds: f64, condition: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs_f64(seconds);
    while std::time::Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}
