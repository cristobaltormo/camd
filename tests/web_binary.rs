use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

const ORIGIN: &str = "http://localhost:8180";

fn jpeg(shade: u8) -> Vec<u8> {
    let pixels: Vec<u8> = (0..320 * 240)
        .flat_map(|i| [shade, (i % 200) as u8, ((i / 320) % 250) as u8])
        .collect();
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, 80)
        .encode(&pixels, 320, 240, jpeg_encoder::ColorType::Rgb)
        .unwrap();
    out
}

fn fake_camera(stop: Arc<AtomicBool>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || serve_camera(stream, &stop));
        }
    });
    port
}

fn serve_camera(mut stream: TcpStream, stop: &AtomicBool) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request = String::new();
    if reader.read_line(&mut request).is_err() {
        return;
    }
    let mut authorized = false;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
            break;
        }
        if line
            .to_ascii_lowercase()
            .starts_with("authorization: bearer test")
        {
            authorized = true;
        }
    }
    if !authorized {
        let _ = stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n");
        return;
    }
    let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
    if path.starts_with("/stream") {
        let _ = stream.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace;boundary=frame\r\n\r\n",
        );
        let mut shade = 0u8;
        while !stop.load(Ordering::Relaxed) {
            let frame = jpeg(shade);
            shade = shade.wrapping_add(1);
            let head = format!(
                "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                frame.len()
            );
            if stream.write_all(head.as_bytes()).is_err()
                || stream.write_all(&frame).is_err()
                || stream.write_all(b"\r\n").is_err()
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    } else {
        let body = if path.starts_with("/status") {
            r#"{"rssi": -51, "fw": "fake", "uptime_s": 12, "standby": false}"#
        } else {
            "ok"
        };
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Server {
    child: Child,
    base: String,
    cookie: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .new_agent()
}

impl Server {
    fn start(dir: &Path, camera_port: u16) -> Server {
        let port = free_port();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python-2.2.0.sqlite3"),
            dir.join("camd.sqlite3"),
        )
        .unwrap();
        let config = dir.join("config.toml");
        std::fs::write(
            &config,
            format!(
                "data_dir = \"{}\"\nlisten_port = {port}\ncamera_host = \"127.0.0.1\"\ncamera_api_port = {camera_port}\ncamera_stream_port = {camera_port}\ncamera_token = \"test\"\ndetection = \"off\"\norigin = \"{ORIGIN}\"\n",
                dir.display()
            ),
        )
        .unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_camd"))
            .env("CAMD_CONFIG", &config)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let server = Server {
            child,
            base: format!("http://127.0.0.1:{port}"),
            cookie: String::new(),
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        while agent()
            .get(format!("{}/healthz", server.base))
            .call()
            .is_err()
        {
            assert!(Instant::now() < deadline, "server did not start");
            std::thread::sleep(Duration::from_millis(100));
        }
        server
    }

    fn get(&self, path: &str) -> (u16, Vec<u8>) {
        let mut response = agent()
            .get(format!("{}{path}", self.base))
            .header("Cookie", &self.cookie)
            .call()
            .unwrap();
        let status = response.status().as_u16();
        (status, response.body_mut().read_to_vec().unwrap())
    }

    fn json(&self, path: &str) -> Value {
        serde_json::from_slice(&self.get(path).1).unwrap()
    }

    fn post(&mut self, path: &str, body: Value) -> (u16, Value) {
        let mut response = agent()
            .post(format!("{}{path}", self.base))
            .header("Origin", ORIGIN)
            .header("Cookie", &self.cookie)
            .send_json(body)
            .unwrap();
        if let Some(cookie) = response.headers().get("set-cookie") {
            let text = cookie.to_str().unwrap();
            self.cookie = text.split(';').next().unwrap().to_string();
        }
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().unwrap();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }
}

#[test]
fn the_binary_serves_a_live_camera_and_stops_quickly() {
    let dir = tempfile::tempdir().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let camera_port = fake_camera(Arc::clone(&stop));
    let mut server = Server::start(dir.path(), camera_port);

    assert_eq!(server.get("/api/status").0, 401);
    let (status, _) = server.post(
        "/api/auth/login/password",
        serde_json::json!({"password": "correct horse battery"}),
    );
    assert_eq!(status, 200);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let status = server.json("/api/status");
        if status["camera"]["online"] == true {
            assert_eq!(status["version"], env!("CARGO_PKG_VERSION"));
            assert_eq!(status["mode"], "armed");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "camera never came online: {status}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    let (code, snapshot) = server.get("/api/snapshot");
    assert_eq!(code, 200);
    assert_eq!(&snapshot[..2], &[0xff, 0xd8]);
    let (code, photo) = server.post("/api/photos", serde_json::json!({}));
    assert_eq!(code, 200, "{photo}");
    let id = photo["id"].as_i64().unwrap();
    assert_eq!(server.get(&format!("/api/photos/{id}/thumb")).0, 200);
    assert_eq!(server.json("/api/settings")["post_roll_s"], 25.0);

    let host = server.base.trim_start_matches("http://").to_string();
    let mut live = TcpStream::connect(&host).unwrap();
    write!(
        live,
        "GET /api/live HTTP/1.1\r\nHost: {host}\r\nCookie: {}\r\n\r\n",
        server.cookie
    )
    .unwrap();
    live.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut seen = Vec::new();
    let mut buffer = [0u8; 8192];
    while seen.windows(7).filter(|w| *w == b"--frame").count() < 2 {
        let n = live.read(&mut buffer).unwrap();
        assert!(n > 0, "live stream closed early");
        seen.extend_from_slice(&buffer[..n]);
    }
    let head = String::from_utf8_lossy(&seen[..200.min(seen.len())]).to_string();
    assert!(head.starts_with("HTTP/1.1 200 OK"), "{head}");
    assert!(head.contains("multipart/x-mixed-replace; boundary=frame"));

    let started = Instant::now();
    unsafe {
        libc::kill(server.child.id() as i32, libc::SIGTERM);
    }
    let status = loop {
        if let Some(status) = server.child.try_wait().unwrap() {
            break status;
        }
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "shutdown took too long"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "{status:?}");
    assert!(
        started.elapsed() < Duration::from_secs(6),
        "{:?}",
        started.elapsed()
    );
    stop.store(true, Ordering::Relaxed);
}

#[test]
fn ctl_output_into_a_closed_pipe_does_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_camd"))
        .args(["ctl", "devices"])
        .env("CAMD_CONFIG", dir.path().join("missing.toml"))
        .env("CAMD_DATA_DIR", dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let mut errors = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut errors)
        .unwrap();
    let status = child.wait().unwrap();
    assert!(!errors.contains("panicked"), "{errors}");
    assert_ne!(status.code(), Some(101));
}
