use std::collections::{HashSet, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bytes::Bytes;
use serde_json::Value;

use crate::clock;
use crate::config::SharedConfig;
use crate::db::{Database, EventDone};
use crate::detect::Detection;
use crate::error::{Error, Result};
use crate::runtime::ManualClip;
use crate::spool::Spool;
use crate::storage::unlink;

pub const START_STREAK: u32 = 3;
pub const MIN_FPS: f64 = 1.0;
pub const MAX_FPS: f64 = 30.0;
pub const STALL_S: f64 = 20.0;
pub const ENCODER_NICE: i32 = 10;
pub const ENCODER_TIMEOUT_S: f64 = 600.0;
pub const DETECTION_RANK_BASE: f64 = 1.0;
pub const RECENT_DETECTION_S: f64 = 4.0;
pub const MAX_CLIP_S: u32 = 300;
pub const FLUSH_TIMEOUT_S: f64 = 30.0;

pub trait RecorderHooks: Send + Sync {
    fn analysis_ready(&self) -> bool {
        false
    }
    fn on_start(&self, _event_id: i64, _start_ts: f64, _analyzed: bool) {}
    fn on_confirm(&self, _event_id: i64, _start_ts: f64, _labels: &[String]) {}
    fn on_finish(&self, _event_id: i64) {}
}

pub struct NoHooks;

impl RecorderHooks for NoHooks {}

struct Recording {
    event_id: i64,
    spool: Spool,
    start: f64,
    kind: String,
    analyzed: bool,
    last_activity: f64,
    manual_until: f64,
    peak_score: f64,
    labels: Vec<(String, f64)>,
    best_rank: f64,
    best_jpeg: Option<Bytes>,
    best_boxes: Vec<Detection>,
    announced: bool,
}

impl Recording {
    fn new(event_id: i64, spool: Spool, start: f64, kind: &str, analyzed: bool) -> Recording {
        Recording {
            event_id,
            spool,
            start,
            kind: kind.to_string(),
            analyzed,
            last_activity: 0.0,
            manual_until: 0.0,
            peak_score: 0.0,
            labels: Vec::new(),
            best_rank: -1.0,
            best_jpeg: None,
            best_boxes: Vec::new(),
            announced: false,
        }
    }

    fn manual(&self) -> bool {
        self.kind == "manual"
    }

    fn consider(&mut self, rank: f64, jpeg: &Bytes, boxes: &[Detection]) {
        if rank > self.best_rank {
            self.best_rank = rank;
            self.best_jpeg = Some(jpeg.clone());
            self.best_boxes = boxes.to_vec();
        }
    }

    fn ranked_labels(&self) -> Vec<String> {
        let mut ranked = self.labels.clone();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        ranked.into_iter().map(|(name, _)| name).collect()
    }

    fn note_label(&mut self, label: &str, confidence: f64) {
        match self.labels.iter_mut().find(|(name, _)| name == label) {
            Some(entry) => entry.1 = entry.1.max(confidence),
            None => self.labels.push((label.to_string(), confidence.max(0.0))),
        }
    }
}

#[derive(Default)]
struct Inner {
    preroll: VecDeque<(f64, Bytes)>,
    current: Option<Recording>,
    streak: u32,
    last_frame: f64,
    recent: Option<(f64, Bytes, Vec<Detection>)>,
}

struct Finisher {
    cfg: SharedConfig,
    db: Arc<Database>,
    hooks: Arc<dyn RecorderHooks>,
    encoder_slot: Mutex<()>,
}

pub struct Recorder {
    finisher: Arc<Finisher>,
    inner: Mutex<Inner>,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn python_list(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|s| format!("'{s}'")).collect();
    format!("[{}]", quoted.join(", "))
}

pub fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| {
            use std::os::unix::fs::PermissionsExt;
            candidate
                .metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

pub fn encoder_command(ffmpeg: &Path, nice: Option<&Path>, fps: f64, out: &Path) -> Command {
    let mut args: Vec<String> = vec![
        ffmpeg.to_string_lossy().into_owned(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
        "-f".into(),
        "image2pipe".into(),
        "-framerate".into(),
        format!("{fps:.3}"),
        "-c:v".into(),
        "mjpeg".into(),
        "-i".into(),
        "-".into(),
        "-threads".into(),
        "1".into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-crf".into(),
        "28".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-movflags".into(),
        "+faststart".into(),
        out.to_string_lossy().into_owned(),
    ];
    if let Some(nice) = nice {
        args.splice(
            0..0,
            [
                nice.to_string_lossy().into_owned(),
                "-n".into(),
                ENCODER_NICE.to_string(),
            ],
        );
    }
    let mut command = Command::new(&args[0]);
    command.args(&args[1..]);
    command
}

pub fn encode(spool: &Spool, fps: f64, out: &Path) -> Result<()> {
    let ffmpeg = which("ffmpeg").ok_or_else(|| Error::internal("ffmpeg is not installed"))?;
    let nice = which("nice");
    let command = encoder_command(&ffmpeg, nice.as_deref(), fps, out);
    run_encoder(
        command,
        spool.frames()?.map(|(_, jpeg)| jpeg),
        Duration::from_secs_f64(ENCODER_TIMEOUT_S),
    )
}

const STDERR_KEPT_BYTES: usize = 64 * 1024;

fn drain_stderr(mut stderr: impl Read) -> String {
    let mut kept = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match stderr.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = STDERR_KEPT_BYTES.saturating_sub(kept.len());
                kept.extend_from_slice(&chunk[..n.min(room)]);
            }
        }
    }
    String::from_utf8_lossy(&kept).into_owned()
}

