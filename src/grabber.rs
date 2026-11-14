use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

use bytes::Bytes;
use serde_json::Value;
use tokio::sync::watch;

use crate::clock;
use crate::runtime::Frame;

pub const RETRY_STEPS: [f64; 7] = [1.0, 2.0, 3.0, 5.0, 10.0, 20.0, 30.0];
pub const ONLINE_WINDOW_S: f64 = 10.0;
pub const MAX_FRAME_BYTES: usize = 2_000_000;
pub const FPS_SMOOTHING: f64 = 0.1;
pub const STREAM_TIMEOUT_S: f64 = 6.0;
pub const API_TIMEOUT_S: f64 = 10.0;
pub const STATUS_TIMEOUT_S: f64 = 5.0;
pub const IDLE_WAIT_S: f64 = 1.0;
const MAX_LINE_BYTES: u64 = 16 * 1024;
pub const MAX_HEADER_LINES: usize = 100;
const MAX_BODY_BYTES: u64 = 1 << 20;

pub fn retry_delay(failures: u32) -> f64 {
    let index = (failures.max(1) as usize).min(RETRY_STEPS.len()) - 1;
    RETRY_STEPS[index]
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn read_line(reader: &mut impl BufRead, line: &mut Vec<u8>) -> io::Result<usize> {
    line.clear();
    let read = reader.take(MAX_LINE_BYTES).read_until(b'\n', line)?;
    if read as u64 == MAX_LINE_BYTES && line.last() != Some(&b'\n') {
        return Err(invalid("line too long"));
    }
    Ok(read)
}

pub enum Body {
    Length(io::Take<BufReader<TcpStream>>),
    Chunked {
        reader: BufReader<TcpStream>,
        remaining: u64,
        started: bool,
        done: bool,
    },
    Close(BufReader<TcpStream>),
}

impl Body {
    fn next_chunk(reader: &mut BufReader<TcpStream>, started: &mut bool) -> io::Result<u64> {
        let mut line = Vec::new();
        if *started {
            read_line(reader, &mut line)?;
        }
        *started = true;
        if read_line(reader, &mut line)? == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        let text = String::from_utf8_lossy(&line);
        let size = text.split(';').next().unwrap_or("").trim();
        u64::from_str_radix(size, 16).map_err(|_| invalid("bad chunk size"))
    }
}

impl BufRead for Body {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        match self {
            Body::Length(reader) => reader.fill_buf(),
            Body::Close(reader) => reader.fill_buf(),
            Body::Chunked {
                reader,
                remaining,
                started,
                done,
            } => {
                if *done {
                    return Ok(&[]);
                }
                if *remaining == 0 {
                    *remaining = Body::next_chunk(reader, started)?;
                    if *remaining == 0 {
                        *done = true;
                        return Ok(&[]);
                    }
                }
                let available = reader.fill_buf()?;
                if available.is_empty() {
                    return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
                }
                let take = available.len().min(*remaining as usize);
                Ok(&available[..take])
            }
        }
    }

    fn consume(&mut self, amount: usize) {
        match self {
            Body::Length(reader) => reader.consume(amount),
            Body::Close(reader) => reader.consume(amount),
            Body::Chunked {
                reader, remaining, ..
            } => {
                reader.consume(amount);
                *remaining -= amount as u64;
            }
        }
    }
}

impl Read for Body {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let available = self.fill_buf()?;
        let read = available.len().min(buf.len());
        buf[..read].copy_from_slice(&available[..read]);
        self.consume(read);
        Ok(read)
    }
}

pub struct Response {
    pub status: u16,
    pub body: Body,
}

pub struct CameraClient {
    pub host: String,
    pub token: String,
    pub api_port: u16,
    pub stream_port: u16,
}

impl CameraClient {
    pub fn new(host: &str, token: &str, api_port: i64, stream_port: i64) -> CameraClient {
        CameraClient {
            host: host.to_string(),
            token: token.to_string(),
            api_port: u16::try_from(api_port).unwrap_or(80),
            stream_port: u16::try_from(stream_port).unwrap_or(81),
        }
    }

    fn connect(&self, port: u16, timeout: Duration) -> io::Result<TcpStream> {
        let mut last = io::Error::new(io::ErrorKind::NotFound, "camera host did not resolve");
        for addr in (self.host.as_str(), port).to_socket_addrs()? {
            match TcpStream::connect_timeout(&addr, timeout) {
                Ok(stream) => {
                    stream.set_read_timeout(Some(timeout))?;
                    stream.set_write_timeout(Some(timeout))?;
                    let _ = stream.set_nodelay(true);
                    return Ok(stream);
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    pub fn request(
        &self,
        port: u16,
        path: &str,
        timeout: Duration,
        socket: Option<&Mutex<Option<TcpStream>>>,
    ) -> io::Result<Response> {
        let mut stream = self.connect(port, timeout)?;
        if let Some(slot) = socket {
            *lock(slot) = Some(stream.try_clone()?);
        }
        let host = if port == 80 {
            self.host.clone()
        } else {
            format!("{}:{port}", self.host)
        };
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}\r\nAccept-Encoding: identity\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n",
            self.token
        );
        stream.write_all(request.as_bytes())?;
        let mut reader = BufReader::with_capacity(64 * 1024, stream);
        let mut line = Vec::new();
        if read_line(&mut reader, &mut line)? == 0 {
            return Err(invalid("remote end closed connection without response"));
        }
        let status_line = String::from_utf8_lossy(&line).to_string();
        let mut parts = status_line.split_whitespace();
        let status = match (parts.next(), parts.next()) {
            (Some(version), Some(code)) if version.starts_with("HTTP/") => code
                .parse::<u16>()
                .map_err(|_| invalid(format!("bad status line: {}", status_line.trim())))?,
            _ => return Err(invalid(format!("bad status line: {}", status_line.trim()))),
        };
        let mut length: Option<u64> = None;
        let mut chunked = false;
        for count in 0.. {
            if count >= MAX_HEADER_LINES {
                return Err(invalid("too many header lines"));
            }
            if read_line(&mut reader, &mut line)? == 0 {
                return Err(invalid("stream closed"));
            }
            let text = String::from_utf8_lossy(&line);
            let text = text.trim();
            if text.is_empty() {
                break;
            }
            if let Some((name, value)) = text.split_once(':') {
                let (name, value) = (name.trim().to_ascii_lowercase(), value.trim());
                if name == "content-length" {
                    length = value.parse().ok();
                } else if name == "transfer-encoding"
                    && value.to_ascii_lowercase().contains("chunked")
                {
                    chunked = true;
                }
            }
        }
        let body = if chunked {
            Body::Chunked {
                reader,
                remaining: 0,
                started: false,
                done: false,
            }
        } else if let Some(n) = length {
            Body::Length(reader.take(n))
        } else {
            Body::Close(reader)
        };
        Ok(Response { status, body })
    }

    pub fn get(&self, path: &str, timeout: Duration) -> io::Result<(u16, Vec<u8>)> {
        let response = self.request(self.api_port, path, timeout, None)?;
        let mut body = Vec::new();
        response.body.take(MAX_BODY_BYTES).read_to_end(&mut body)?;
        Ok((response.status, body))
    }

    pub fn set_standby(&self, on: bool) -> bool {
        match self.get(
            &format!("/control?var=standby&val={}", on as i32),
            Duration::from_secs_f64(API_TIMEOUT_S),
        ) {
            Ok((status, _)) => status == 200,
            Err(e) => {
                crate::warning!("camd.grabber", "standby request failed: {e}");
                false
            }
        }
    }

    pub fn apply_controls(&self, controls: &[(&str, i64)]) -> Vec<(String, bool)> {
        controls
            .iter()
            .map(|(name, value)| {
                let ok = match self.get(
                    &format!("/control?var={name}&val={value}"),
                    Duration::from_secs_f64(API_TIMEOUT_S),
                ) {
                    Ok((status, _)) => status == 200,
                    Err(e) => {
                        crate::warning!("camd.grabber", "control {name} failed: {e}");
                        false
                    }
                };
                (name.to_string(), ok)
            })
            .collect()
    }

    pub fn status(&self) -> Option<Value> {
        let (status, body) = self
            .get("/status", Duration::from_secs_f64(STATUS_TIMEOUT_S))
            .ok()?;
        if status != 200 {
            return None;
        }
        serde_json::from_slice(&body).ok()
    }
}

pub fn next_frame_length(reader: &mut impl BufRead) -> io::Result<usize> {
    let mut line = Vec::new();
    for count in 0.. {
        if count >= MAX_HEADER_LINES {
            return Err(invalid("no frame boundary"));
        }
        if read_line(reader, &mut line)? == 0 {
            return Err(invalid("stream closed"));
        }
        if line.trim_ascii() == b"--frame" {
            break;
        }
    }
    let mut length: Option<i64> = None;
    for count in 0.. {
        if count >= MAX_HEADER_LINES {
            return Err(invalid("too many frame header lines"));
        }
        if read_line(reader, &mut line)? == 0 {
            return Err(invalid("stream closed"));
        }
        let text = line.trim_ascii();
        if text.is_empty() {
            break;
        }
        if text.len() >= 15 && text[..15].eq_ignore_ascii_case(b"content-length:") {
            let value = String::from_utf8_lossy(&text[15..]).trim().to_string();
            length = Some(value.parse().map_err(|_| invalid("bad frame length"))?);
        }
    }
    match length {
        Some(n) if n > 0 && n as usize <= MAX_FRAME_BYTES => Ok(n as usize),
        _ => Err(invalid("bad frame length")),
    }
}

pub trait FrameSink: Send + Sync {
    fn on_frame(&self, ts: f64, jpeg: &Bytes);
    fn on_connect(&self);
}

#[derive(Default)]
struct Counters {
    fps: f64,
    failures: u32,
}

pub struct Grabber {
    pub camera: CameraClient,
    counters: Mutex<Counters>,
    enabled: AtomicBool,
    standby: AtomicBool,
    halt: AtomicBool,
    kicked: Mutex<bool>,
    kick: Condvar,
    frames: watch::Sender<Option<Frame>>,
    socket: Mutex<Option<TcpStream>>,
}

impl Grabber {
    pub fn new(camera: CameraClient) -> Grabber {
        Grabber {
            camera,
            counters: Mutex::new(Counters::default()),
            enabled: AtomicBool::new(true),
            standby: AtomicBool::new(false),
            halt: AtomicBool::new(false),
            kicked: Mutex::new(false),
            kick: Condvar::new(),
            frames: watch::Sender::new(None),
            socket: Mutex::new(None),
        }
    }

    pub fn subscribe(&self) -> watch::Receiver<Option<Frame>> {
        self.frames.subscribe()
    }

    pub fn latest(&self) -> Option<Frame> {
        self.frames.borrow().clone()
    }

    pub fn latest_ts(&self) -> Option<f64> {
        self.frames.borrow().as_ref().map(|f| f.ts)
    }

    pub fn online(&self) -> bool {
        self.enabled()
            && self
                .latest_ts()
                .is_some_and(|ts| clock::now() - ts < ONLINE_WINDOW_S)
    }

    pub fn standby(&self) -> bool {
        self.standby.load(Ordering::SeqCst)
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    pub fn fps(&self) -> f64 {
        lock(&self.counters).fps
    }

    pub fn failures(&self) -> u32 {
        lock(&self.counters).failures
    }

    fn clear(&self) {
        self.frames.send_if_modified(|latest| {
            *latest = None;
            false
        });
        lock(&self.counters).fps = 0.0;
    }

    fn drop_connection(&self) {
        if let Some(socket) = lock(&self.socket).take() {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }

    fn kick(&self) {
        *lock(&self.kicked) = true;
        self.kick.notify_all();
    }

    fn wait_kick(&self, seconds: f64) -> bool {
        let guard = lock(&self.kicked);
        let (mut guard, _) = self
            .kick
            .wait_timeout_while(guard, Duration::from_secs_f64(seconds), |k| !*k)
            .unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *guard)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::SeqCst);
        if on {
            lock(&self.counters).failures = 0;
            self.kick();
        } else {
            self.clear();
            self.drop_connection();
        }
    }

    pub fn mark_standby(&self, on: bool) {
        self.standby.store(on, Ordering::SeqCst);
        if on {
            self.clear();
            self.drop_connection();
        } else {
            self.kick();
        }
    }

    pub fn set_standby(&self, on: bool) -> bool {
        if !self.camera.set_standby(on) {
            return false;
        }
        self.mark_standby(on);
        true
    }

    pub fn stop(&self) {
        self.halt.store(true, Ordering::SeqCst);
        self.kick();
        self.drop_connection();
    }

    pub fn halted(&self) -> bool {
        self.halt.load(Ordering::SeqCst)
    }

    fn streaming(&self) -> bool {
        !self.halted() && !self.standby() && self.enabled()
    }

    pub fn publish(&self, ts: f64, jpeg: Bytes) -> bool {
        if !(self.enabled() && !self.standby()) {
            return false;
        }
        if let Some(previous) = self.latest_ts()
            && ts > previous
        {
            let mut counters = lock(&self.counters);
            counters.fps += FPS_SMOOTHING * (1.0 / (ts - previous) - counters.fps);
        }
        self.frames.send_replace(Some(Frame { ts, jpeg }));
        true
    }

    pub fn run(&self, sink: &dyn FrameSink) {
        while !self.halted() {
            if self.standby() || !self.enabled() {
                self.wait_kick(IDLE_WAIT_S);
                continue;
            }
            match self.consume(sink) {
                Ok(()) => lock(&self.counters).failures = 0,
                Err(e) => {
                    let failures = {
                        let mut counters = lock(&self.counters);
                        counters.failures += 1;
                        counters.failures
                    };
                    if !self.halted() && (failures == 1 || failures == 5 || failures % 20 == 0) {
                        crate::warning!(
                            "camd.grabber",
                            "camera unreachable ({failures} failures): {e}"
                        );
                    }
                }
            }
            if self.halted() {
                break;
            }
            let delay = retry_delay(self.failures());
            if self.wait_kick(delay) {
                lock(&self.counters).failures = 0;
            }
        }
    }

    fn consume(&self, sink: &dyn FrameSink) -> io::Result<()> {
        let result = self.consume_stream(sink);
        lock(&self.socket).take();
        result
    }

    fn consume_stream(&self, sink: &dyn FrameSink) -> io::Result<()> {
        let response = self.camera.request(
            self.camera.stream_port,
            "/stream",
            Duration::from_secs_f64(STREAM_TIMEOUT_S),
            Some(&self.socket),
        )?;
        if response.status != 200 {
            return Err(invalid(format!("stream status {}", response.status)));
        }
        crate::info!("camd.grabber", "stream connected");
        sink.on_connect();
        let mut reader = response.body;
        while self.streaming() {
            let length = next_frame_length(&mut reader)?;
            let mut data = vec![0u8; length];
            reader
                .read_exact(&mut data)
                .map_err(|_| invalid("short frame"))?;
            let data = Bytes::from(data);
            let ts = clock::now();
            if self.publish(ts, data.clone()) {
                sink.on_frame(ts, &data);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn retry_backs_off_to_thirty_seconds() {
        let delays: Vec<f64> = (0..12).map(retry_delay).collect();
        assert_eq!(delays[0], 1.0);
        assert_eq!(delays[1], 1.0);
        assert!(delays.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(delays.iter().cloned().fold(0.0, f64::max), 30.0);
        assert_eq!(delays[4], 5.0);
    }

    #[test]
    fn frame_headers_are_parsed_and_checked() {
        let mut ok = Cursor::new(
            b"\r\n--frame\r\nContent-Type: image/jpeg\r\nContent-Length: 5\r\n\r\nhello".to_vec(),
        );
        assert_eq!(next_frame_length(&mut ok).unwrap(), 5);
        let mut missing = Cursor::new(b"--frame\r\nContent-Type: image/jpeg\r\n\r\n".to_vec());
        assert!(next_frame_length(&mut missing).is_err());
        let mut huge = Cursor::new(b"--frame\r\ncontent-length: 2000001\r\n\r\n".to_vec());
        assert!(next_frame_length(&mut huge).is_err());
        let mut zero = Cursor::new(b"--frame\r\nContent-Length: 0\r\n\r\n".to_vec());
        assert!(next_frame_length(&mut zero).is_err());
        let mut closed = Cursor::new(b"garbage\r\n".to_vec());
        assert!(next_frame_length(&mut closed).is_err());
        let mut endless = Cursor::new(b"x\r\n".repeat(10_000));
        assert_eq!(
            next_frame_length(&mut endless).unwrap_err().to_string(),
            "no frame boundary"
        );
        let mut headers = b"--frame\r\n".to_vec();
        headers.extend(b"X-A: b\r\n".repeat(10_000));
        assert_eq!(
            next_frame_length(&mut Cursor::new(headers))
                .unwrap_err()
                .to_string(),
            "too many frame header lines"
        );
    }

    struct Sink;
    impl FrameSink for Sink {
        fn on_frame(&self, _: f64, _: &Bytes) {}
        fn on_connect(&self) {}
    }

    #[test]
    fn frames_are_dropped_while_disabled_or_in_standby() {
        let grabber = Grabber::new(CameraClient::new("127.0.0.1", "t", 1, 1));
        let rx = grabber.subscribe();
        grabber.set_enabled(false);
        assert!(!grabber.publish(clock::now(), Bytes::from_static(b"x")));
        assert!(grabber.latest().is_none() && !grabber.online());
        grabber.set_enabled(true);
        assert!(grabber.publish(clock::now(), Bytes::from_static(b"x")));
        assert!(grabber.online() && rx.has_changed().unwrap());
        grabber.mark_standby(true);
        assert!(grabber.latest().is_none() && grabber.fps() == 0.0);
        assert!(!grabber.publish(clock::now(), Bytes::from_static(b"x")));
        let _ = Sink;
    }

    #[test]
    fn fps_is_smoothed() {
        let grabber = Grabber::new(CameraClient::new("127.0.0.1", "t", 1, 1));
        let now = clock::now();
        for i in 0..50 {
            grabber.publish(now + i as f64 * 0.1, Bytes::from_static(b"x"));
        }
        assert!((grabber.fps() - 10.0).abs() < 0.1);
    }

    #[test]
    fn chunked_streams_are_read_through_one_buffer() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            let part =
                b"--frame\r\nContent-Type: image/jpeg\r\nContent-Length: 10\r\n\r\n0123456789\r\n";
            let mut out = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
            for piece in [&part[..20], &part[20..65], &part[65..]] {
                out.extend(format!("{:x}\r\n", piece.len()).as_bytes());
                out.extend(piece);
                out.extend(b"\r\n");
            }
            out.extend(b"0\r\n\r\n");
            stream.write_all(&out).unwrap();
        });
        let camera = CameraClient::new("127.0.0.1", "t", port as i64, port as i64);
        let mut response = camera
            .request(port, "/stream", Duration::from_secs(5), None)
            .unwrap();
        assert!(matches!(response.body, Body::Chunked { .. }));
        let length = next_frame_length(&mut response.body).unwrap();
        let mut frame = vec![0u8; length];
        response.body.read_exact(&mut frame).unwrap();
        assert_eq!(frame, b"0123456789");
        let mut rest = Vec::new();
        response.body.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, b"\r\n");
        server.join().unwrap();
    }

    #[test]
    fn unreachable_cameras_fail_quickly() {
        let camera = CameraClient::new("127.0.0.1", "t", 1, 1);
        assert!(!camera.set_standby(true));
        assert!(camera.status().is_none());
        assert_eq!(
            camera.apply_controls(&[("vflip", 1)]),
            vec![("vflip".to_string(), false)]
        );
    }
}