pub fn run_encoder(
    mut command: Command,
    frames: impl Iterator<Item = Vec<u8>> + Send,
    timeout: Duration,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let stderr = child.stderr.take();
    let stdin = child.stdin.take();
    std::thread::scope(|scope| {
        let reader = scope.spawn(move || stderr.map(drain_stderr).unwrap_or_default());
        let feeder = scope.spawn(move || -> std::io::Result<()> {
            let mut stdin = stdin.ok_or_else(|| std::io::Error::other("no stdin"))?;
            for jpeg in frames {
                match stdin.write_all(&jpeg) {
                    Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => break,
                    other => other?,
                }
            }
            Ok(())
        });
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(Some(status)),
                Ok(None) if Instant::now() >= deadline => break Ok(None),
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(e) => break Err(e),
            }
        };
        let status = match status {
            Ok(Some(status)) => status,
            other => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = feeder.join();
                let _ = reader.join();
                return match other {
                    Err(e) => Err(e.into()),
                    _ => Err(Error::internal("ffmpeg timed out")),
                };
            }
        };
        let fed = feeder
            .join()
            .unwrap_or_else(|_| Err(std::io::Error::other("feeder panicked")));
        let error = reader.join().unwrap_or_default();
        fed?;
        if !status.success() {
            return Err(Error::internal(format!("ffmpeg failed: {}", error.trim())));
        }
        Ok(())
    })
}

impl Finisher {
    fn spool_path(&self, event_id: i64) -> PathBuf {
        lock_read(&self.cfg)
            .spool_dir()
            .join(format!("{event_id}.spool"))
    }

    fn finalize(&self, mut rec: Recording) {
        if let Err(e) = self.write_clip(&mut rec) {
            crate::error!("camd.events", "event {} failed: {e}", rec.event_id);
            let _ = self.db.fail_event(rec.event_id, clock::now());
        }
        let _ = rec.spool.delete();
        self.hooks.on_finish(rec.event_id);
    }

    fn write_clip(&self, rec: &mut Recording) -> Result<()> {
        let (strict, clips_dir) = {
            let cfg = lock_read(&self.cfg);
            (cfg.detection == "strict", cfg.clips_dir())
        };
        if rec.spool.count < 2
            || (rec.analyzed && rec.labels.is_empty() && rec.kind == "motion" && strict)
        {
            self.db.delete_event_row(rec.event_id)?;
            return Ok(());
        }
        let folder = clips_dir.join(clock::local(rec.start).day_path());
        std::fs::create_dir_all(&folder)?;
        let clip = folder.join(format!("{}.mp4", rec.event_id));
        let thumb = folder.join(format!("{}.jpg", rec.event_id));
        let still = match &rec.best_jpeg {
            Some(jpeg) => jpeg.to_vec(),
            None => rec
                .spool
                .frames()?
                .nth(rec.spool.count / 2)
                .map(|(_, jpeg)| jpeg)
                .ok_or_else(|| Error::internal("the recording has no middle frame"))?,
        };
        std::fs::write(&thumb, still)?;
        let fps = ((rec.spool.count as f64 - 1.0) / rec.spool.duration()).clamp(MIN_FPS, MAX_FPS);
        {
            let _slot = lock(&self.encoder_slot);
            encode(&rec.spool, fps, &clip)?;
        }
        let size = std::fs::metadata(&clip)?.len() + std::fs::metadata(&thumb)?.len();
        let labels = rec.ranked_labels();
        self.db.finish_event(
            rec.event_id,
            &EventDone {
                end_ts: rec.spool.last_ts,
                score: rec.peak_score,
                clip_path: clip.to_string_lossy().into_owned(),
                thumb_path: thumb.to_string_lossy().into_owned(),
                size_bytes: size as i64,
                labels: labels.clone(),
                detections: Value::Array(rec.best_boxes.iter().map(Detection::to_json).collect()),
            },
        )?;
        crate::info!(
            "camd.events",
            "event {} done: {:.1}s, {} frames, {:.0} KB, labels={}",
            rec.event_id,
            rec.spool.duration(),
            rec.spool.count,
            size as f64 / 1024.0,
            python_list(&labels)
        );
        Ok(())
    }
}

fn lock_read(cfg: &SharedConfig) -> std::sync::RwLockReadGuard<'_, crate::config::Config> {
    cfg.read().unwrap_or_else(|e| e.into_inner())
}

impl Recorder {
    pub fn new(cfg: SharedConfig, db: Arc<Database>, hooks: Arc<dyn RecorderHooks>) -> Recorder {
        Recorder {
            finisher: Arc::new(Finisher {
                cfg,
                db,
                hooks,
                encoder_slot: Mutex::new(()),
            }),
            inner: Mutex::new(Inner::default()),
            workers: Mutex::new(Vec::new()),
        }
    }

    pub fn recording(&self) -> bool {
        lock(&self.inner).current.is_some()
    }

    pub fn current_event(&self) -> Option<i64> {
        lock(&self.inner).current.as_ref().map(|r| r.event_id)
    }

    pub fn spool_path(&self, event_id: i64) -> PathBuf {
        self.finisher.spool_path(event_id)
    }

    pub fn feed(&self, ts: f64, jpeg: &Bytes, motion: bool, score: f64, armed: bool) -> Result<()> {
        let (pre_roll, post_roll, max_event) = {
            let cfg = lock_read(&self.finisher.cfg);
            (cfg.pre_roll_s, cfg.post_roll_s, cfg.max_event_s)
        };
        let mut inner = lock(&self.inner);
        inner.last_frame = ts;
        if inner.current.is_none() {
            inner.preroll.push_back((ts, jpeg.clone()));
            while inner
                .preroll
                .front()
                .is_some_and(|(first, _)| ts - first > pre_roll)
            {
                inner.preroll.pop_front();
            }
            inner.streak = if armed && motion { inner.streak + 1 } else { 0 };
            if inner.streak >= START_STREAK {
                self.begin(&mut inner, "motion", ts)?;
            }
            return Ok(());
        }
        let rec = inner.current.as_mut().expect("recording");
        if let Err(e) = rec.spool.append(ts, jpeg) {
            crate::error!(
                "camd.events",
                "event {}: cannot write the recording, closing it: {e}",
                rec.event_id
            );
            self.end(&mut inner);
            return Ok(());
        }
        rec.peak_score = rec.peak_score.max(score);
        rec.consider(score, jpeg, &[]);
        if motion {
            rec.last_activity = ts;
        }
        let finished = if rec.manual() {
            ts >= rec.manual_until
        } else {
            !armed || ts - rec.last_activity > post_roll || ts - rec.start > max_event
        };
        if finished {
            self.end(&mut inner);
        }
        Ok(())
    }

    pub fn observe(&self, ts: f64, jpeg: &Bytes, detections: &[Detection]) -> Result<()> {
        let mut inner = lock(&self.inner);
        if detections.is_empty() {
            return Ok(());
        }
        match inner.current.as_mut() {
            None => {
                inner.recent = Some((ts, jpeg.clone(), detections.to_vec()));
                Ok(())
            }
            Some(rec) if !rec.manual() => self.apply(rec, ts, jpeg, detections),
            Some(_) => Ok(()),
        }
    }

    fn apply(
        &self,
        rec: &mut Recording,
        ts: f64,
        jpeg: &Bytes,
        detections: &[Detection],
    ) -> Result<()> {
        rec.last_activity = rec.last_activity.max(ts);
        let fresh = detections
            .iter()
            .any(|d| !rec.labels.iter().any(|(name, _)| *name == d.label));
        for d in detections {
            rec.note_label(&d.label, d.confidence);
        }
        let top = detections
            .iter()
            .map(|d| d.confidence)
            .fold(f64::NEG_INFINITY, f64::max);
        rec.consider(DETECTION_RANK_BASE + top, jpeg, detections);
        if fresh {
            self.finisher
                .db
                .set_event_labels(rec.event_id, &rec.ranked_labels())?;
        }
        if !rec.announced {
            rec.announced = true;
            self.finisher
                .hooks
                .on_confirm(rec.event_id, rec.start, &rec.ranked_labels());
        }
        Ok(())
    }

    fn begin(&self, inner: &mut Inner, kind: &str, now: f64) -> Result<i64> {
        let preroll: Vec<(f64, Bytes)> = inner.preroll.drain(..).collect();
        let start = preroll.first().map(|(ts, _)| *ts).unwrap_or(now);
        let analyzed = kind == "motion" && self.finisher.hooks.analysis_ready();
        let db = &self.finisher.db;
        let event_id = db.insert_event(start, kind, kind == "manual", analyzed)?;
        let spool = (|| -> std::io::Result<Spool> {
            let mut spool = Spool::create(&self.spool_path(event_id))?;
            for (ts, jpeg) in &preroll {
                spool.append(*ts, jpeg)?;
            }
            Ok(spool)
        })();
        let spool = match spool {
            Ok(spool) => spool,
            Err(e) => {
                let _ = db.delete_event_row(event_id);
                unlink(Some(&self.spool_path(event_id).to_string_lossy()));
                return Err(e.into());
            }
        };
        let mut rec = Recording::new(event_id, spool, start, kind, analyzed);
        rec.last_activity = now;
        inner.streak = 0;
        crate::info!("camd.events", "event {event_id} started ({kind})");
        if kind == "motion" {
            self.finisher.hooks.on_start(event_id, start, analyzed);
            if let Some((ts, jpeg, detections)) = inner.recent.take()
                && now - ts <= RECENT_DETECTION_S
            {
                self.apply(&mut rec, ts, &jpeg, &detections)?;
            }
        }
        inner.current = Some(rec);
        Ok(event_id)
    }

    fn end(&self, inner: &mut Inner) {
        let Some(mut rec) = inner.current.take() else {
            return;
        };
        inner.streak = 0;
        let _ = rec.spool.close();
        self.spawn(rec);
    }

    fn spawn(&self, rec: Recording) {
        let finisher = Arc::clone(&self.finisher);
        let name = format!("encode-{}", rec.event_id);
        let mut workers = lock(&self.workers);
        workers.retain(|w| !w.is_finished());
        match std::thread::Builder::new()
            .name(name)
            .spawn(move || finisher.finalize(rec))
        {
            Ok(worker) => workers.push(worker),
            Err(e) => crate::error!("camd.events", "cannot start the encoder: {e}"),
        }
    }

    pub fn flush(&self, timeout: f64) {
        let deadline = Instant::now() + Duration::from_secs_f64(timeout);
        let workers: Vec<JoinHandle<()>> = lock(&self.workers).drain(..).collect();
        let mut pending = Vec::new();
        for worker in workers {
            while !worker.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            if worker.is_finished() {
                let _ = worker.join();
            } else {
                pending.push(worker);
            }
        }
        lock(&self.workers).extend(pending);
    }

    pub fn close_open_event(&self) {
        let mut inner = lock(&self.inner);
        self.end(&mut inner);
    }

    pub fn start_manual(&self, seconds: f64, now: f64) -> Result<i64> {
        let mut inner = lock(&self.inner);
        if inner.current.is_none() {
            self.begin(&mut inner, "manual", now)?;
        }
        inner.last_frame = inner.last_frame.max(now);
        let rec = inner.current.as_mut().expect("recording");
        rec.manual_until = now + seconds;
        if !rec.manual() {
            rec.kind = "manual".to_string();
            self.finisher.db.mark_event_manual(rec.event_id)?;
        }
        Ok(rec.event_id)
    }

    pub fn stop_manual(&self) -> Option<i64> {
        let mut inner = lock(&self.inner);
        let id = inner
            .current
            .as_ref()
            .filter(|r| r.manual())
            .map(|r| r.event_id)?;
        self.end(&mut inner);
        Some(id)
    }

    pub fn manual_state(&self) -> ManualClip {
        let inner = lock(&self.inner);
        match inner.current.as_ref().filter(|r| r.manual()) {
            Some(rec) => ManualClip {
                active: true,
                id: Some(rec.event_id),
                until: rec.manual_until,
                started: rec.start,
            },
            None => ManualClip::inactive(),
        }
    }

    pub fn tick(&self, now: f64) {
        let mut inner = lock(&self.inner);
        let last_frame = inner.last_frame;
        let stale = inner
            .current
            .as_ref()
            .is_some_and(|r| (r.manual() && now >= r.manual_until) || now - last_frame > STALL_S);
        if stale {
            self.end(&mut inner);
        }
    }

    pub fn recover(&self) -> Result<()> {
        let db = &self.finisher.db;
        let interrupted = db.recording_events()?;
        let ids: HashSet<i64> = interrupted.iter().map(|row| row.id).collect();
        for row in interrupted {
            let path = self.spool_path(row.id);
            let spool = if path.is_file() {
                Spool::load(&path).ok()
            } else {
                None
            };
            let Some(spool) = spool.filter(|s| s.count >= 2) else {
                db.delete_event_row(row.id)?;
                unlink(Some(&path.to_string_lossy()));
                continue;
            };
            let mut rec = Recording::new(row.id, spool, row.start_ts, &row.kind, false);
            let known = row.label_list();
            let total = known.len();
            for (i, label) in known.into_iter().enumerate() {
                rec.note_label(&label, (total - i) as f64);
            }
            self.spawn(rec);
        }
        let spool_dir = lock_read(&self.finisher.cfg).spool_dir();
        if let Ok(entries) = std::fs::read_dir(&spool_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("spool") {
                    continue;
                }
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                let ours = stem.bytes().all(|b| b.is_ascii_digit())
                    && stem.parse::<i64>().is_ok_and(|id| ids.contains(&id));
                if !ours {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::motion::tests::scene;
    use crate::storage::purge;

    #[derive(Default)]
    struct Log {
        analysis: bool,
        start: Mutex<Vec<(i64, f64, bool)>>,
        confirm: Mutex<Vec<(i64, f64, Vec<String>)>>,
        finish: Mutex<Vec<i64>>,
    }

    impl RecorderHooks for Log {
        fn analysis_ready(&self) -> bool {
            self.analysis
        }
        fn on_start(&self, id: i64, start: f64, analyzed: bool) {
            self.start.lock().unwrap().push((id, start, analyzed));
        }
        fn on_confirm(&self, id: i64, start: f64, labels: &[String]) {
            self.confirm
                .lock()
                .unwrap()
                .push((id, start, labels.to_vec()));
        }
        fn on_finish(&self, id: i64) {
            self.finish.lock().unwrap().push(id);
        }
    }

    struct Parts {
        cfg: SharedConfig,
        db: Arc<Database>,
        recorder: Recorder,
        log: Arc<Log>,
        _dir: Option<tempfile::TempDir>,
    }

    fn parts_in(dir: &Path, analysis: bool, tweak: impl FnOnce(&mut Config)) -> Parts {
        let mut cfg = Config::with_data_dir(dir);
        tweak(&mut cfg);
        let db = Arc::new(Database::open(&cfg.db_path()).unwrap());
        let cfg = cfg.shared();
        let log = Arc::new(Log {
            analysis,
            ..Log::default()
        });
        let recorder = Recorder::new(Arc::clone(&cfg), Arc::clone(&db), log.clone());
        Parts {
            cfg,
            db,
            recorder,
            log,
            _dir: None,
        }
    }

    fn parts(analysis: bool, tweak: impl FnOnce(&mut Config)) -> Parts {
        let dir = tempfile::tempdir().unwrap();
        let mut parts = parts_in(dir.path(), analysis, tweak);
        parts._dir = Some(dir);
        parts
    }

    struct Frames {
        quiet: Bytes,
        busy: Bytes,
    }

    fn frames() -> &'static Frames {
        static FRAMES: std::sync::OnceLock<Frames> = std::sync::OnceLock::new();
        FRAMES.get_or_init(|| Frames {
            quiet: Bytes::from(scene(0, 90, false)),
            busy: Bytes::from(scene(0, 90, true)),
        })
    }

    fn run(recorder: &Recorder, start: f64, count: usize, motion: bool, armed: bool) -> f64 {
        let mut t = start;
        let jpeg = if motion {
            &frames().busy
        } else {
            &frames().quiet
        };
        for _ in 0..count {
            recorder
                .feed(t, jpeg, motion, if motion { 0.1 } else { 0.0 }, armed)
                .unwrap();
            t += 0.1;
        }
        t
    }

    fn person() -> Detection {
        Detection::new("person", 0.9, [0.2, 0.2, 0.3, 0.6])
    }

    fn cat() -> Detection {
        Detection::new("cat", 0.7, [0.5, 0.5, 0.2, 0.2])
    }

    fn only_event(db: &Database) -> Option<crate::db::EventRow> {
        let id: Option<i64> = db
            .with(|c| {
                use rusqlite::OptionalExtension;
                c.query_row("SELECT id FROM events ORDER BY id LIMIT 1", [], |r| {
                    r.get(0)
                })
                .optional()
            })
            .unwrap();
        id.and_then(|id| db.event(id).unwrap())
    }

    fn count_events(db: &Database) -> i64 {
        db.events_usage().unwrap().0
    }

    fn spool_files(cfg: &SharedConfig) -> usize {
        std::fs::read_dir(cfg.read().unwrap().spool_dir())
            .map(|d| d.count())
            .unwrap_or(0)
    }

    #[test]
    fn event_is_recorded_with_clip_and_thumbnail() {
        let p = parts(false, |c| c.post_roll_s = 1.0);
        let t = run(&p.recorder, 1000.0, 20, false, true);
        let t = run(&p.recorder, t, 30, true, true);
        assert!(p.recorder.recording() && p.log.start.lock().unwrap().len() == 1);
        run(&p.recorder, t, 30, false, true);
        p.recorder.flush(30.0);
        let row = only_event(&p.db).unwrap();
        assert_eq!(row.status, "done");
        assert!(!p.log.finish.lock().unwrap().is_empty());
        assert!(row.end_ts.unwrap() > row.start_ts);
        assert!(std::fs::metadata(row.clip_path.unwrap()).unwrap().len() > 1000);
        assert!(std::fs::metadata(row.thumb_path.unwrap()).unwrap().len() > 1000);
        assert_eq!(spool_files(&p.cfg), 0);
        assert!(row.start_ts >= 1000.0 && row.start_ts < 1002.0);
    }

    #[test]
    fn disarmed_never_records() {
        let p = parts(false, |_| {});
        run(&p.recorder, 1000.0, 60, true, false);
        assert!(!p.recorder.recording());
        assert_eq!(count_events(&p.db), 0);
    }

    #[test]
    fn a_short_blip_does_not_start_an_event() {
        let p = parts(false, |_| {});
        run(&p.recorder, 1000.0, 2, true, true);
        run(&p.recorder, 1000.5, 20, false, true);
        assert_eq!(count_events(&p.db), 0);
    }

    #[test]
    fn disarming_closes_the_open_event() {
        let p = parts(false, |_| {});
        let t = run(&p.recorder, 1000.0, 20, true, true);
        assert!(p.recorder.recording());
        run(&p.recorder, t, 2, true, false);
        p.recorder.flush(30.0);
        assert!(!p.recorder.recording());
        assert_eq!(only_event(&p.db).unwrap().status, "done");
    }

    #[test]
    fn maximum_duration_splits_an_event() {
        let p = parts(false, |c| c.max_event_s = 3.0);
        run(&p.recorder, 1000.0, 120, true, true);
        p.recorder.flush(60.0);
        let done: i64 =
            p.db.with(|c| {
                c.query_row("SELECT COUNT(*) FROM events WHERE status='done'", [], |r| {
                    r.get(0)
                })
            })
            .unwrap();
        assert!(done >= 2);
    }

    #[test]
    fn stalled_event_is_closed_by_tick() {
        let p = parts(false, |c| c.post_roll_s = 1.0);
        let t = run(&p.recorder, 1000.0, 10, false, true);
        run(&p.recorder, t, 20, true, true);
        assert!(p.recorder.recording());
        p.recorder.tick(t + 60.0);
        assert!(!p.recorder.recording());
        p.recorder.flush(30.0);
    }

    #[test]
    fn confirmation_labels_the_event_and_announces_once() {
        let p = parts(true, |c| c.post_roll_s = 1.0);
        let t = run(&p.recorder, 1000.0, 10, false, true);
        let t = run(&p.recorder, t, 10, true, true);
        assert!(p.log.start.lock().unwrap()[0].2);
        assert!(p.log.confirm.lock().unwrap().is_empty());
        let busy = &frames().busy;
        p.recorder.observe(t, busy, &[person()]).unwrap();
        p.recorder
            .observe(t + 0.5, busy, &[person(), cat()])
            .unwrap();
        {
            let confirm = p.log.confirm.lock().unwrap();
            assert_eq!(confirm.len(), 1);
            assert_eq!(confirm[0].2, vec!["person".to_string()]);
        }
        let id = p.recorder.current_event().unwrap();
        assert_eq!(
            p.db.event(id).unwrap().unwrap().label_list(),
            vec!["person", "cat"]
        );
        run(&p.recorder, t, 40, false, true);
        p.recorder.flush(30.0);
        let row = p.db.event(id).unwrap().unwrap();
        assert!(row.confirmed && row.analyzed);
        let detections: Vec<Value> = serde_json::from_str(&row.detections).unwrap();
        let labels: Vec<&str> = detections
            .iter()
            .map(|d| d["label"].as_str().unwrap())
            .collect();
        assert_eq!(labels, vec!["person"]);
    }

    #[test]
    fn detection_arriving_just_before_the_event_still_counts() {
        let p = parts(true, |_| {});
        let t = run(&p.recorder, 1000.0, 5, false, true);
        p.recorder.observe(t, &frames().busy, &[person()]).unwrap();
        run(&p.recorder, t, 6, true, true);
        let confirm = p.log.confirm.lock().unwrap();
        assert_eq!(confirm[0].2, vec!["person".to_string()]);
        drop(confirm);
        p.recorder.close_open_event();
        p.recorder.flush(30.0);
    }

    #[test]
    fn an_old_detection_does_not_count() {
        let p = parts(true, |_| {});
        let t = run(&p.recorder, 1000.0, 5, false, true);
        p.recorder
            .observe(t - 10.0, &frames().busy, &[person()])
            .unwrap();
        run(&p.recorder, t, 6, true, true);
        assert!(p.log.confirm.lock().unwrap().is_empty());
        p.recorder.close_open_event();
        p.recorder.flush(30.0);
    }

    #[test]
    fn a_still_person_keeps_the_event_alive() {
        let p = parts(true, |c| c.post_roll_s = 2.0);
        let t = run(&p.recorder, 1000.0, 10, false, true);
        let mut t = run(&p.recorder, t, 10, true, true);
        for _ in 0..5 {
            p.recorder.observe(t, &frames().quiet, &[person()]).unwrap();
            t = run(&p.recorder, t, 10, false, true);
        }
        assert!(p.recorder.recording());
        p.recorder.close_open_event();
        p.recorder.flush(30.0);
    }

    #[test]
    fn strict_mode_discards_unconfirmed_events_without_encoding() {
        let p = parts(true, |c| {
            c.detection = "strict".into();
            c.post_roll_s = 1.0;
        });
        let t = run(&p.recorder, 1000.0, 10, false, true);
        let t = run(&p.recorder, t, 20, true, true);
        run(&p.recorder, t, 30, false, true);
        p.recorder.flush(30.0);
        assert_eq!(count_events(&p.db), 0);
        assert!(!p.log.finish.lock().unwrap().is_empty());
        assert_eq!(spool_files(&p.cfg), 0);
    }

    #[test]
    fn alert_mode_keeps_unconfirmed_events_for_review() {
        let p = parts(true, |c| {
            c.detection = "alert".into();
            c.post_roll_s = 1.0;
        });
        let t = run(&p.recorder, 1000.0, 10, false, true);
        let t = run(&p.recorder, t, 20, true, true);
        run(&p.recorder, t, 30, false, true);
        p.recorder.flush(30.0);
        let row = only_event(&p.db).unwrap();
        assert!(row.status == "done" && row.analyzed && !row.confirmed);
    }

    #[test]
    fn strict_mode_without_a_ready_detector_keeps_everything() {
        let p = parts(false, |c| {
            c.detection = "strict".into();
            c.post_roll_s = 1.0;
        });
        let t = run(&p.recorder, 1000.0, 10, false, true);
        let t = run(&p.recorder, t, 20, true, true);
        run(&p.recorder, t, 30, false, true);
        p.recorder.flush(30.0);
        assert_eq!(only_event(&p.db).unwrap().status, "done");
    }

    #[test]
    fn manual_clip_is_kept_and_survives_purge() {
        let p = parts(false, |c| c.retention_days = 1);
        let t = run(&p.recorder, 1000.0, 20, false, true);
        let id = p.recorder.start_manual(60.0, t).unwrap();
        assert!(p.recorder.manual_state().active);
        assert!(p.log.start.lock().unwrap().is_empty());
        run(&p.recorder, t, 30, false, false);
        assert!(p.recorder.recording());
        assert_eq!(p.recorder.stop_manual(), Some(id));
        p.recorder.flush(30.0);
        let row = p.db.event(id).unwrap().unwrap();
        assert!(row.kind == "manual" && row.keep && row.status == "done");
        p.db.execute("UPDATE events SET start_ts=1 WHERE id=?", [id])
            .unwrap();
        assert_eq!(purge(&p.cfg.read().unwrap(), &p.db).unwrap(), 0);
        assert!(Path::new(&row.clip_path.unwrap()).exists());
    }

    #[test]
    fn manual_clip_ends_by_itself() {
        let p = parts(false, |_| {});
        let t = run(&p.recorder, 1000.0, 10, false, true);
        p.recorder.start_manual(2.0, t).unwrap();
        run(&p.recorder, t, 40, false, false);
        assert!(!p.recorder.recording());
        p.recorder.flush(30.0);
        assert_eq!(only_event(&p.db).unwrap().status, "done");
        assert_eq!(p.recorder.manual_state(), ManualClip::inactive());
        assert_eq!(p.recorder.stop_manual(), None);
    }

    #[test]
    fn manual_clip_ignores_detections() {
        let p = parts(true, |_| {});
        let t = run(&p.recorder, 1000.0, 10, false, true);
        p.recorder.start_manual(30.0, t).unwrap();
        p.recorder.observe(t, &frames().quiet, &[person()]).unwrap();
        assert!(p.log.confirm.lock().unwrap().is_empty());
        p.recorder.stop_manual();
        p.recorder.flush(30.0);
    }

    #[test]
    fn starting_a_manual_clip_during_motion_keeps_the_event() {
        let p = parts(false, |c| c.post_roll_s = 1.0);
        let t = run(&p.recorder, 1000.0, 10, false, true);
        let t = run(&p.recorder, t, 20, true, true);
        let id = p.recorder.start_manual(60.0, t).unwrap();
        let row = p.db.event(id).unwrap().unwrap();
        assert!(row.kind == "manual" && row.keep);
        run(&p.recorder, t, 40, false, true);
        assert!(p.recorder.recording());
        p.recorder.stop_manual();
        p.recorder.flush(30.0);
    }

    #[test]
    fn interrupted_recordings_are_recovered_from_the_spool() {
        let dir = tempfile::tempdir().unwrap();
        let id = {
            let p = parts_in(dir.path(), true, |_| {});
            run(&p.recorder, 1000.0, 10, false, true);
            run(&p.recorder, 1001.0, 25, true, true);
            let id = p.recorder.current_event().unwrap();
            p.recorder
                .observe(1003.0, &frames().busy, &[person()])
                .unwrap();
            let mut inner = p.recorder.inner.lock().unwrap();
            inner.current.as_mut().unwrap().spool.close().unwrap();
            std::mem::forget(inner.current.take());
            id
        };
        let p = parts_in(dir.path(), false, |_| {});
        p.recorder.recover().unwrap();
        p.recorder.flush(30.0);
        let row = p.db.event(id).unwrap().unwrap();
        assert_eq!(row.status, "done");
        assert!(Path::new(row.clip_path.as_ref().unwrap()).exists());
        assert_eq!(spool_files(&p.cfg), 0);
        assert_eq!(row.label_list(), vec!["person"]);
        assert!(row.confirmed);
    }

    #[test]
    fn recovery_drops_empty_spools_and_orphans() {
        let p = parts(false, |_| {});
        p.db.execute(
            "INSERT INTO events(start_ts, status) VALUES(1, 'recording')",
            [],
        )
        .unwrap();
        let spool_dir = p.cfg.read().unwrap().spool_dir();
        std::fs::create_dir_all(&spool_dir).unwrap();
        std::fs::write(spool_dir.join("99.spool"), b"").unwrap();
        std::fs::write(spool_dir.join("x.spool"), b"").unwrap();
        p.recorder.recover().unwrap();
        assert_eq!(count_events(&p.db), 0);
        assert_eq!(spool_files(&p.cfg), 0);
    }

    #[test]
    fn a_full_disk_closes_the_event_instead_of_wedging_it() {
        let p = parts(false, |_| {});
        let t = run(&p.recorder, 1000.0, 10, false, true);
        let t = run(&p.recorder, t, 10, true, true);
        assert!(p.recorder.recording());
        p.recorder
            .inner
            .lock()
            .unwrap()
            .current
            .as_mut()
            .unwrap()
            .spool
            .close()
            .unwrap();
        run(&p.recorder, t, 2, true, true);
        assert!(!p.recorder.recording());
        p.recorder.flush(30.0);
    }

    #[test]
    fn a_failed_encode_marks_the_event_failed() {
        let p = parts(false, |c| c.post_roll_s = 1.0);
        let t = run(&p.recorder, 1000.0, 10, false, true);
        let bad = Bytes::from_static(b"not a jpeg at all");
        let mut t = t;
        for _ in 0..20 {
            p.recorder.feed(t, &bad, true, 0.1, true).unwrap();
            t += 0.1;
        }
        let id = p.recorder.current_event().unwrap();
        let clips = p.cfg.read().unwrap().clips_dir();
        std::fs::create_dir_all(clips.parent().unwrap()).unwrap();
        std::fs::write(&clips, b"a file where the clips folder should be").unwrap();
        p.recorder.close_open_event();
        p.recorder.flush(30.0);
        let row = p.db.event(id).unwrap().unwrap();
        assert_eq!(row.status, "failed");
        assert!(row.end_ts.is_some());
        assert_eq!(spool_files(&p.cfg), 0);
        assert_eq!(*p.log.finish.lock().unwrap(), vec![id]);
    }

    fn shell(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }

    #[test]
    fn a_stalled_encoder_is_killed_at_the_deadline() {
        let started = Instant::now();
        let frames = (0..64).map(|_| vec![0u8; 64 * 1024]);
        let result = run_encoder(shell("exec sleep 30"), frames, Duration::from_millis(500));
        assert_eq!(result.unwrap_err().message(), "ffmpeg timed out");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_chatty_encoder_does_not_deadlock_and_its_error_is_kept() {
        let frames = (0..64).map(|_| vec![0u8; 64 * 1024]);
        let script =
            "head -c 300000 /dev/zero | tr '\\0' x >&2; cat >/dev/null; echo boom >&2; exit 3";
        let result = run_encoder(shell(script), frames, Duration::from_secs(20));
        let message = result.unwrap_err().message().to_string();
        assert!(
            message.starts_with("ffmpeg failed: xxx"),
            "{}",
            &message[..40]
        );
        assert!(message.len() < 70 * 1024);
        assert!(
            run_encoder(
                shell("cat >/dev/null"),
                std::iter::once(vec![1]),
                Duration::from_secs(20)
            )
            .is_ok()
        );
    }

    #[test]
    fn encoder_arguments_match_the_python_server() {
        let command = encoder_command(
            Path::new("/usr/bin/ffmpeg"),
            Some(Path::new("/usr/bin/nice")),
            9.87654,
            Path::new("/c/1.mp4"),
        );
        let args: Vec<String> = std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args.join(" "),
            "/usr/bin/nice -n 10 /usr/bin/ffmpeg -hide_banner -loglevel error -y -f image2pipe -framerate 9.877 -c:v mjpeg -i - -threads 1 -c:v libx264 -preset veryfast -crf 28 -pix_fmt yuv420p -movflags +faststart /c/1.mp4"
        );
    }

    #[test]
    fn labels_are_logged_like_python_lists() {
        assert_eq!(
            python_list(&["person".into(), "cat".into()]),
            "['person', 'cat']"
        );
        assert_eq!(python_list(&[]), "[]");
    }
}
