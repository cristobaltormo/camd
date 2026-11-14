use std::fs::File;
use std::io::{self, Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use bytes::Bytes;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tract_onnx::prelude::*;

use crate::clock;
use crate::config::SharedConfig;
use crate::error::{Error, Result};
use crate::imaging::{self, Image};
use crate::labels::{COCO_LABELS, index_of};

pub const MODEL_URL: &str = "https://github.com/opencv/opencv_zoo/raw/main/models/object_detection_nanodet/object_detection_nanodet_2022nov.onnx";
pub const MODEL_SHA256: &str = "4b82da9944b88577175ee23a459dce2e26e6e4be573def65b1055dc2d9720186";
pub const MODEL_NAME: &str = "nanodet-plus-m.onnx";
pub const PARTIAL_NAME: &str = "nanodet-plus-m.part";
pub const INPUT_SIZE: usize = 416;
pub const STRIDES: [usize; 3] = [8, 16, 32];
pub const REG_BINS: usize = 8;
pub const MEAN: [f32; 3] = [103.53, 116.28, 123.675];
pub const STD: [f32; 3] = [57.375, 57.12, 58.395];
pub const NMS_IOU: f32 = 0.5;
pub const RETRY_AFTER_S: f64 = 300.0;
pub const DOWNLOAD_TIMEOUT_S: u64 = 300;
pub const UNLOADED_WAIT_S: f64 = 30.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Detection {
    pub label: String,
    pub confidence: f64,
    pub bbox: [f64; 4],
}

fn round_to(value: f64, digits: i32) -> f64 {
    let scale = 10f64.powi(digits);
    (value * scale).round() / scale
}

impl Detection {
    pub fn new(label: &str, confidence: f64, bbox: [f64; 4]) -> Detection {
        Detection {
            label: label.to_string(),
            confidence,
            bbox,
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "label": self.label,
            "confidence": round_to(self.confidence, 3),
            "box": self.bbox.iter().map(|v| round_to(*v, 4)).collect::<Vec<_>>(),
        })
    }
}

pub fn in_zones(detection: &Detection, rects: &[Vec<f64>]) -> bool {
    if rects.is_empty() {
        return true;
    }
    let [x, y, w, h] = detection.bbox;
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    rects
        .iter()
        .any(|r| r.len() == 4 && r[0] <= cx && cx <= r[0] + r[2] && r[1] <= cy && cy <= r[1] + r[3])
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn download(out: &mut dyn Write) -> Result<()> {
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::Rustls)
        .root_certs(ureq::tls::RootCerts::WebPki)
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(Duration::from_secs(DOWNLOAD_TIMEOUT_S)))
        .build()
        .into();
    let response = agent
        .get(MODEL_URL)
        .call()
        .map_err(|e| Error::internal(e.to_string()))?;
    io::copy(&mut response.into_body().into_reader(), out)?;
    Ok(())
}

pub fn ensure_model(directory: &Path) -> Result<PathBuf> {
    ensure_model_with(directory, download)
}

pub fn ensure_model_with(
    directory: &Path,
    fetch: impl FnOnce(&mut dyn Write) -> Result<()>,
) -> Result<PathBuf> {
    let path = directory.join(MODEL_NAME);
    if path.is_file() && sha256_file(&path).ok().as_deref() == Some(MODEL_SHA256) {
        return Ok(path);
    }
    std::fs::create_dir_all(directory)?;
    let partial = directory.join(PARTIAL_NAME);
    crate::info!("camd.detect", "downloading object detection model");
    let fetched = (|| -> Result<()> {
        let mut out = io::BufWriter::new(File::create(&partial)?);
        fetch(&mut out)?;
        out.flush()?;
        Ok(())
    })();
    if let Err(e) = fetched {
        let _ = std::fs::remove_file(&partial);
        return Err(e);
    }
    if sha256_file(&partial)? != MODEL_SHA256 {
        let _ = std::fs::remove_file(&partial);
        return Err(Error::internal(
            "downloaded model does not match the expected checksum",
        ));
    }
    std::fs::rename(&partial, &path)?;
    Ok(path)
}

pub fn cells(stride: usize) -> usize {
    INPUT_SIZE.div_ceil(stride)
}

pub fn anchors(stride: usize) -> Vec<(f32, f32)> {
    let n = cells(stride);
    let offset = (stride / 2) as f32;
    (0..n * n)
        .map(|i| {
            (
                ((i % n) * stride) as f32 + offset,
                ((i / n) * stride) as f32 + offset,
            )
        })
        .collect()
}

fn round_half_even(value: f64) -> usize {
    value.round_ties_even().max(0.0) as usize
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Letterbox {
    pub scale: f64,
    pub pad_x: usize,
    pub pad_y: usize,
    pub width: usize,
    pub height: usize,
}

impl Letterbox {
    pub fn fit(width: usize, height: usize) -> Letterbox {
        let scale = INPUT_SIZE as f64 / width.max(height).max(1) as f64;
        let (new_w, new_h) = (
            round_half_even(width as f64 * scale),
            round_half_even(height as f64 * scale),
        );
        Letterbox {
            scale,
            pad_x: INPUT_SIZE.saturating_sub(new_w) / 2,
            pad_y: INPUT_SIZE.saturating_sub(new_h) / 2,
            width,
            height,
        }
    }

    pub fn resized(&self) -> (usize, usize) {
        (
            round_half_even(self.width as f64 * self.scale).min(INPUT_SIZE),
            round_half_even(self.height as f64 * self.scale).min(INPUT_SIZE),
        )
    }

    pub fn blob(&self, rgb: &Image) -> Vec<f32> {
        let (new_w, new_h) = self.resized();
        let small = imaging::resize_area(rgb, new_w, new_h);
        let plane = INPUT_SIZE * INPUT_SIZE;
        let mut blob = vec![0f32; 3 * plane];
        for channel in 0..3 {
            let background = (0.0 - MEAN[channel]) / STD[channel];
            let source_channel = 2 - channel;
            let target = &mut blob[channel * plane..(channel + 1) * plane];
            target.fill(background);
            for y in 0..new_h {
                let row = &mut target[(self.pad_y + y) * INPUT_SIZE + self.pad_x..];
                let source = &small.pixels[y * new_w * 3..(y + 1) * new_w * 3];
                for (slot, pixel) in row[..new_w]
                    .iter_mut()
                    .zip(source.as_chunks::<3>().0.iter())
                {
                    *slot = (pixel[source_channel] as f32 - MEAN[channel]) / STD[channel];
                }
            }
        }
        blob
    }
}

pub struct Output<'a> {
    pub shape: &'a [usize],
    pub data: &'a [f32],
}

pub struct Level<'a> {
    pub cells: usize,
    pub scores: &'a [f32],
    pub distances: &'a [f32],
}

type Slots<'a> = (usize, Option<&'a [f32]>, Option<&'a [f32]>);

pub fn pair_levels<'a>(outputs: &[Output<'a>]) -> Result<Vec<Level<'a>>> {
    let unexpected = || Error::internal("unexpected outputs from the detection model");
    let mut levels: Vec<Slots<'a>> = Vec::new();
    for output in outputs {
        if output.shape.len() != 3 || output.shape[0] != 1 {
            return Err(unexpected());
        }
        let (count, last) = (output.shape[1], output.shape[2]);
        if output.data.len() != count * last {
            return Err(unexpected());
        }
        let slot = match levels.iter().position(|l| l.0 == count) {
            Some(i) => i,
            None => {
                levels.push((count, None, None));
                levels.len() - 1
            }
        };
        if last == COCO_LABELS.len() {
            levels[slot].1 = Some(output.data);
        } else if last == 4 * REG_BINS {
            levels[slot].2 = Some(output.data);
        } else {
            return Err(unexpected());
        }
    }
    levels.sort_by_key(|level| std::cmp::Reverse(level.0));
    if levels.len() != STRIDES.len() {
        return Err(unexpected());
    }
    levels
        .into_iter()
        .zip(STRIDES)
        .map(
            |((count, scores, distances), stride)| match (scores, distances) {
                (Some(scores), Some(distances)) if count == cells(stride) * cells(stride) => {
                    Ok(Level {
                        cells: count,
                        scores,
                        distances,
                    })
                }
                _ => Err(unexpected()),
            },
        )
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Candidates {
    pub boxes: Vec<[f32; 4]>,
    pub scores: Vec<f32>,
    pub classes: Vec<usize>,
}

pub fn decode(
    outputs: &[Output<'_>],
    class_ids: &[usize],
    min_confidence: f32,
    letterbox: &Letterbox,
) -> Result<Candidates> {
    let mut found = Candidates::default();
    let scale = letterbox.scale as f32;
    let (pad_x, pad_y) = (letterbox.pad_x as f32, letterbox.pad_y as f32);
    for (level, stride) in pair_levels(outputs)?.into_iter().zip(STRIDES) {
        let level_anchors = anchors(stride);
        for cell in 0..level.cells {
            let row = &level.scores[cell * COCO_LABELS.len()..(cell + 1) * COCO_LABELS.len()];
            let mut best = f32::NEG_INFINITY;
            let mut best_class = 0;
            for (i, class) in class_ids.iter().enumerate() {
                if row[*class] > best {
                    best = row[*class];
                    best_class = i;
                }
            }
            if class_ids.is_empty() || best < min_confidence {
                continue;
            }
            let bins = &level.distances[cell * 4 * REG_BINS..(cell + 1) * 4 * REG_BINS];
            let mut distance = [0f32; 4];
            for (side, slot) in distance.iter_mut().enumerate() {
                let logits = &bins[side * REG_BINS..(side + 1) * REG_BINS];
                let top = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let weights: Vec<f32> = logits.iter().map(|v| (v - top).exp()).collect();
                let total: f32 = weights.iter().sum();
                let expected: f32 = weights
                    .iter()
                    .enumerate()
                    .map(|(i, w)| (w / total) * i as f32)
                    .sum();
                *slot = expected * stride as f32;
            }
            let (cx, cy) = level_anchors[cell];
            let x1 = (cx - distance[0] - pad_x) / scale;
            let y1 = (cy - distance[1] - pad_y) / scale;
            let x2 = (cx + distance[2] - pad_x) / scale;
            let y2 = (cy + distance[3] - pad_y) / scale;
            found.boxes.push([x1, y1, x2 - x1, y2 - y1]);
            found.scores.push(best);
            found.classes.push(best_class);
        }
    }
    Ok(found)
}

fn iou(a: &[f64; 4], b: &[f64; 4]) -> f64 {
    let (ax2, ay2, bx2, by2) = (a[0] + a[2], a[1] + a[3], b[0] + b[2], b[1] + b[3]);
    let w = ax2.min(bx2) - a[0].max(b[0]);
    let h = ay2.min(by2) - a[1].max(b[1]);
    if w <= 0.0 || h <= 0.0 {
        return 0.0;
    }
    let inter = w * h;
    let union = a[2] * a[3] + b[2] * b[3] - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
}

pub fn nms_batched(
    boxes: &[[f32; 4]],
    scores: &[f32],
    classes: &[usize],
    score_threshold: f32,
    iou_threshold: f32,
) -> Vec<usize> {
    let mut order: Vec<usize> = (0..scores.len())
        .filter(|i| scores[*i] > score_threshold)
        .collect();
    order.sort_by(|a, b| scores[*b].total_cmp(&scores[*a]));
    let wide: Vec<[f64; 4]> = boxes.iter().map(|b| b.map(|v| v as f64)).collect();
    let mut kept: Vec<usize> = Vec::new();
    for index in order {
        let clear = kept.iter().all(|k| {
            classes[*k] != classes[index] || iou(&wide[*k], &wide[index]) <= iou_threshold as f64
        });
        if clear {
            kept.push(index);
        }
    }
    kept
}

pub fn finish(
    found: &Candidates,
    kept: &[usize],
    labels: &[String],
    width: usize,
    height: usize,
) -> Vec<Detection> {
    let (fw, fh) = (width as f32, height as f32);
    let mut result: Vec<Detection> = kept
        .iter()
        .filter_map(|i| {
            let [x, y, w, h] = found.boxes[*i];
            let (x0, y0) = ((x / fw).max(0.0), (y / fh).max(0.0));
            let (x1, y1) = (((x + w) / fw).min(1.0), ((y + h) / fh).min(1.0));
            (x1 > x0 && y1 > y0).then(|| Detection {
                label: labels[found.classes[*i]].clone(),
                confidence: found.scores[*i] as f64,
                bbox: [x0 as f64, y0 as f64, (x1 - x0) as f64, (y1 - y0) as f64],
            })
        })
        .collect();
    result.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    result
}

pub fn class_ids(labels: &[String]) -> Result<Vec<usize>> {
    let unknown: Vec<&str> = labels
        .iter()
        .filter(|l| index_of(l).is_none())
        .map(String::as_str)
        .collect();
    if !unknown.is_empty() {
        return Err(Error::invalid(format!(
            "unknown labels: {}",
            unknown.join(", ")
        )));
    }
    Ok(labels.iter().filter_map(|l| index_of(l)).collect())
}

pub trait Detector: Send {
    fn detect(
        &mut self,
        jpeg: &[u8],
        labels: &[String],
        min_confidence: f64,
    ) -> Result<Vec<Detection>>;
}

type Plan = Arc<TypedRunnableModel>;

pub struct NanoDet {
    plan: Plan,
}

fn tract_error(e: impl std::fmt::Display) -> Error {
    Error::internal(format!("detection model: {e}"))
}

impl NanoDet {
    pub fn load(path: &Path) -> Result<NanoDet> {
        let plan = tract_onnx::onnx()
            .model_for_path(path)
            .and_then(|m| m.with_input_fact(0, f32::fact([1, 3, INPUT_SIZE, INPUT_SIZE]).into()))
            .and_then(|m| m.into_optimized())
            .and_then(|m| m.into_runnable())
            .map_err(tract_error)?;
        Ok(NanoDet { plan })
    }

    pub fn detect_image(
        &self,
        rgb: &Image,
        labels: &[String],
        min_confidence: f64,
    ) -> Result<Vec<Detection>> {
        let ids = class_ids(labels)?;
        let letterbox = Letterbox::fit(rgb.width, rgb.height);
        let blob = letterbox.blob(rgb);
        let input =
            Tensor::from_shape(&[1, 3, INPUT_SIZE, INPUT_SIZE], &blob).map_err(tract_error)?;
        drop(blob);
        let values = self.plan.run(tvec!(input.into())).map_err(tract_error)?;
        let mut outputs = Vec::with_capacity(values.len());
        for value in &values {
            outputs.push(Output {
                shape: value.shape(),
                data: value
                    .try_as_plain_ram()
                    .and_then(|plain| plain.as_slice::<f32>())
                    .map_err(tract_error)?,
            });
        }
        let threshold = min_confidence as f32;
        let found = decode(&outputs, &ids, threshold, &letterbox)?;
        if found.scores.is_empty() {
            return Ok(Vec::new());
        }
        let kept = nms_batched(
            &found.boxes,
            &found.scores,
            &found.classes,
            threshold,
            NMS_IOU,
        );
        Ok(finish(&found, &kept, labels, rgb.width, rgb.height))
    }
}

impl Detector for NanoDet {
    fn detect(
        &mut self,
        jpeg: &[u8],
        labels: &[String],
        min_confidence: f64,
    ) -> Result<Vec<Detection>> {
        match imaging::decode_rgb(jpeg) {
            Some(rgb) => self.detect_image(&rgb, labels, min_confidence),
            None => Ok(Vec::new()),
        }
    }
}

pub type ResultHandler<'a> = dyn Fn(f64, &Bytes, Vec<Detection>) -> Result<()> + Sync + 'a;

pub type Loader = Box<dyn Fn(&Path) -> Result<Box<dyn Detector>> + Send + Sync>;

pub fn nanodet_loader() -> Loader {
    Box::new(|models_dir: &Path| {
        let path = ensure_model(models_dir)?;
        Ok(Box::new(NanoDet::load(&path)?) as Box<dyn Detector>)
    })
}

pub const STATE_IDLE: &str = "idle";
pub const STATE_LOADING: &str = "loading";
pub const STATE_READY: &str = "ready";
pub const STATE_ERROR: &str = "error";

struct Shared {
    state: &'static str,
    error: String,
    last_ms: f64,
    job: Option<(f64, Bytes)>,
    woken: bool,
    halted: bool,
    retry_at: f64,
}

pub struct DetectionWorker {
    cfg: SharedConfig,
    loader: Loader,
    shared: Mutex<Shared>,
    wake: Condvar,
}

fn release_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        libc::malloc_trim(0);
    }
}

impl DetectionWorker {
    pub fn new(cfg: SharedConfig) -> DetectionWorker {
        DetectionWorker::with_loader(cfg, nanodet_loader())
    }

    pub fn with_loader(cfg: SharedConfig, loader: Loader) -> DetectionWorker {
        DetectionWorker {
            cfg,
            loader,
            shared: Mutex::new(Shared {
                state: STATE_IDLE,
                error: String::new(),
                last_ms: 0.0,
                job: None,
                woken: false,
                halted: false,
                retry_at: 0.0,
            }),
            wake: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn state(&self) -> &'static str {
        self.lock().state
    }

    pub fn error(&self) -> String {
        self.lock().error.clone()
    }

    pub fn last_ms(&self) -> f64 {
        self.lock().last_ms
    }

    pub fn ready(&self) -> bool {
        self.state() == STATE_READY
    }

    pub fn submit(&self, ts: f64, jpeg: Bytes) -> bool {
        let mut shared = self.lock();
        if shared.job.is_some() || shared.state != STATE_READY {
            return false;
        }
        shared.job = Some((ts, jpeg));
        shared.woken = true;
        drop(shared);
        self.wake.notify_all();
        true
    }

    pub fn poke(&self) {
        self.lock().woken = true;
        self.wake.notify_all();
    }

    pub fn stop(&self) {
        let mut shared = self.lock();
        shared.halted = true;
        shared.woken = true;
        drop(shared);
        self.wake.notify_all();
    }

    fn settings(&self) -> (String, PathBuf, Vec<String>, f64) {
        let cfg = self.cfg.read().unwrap_or_else(|e| e.into_inner());
        (
            cfg.detection.clone(),
            cfg.models_dir(),
            cfg.detect_labels.clone(),
            cfg.detect_confidence,
        )
    }

    pub fn try_load(&self) -> Option<Box<dyn Detector>> {
        let (mode, models_dir, _, _) = self.settings();
        {
            let mut shared = self.lock();
            if mode == "off" || clock::now() < shared.retry_at {
                return None;
            }
            shared.state = STATE_LOADING;
        }
        let loaded = catch_unwind(AssertUnwindSafe(|| (self.loader)(&models_dir)))
            .unwrap_or_else(|_| Err(Error::internal("the detection model crashed while loading")));
        let mut shared = self.lock();
        match loaded {
            Ok(detector) => {
                shared.state = STATE_READY;
                shared.error.clear();
                crate::info!("camd.detect", "object detector ready");
                Some(detector)
            }
            Err(e) => {
                shared.state = STATE_ERROR;
                shared.error = e.to_string();
                shared.retry_at = clock::now() + RETRY_AFTER_S;
                crate::warning!("camd.detect", "object detector unavailable: {e}");
                None
            }
        }
    }

    pub fn run(&self, on_result: &ResultHandler<'_>) {
        let mut detector: Option<Box<dyn Detector>> = None;
        loop {
            if self.lock().halted {
                break;
            }
            let (mode, ..) = self.settings();
            if mode == "off" {
                if detector.take().is_some() {
                    let mut shared = self.lock();
                    shared.state = STATE_IDLE;
                    shared.job = None;
                    drop(shared);
                    release_memory();
                }
            } else if detector.is_none() {
                detector = self.try_load();
            }
            let job = {
                let mut shared = self.lock();
                if !shared.woken && !shared.halted {
                    shared = if detector.is_none() {
                        self.wake
                            .wait_timeout_while(
                                shared,
                                Duration::from_secs_f64(UNLOADED_WAIT_S),
                                |s| !s.woken && !s.halted,
                            )
                            .unwrap_or_else(|e| e.into_inner())
                            .0
                    } else {
                        self.wake
                            .wait_while(shared, |s| !s.woken && !s.halted)
                            .unwrap_or_else(|e| e.into_inner())
                    };
                }
                shared.woken = false;
                if shared.halted {
                    break;
                }
                shared.job.take()
            };
            let (Some((ts, jpeg)), Some(model)) = (job, detector.as_mut()) else {
                continue;
            };
            let (mode, _, labels, confidence) = self.settings();
            if mode == "off" {
                continue;
            }
            let started = Instant::now();
            let found = catch_unwind(AssertUnwindSafe(|| {
                model.detect(&jpeg, &labels, confidence)
            }));
            let found = match found {
                Ok(Ok(found)) => found,
                Ok(Err(e)) => {
                    crate::error!("camd.detect", "detection failed: {e}");
                    continue;
                }
                Err(_) => {
                    crate::error!("camd.detect", "detection failed");
                    continue;
                }
            };
            self.lock().last_ms = started.elapsed().as_secs_f64() * 1000.0;
            match catch_unwind(AssertUnwindSafe(|| on_result(ts, &jpeg, found))) {
                Ok(Ok(())) => {}
                Ok(Err(e)) => crate::error!("camd.detect", "detection result handler failed: {e}"),
                Err(_) => crate::error!("camd.detect", "detection result handler failed"),
            }
        }
    }
}

pub struct FakeDetector {
    pub found: Vec<Detection>,
    pub calls: Arc<Mutex<usize>>,
}

impl FakeDetector {
    pub fn always(found: Vec<Detection>) -> FakeDetector {
        FakeDetector {
            found,
            calls: Arc::new(Mutex::new(0)),
        }
    }

    pub fn loader(found: Vec<Detection>) -> Loader {
        Box::new(move |_: &Path| {
            Ok(Box::new(FakeDetector::always(found.clone())) as Box<dyn Detector>)
        })
    }
}

impl Detector for FakeDetector {
    fn detect(&mut self, _: &[u8], labels: &[String], _: f64) -> Result<Vec<Detection>> {
        *self.calls.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        Ok(self
            .found
            .iter()
            .filter(|d| labels.contains(&d.label))
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    struct Synthetic {
        shapes: Vec<Vec<usize>>,
        data: Vec<Vec<f32>>,
    }

    impl Synthetic {
        fn new(level: usize, cell: usize, label: &str, score: f32, bins: [usize; 4]) -> Synthetic {
            let mut shapes = Vec::new();
            let mut data = Vec::new();
            for (i, stride) in STRIDES.iter().enumerate() {
                let n = cells(*stride) * cells(*stride);
                let mut cls = vec![0f32; n * COCO_LABELS.len()];
                let mut reg = vec![0f32; n * 4 * REG_BINS];
                if i == level {
                    cls[cell * COCO_LABELS.len() + index_of(label).unwrap()] = score;
                    for (side, bin) in bins.iter().enumerate() {
                        reg[cell * 4 * REG_BINS + side * REG_BINS + bin] = 30.0;
                    }
                }
                shapes.push(vec![1, n, COCO_LABELS.len()]);
                data.push(cls);
                shapes.push(vec![1, n, 4 * REG_BINS]);
                data.push(reg);
            }
            Synthetic { shapes, data }
        }

        fn outputs(&self, order: &[usize]) -> Vec<Output<'_>> {
            order
                .iter()
                .map(|i| Output {
                    shape: &self.shapes[*i],
                    data: &self.data[*i],
                })
                .collect()
        }
    }

    const IN_ORDER: [usize; 6] = [0, 1, 2, 3, 4, 5];

    #[test]
    fn decode_recovers_a_box_from_raw_outputs() {
        let letterbox = Letterbox::fit(800, 600);
        let cell = 20 * (INPUT_SIZE / 16) + 13;
        let synthetic = Synthetic::new(1, cell, "person", 0.9, [2, 3, 4, 5]);
        let found = decode(&synthetic.outputs(&IN_ORDER), &[0], 0.5, &letterbox).unwrap();
        assert_eq!(found.scores.len(), 1);
        assert_eq!(found.classes[0], 0);
        let (cx, cy) = anchors(16)[cell];
        let [x, y, w, h] = found.boxes[0];
        let scale = letterbox.scale as f32;
        assert!((x - (cx - 32.0 - letterbox.pad_x as f32) / scale).abs() < 1.0);
        assert!((y - (cy - 48.0 - letterbox.pad_y as f32) / scale).abs() < 1.0);
        assert!((w - 96.0 / scale).abs() < 1.0 && (h - 128.0 / scale).abs() < 1.0);
    }

    #[test]
    fn decode_does_not_depend_on_the_order_of_the_outputs() {
        let letterbox = Letterbox::fit(800, 600);
        let cat = index_of("cat").unwrap();
        let synthetic = Synthetic::new(2, 40, "cat", 0.8, [3, 3, 3, 3]);
        let first = decode(&synthetic.outputs(&IN_ORDER), &[cat], 0.5, &letterbox).unwrap();
        let second = decode(
            &synthetic.outputs(&[5, 2, 0, 3, 1, 4]),
            &[cat],
            0.5,
            &letterbox,
        )
        .unwrap();
        assert_eq!(first.scores.len(), 1);
        assert_eq!(first, second);
    }

    #[test]
    fn decode_rejects_a_model_with_unexpected_outputs() {
        let letterbox = Letterbox::fit(800, 600);
        let data = vec![0f32; 800];
        let outputs = [Output {
            shape: &[1, 10, 80],
            data: &data,
        }];
        assert!(decode(&outputs, &[0], 0.5, &letterbox).is_err());
    }

    #[test]
    fn decode_ignores_low_scores_and_unwanted_classes() {
        let letterbox = Letterbox::fit(800, 600);
        let low = Synthetic::new(0, 100, "person", 0.3, [1, 1, 1, 1]);
        assert!(
            decode(&low.outputs(&IN_ORDER), &[0], 0.5, &letterbox)
                .unwrap()
                .scores
                .is_empty()
        );
        let chair = Synthetic::new(0, 100, "chair", 0.9, [1, 1, 1, 1]);
        assert!(
            decode(&chair.outputs(&IN_ORDER), &[0], 0.5, &letterbox)
                .unwrap()
                .scores
                .is_empty()
        );
    }

    #[test]
    fn letterbox_keeps_the_aspect_ratio() {
        let letterbox = Letterbox::fit(800, 600);
        assert!((letterbox.scale - 0.52).abs() < 1e-12);
        assert_eq!((letterbox.pad_x, letterbox.pad_y), (0, 52));
        let blob = letterbox.blob(&Image::filled(800, 600, 3, 0));
        assert_eq!(blob.len(), 3 * INPUT_SIZE * INPUT_SIZE);
        assert!((blob[0] - (-MEAN[0] / STD[0])).abs() < 1e-6);
        let small = Letterbox::fit(320, 240);
        assert_eq!(small.resized(), (416, 312));
    }

    #[test]
    fn letterbox_puts_blue_first() {
        let mut image = Image::filled(416, 416, 3, 0);
        for pixel in image.pixels.as_chunks_mut::<3>().0.iter_mut() {
            pixel[2] = 255;
        }
        let blob = Letterbox::fit(416, 416).blob(&image);
        let plane = INPUT_SIZE * INPUT_SIZE;
        assert!((blob[0] - (255.0 - MEAN[0]) / STD[0]).abs() < 1e-5);
        assert!((blob[2 * plane] - (0.0 - MEAN[2]) / STD[2]).abs() < 1e-5);
    }

    #[test]
    fn detections_are_serialised_with_rounded_boxes() {
        let item = Detection::new("person", 0.87654, [0.123456, 0.2, 0.3, 0.4]);
        assert_eq!(
            item.to_json(),
            json!({"label": "person", "confidence": 0.877, "box": [0.1235, 0.2, 0.3, 0.4]})
        );
    }

    #[test]
    fn default_labels_are_known_to_the_model() {
        assert!(
            crate::labels::DEFAULT_LABELS
                .iter()
                .all(|l| index_of(l).is_some())
        );
        assert!(class_ids(&["person".into(), "unicorn".into()]).is_err());
    }

    #[test]
    fn nms_matches_opencv_batched_semantics() {
        let boxes = [[0.0, 0.0, 1.9, 1.9], [0.9, 0.9, 1.9, 1.9]];
        assert_eq!(
            nms_batched(&boxes, &[0.9, 0.8], &[0, 0], 0.5, 0.5),
            vec![0, 1]
        );
        let same = [[0.0, 0.0, 10.0, 10.0], [0.0, 0.0, 10.0, 10.0]];
        assert_eq!(nms_batched(&same, &[0.5, 0.8], &[0, 0], 0.5, 0.5), vec![1]);
        assert_eq!(nms_batched(&same, &[0.8, 0.8], &[0, 0], 0.5, 0.5), vec![0]);
        assert_eq!(
            nms_batched(&same, &[0.8, 0.7], &[0, 1], 0.5, 0.5),
            vec![0, 1]
        );
        let shifted = [[0.0, 0.0, 10.0, 10.0], [5.0, 0.0, 10.0, 10.0]];
        assert_eq!(
            nms_batched(&shifted, &[0.8, 0.7], &[0, 0], 0.5, 0.33),
            vec![0]
        );
        assert_eq!(
            nms_batched(&shifted, &[0.8, 0.7], &[0, 0], 0.5, 0.34),
            vec![0, 1]
        );
    }

    #[test]
    fn boxes_are_normalised_clipped_and_sorted() {
        let found = Candidates {
            boxes: vec![
                [-10.0, 0.0, 110.0, 50.0],
                [10.0, 10.0, 0.0, 10.0],
                [700.0, 500.0, 200.0, 200.0],
            ],
            scores: vec![0.6, 0.9, 0.8],
            classes: vec![0, 0, 1],
        };
        let labels = vec!["person".to_string(), "cat".to_string()];
        let result = finish(&found, &[0, 1, 2], &labels, 800, 600);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].label, "cat");
        assert!((result[0].bbox[2] - 0.125).abs() < 1e-6);
        assert_eq!(result[1].bbox[0], 0.0);
    }

    #[test]
    fn model_download_is_verified() {
        let dir = tempfile::tempdir().unwrap();
        let result = ensure_model_with(dir.path(), |out| {
            out.write_all(b"not the model")?;
            Ok(())
        });
        assert!(result.unwrap_err().to_string().contains("checksum"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        let failed = ensure_model_with(dir.path(), |_| Err(Error::internal("offline")));
        assert_eq!(failed.unwrap_err().to_string(), "offline");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    fn settings(dir: &Path, mode: &str) -> SharedConfig {
        let mut cfg = Config::with_data_dir(dir);
        cfg.detection = mode.to_string();
        cfg.detect_labels = vec!["person".into()];
        cfg.shared()
    }

    #[test]
    fn worker_reports_an_error_when_the_model_cannot_be_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let worker = DetectionWorker::with_loader(
            settings(dir.path(), "alert"),
            Box::new(|_: &Path| Err(Error::internal("offline"))),
        );
        assert!(worker.try_load().is_none());
        assert_eq!(worker.state(), STATE_ERROR);
        assert!(worker.error().contains("offline") && !worker.ready());
        assert!(!worker.submit(0.0, Bytes::new()));
        assert!(worker.try_load().is_none());
        assert_eq!(worker.state(), STATE_ERROR);
    }

    fn wait(condition: impl Fn() -> bool) -> bool {
        for _ in 0..250 {
            if condition() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn worker_delivers_results_and_skips_busy_frames() {
        let dir = tempfile::tempdir().unwrap();
        let person = Detection::new("person", 0.9, [0.0, 0.0, 1.0, 1.0]);
        let worker = Arc::new(DetectionWorker::with_loader(
            settings(dir.path(), "alert"),
            FakeDetector::loader(vec![person]),
        ));
        type Results = Arc<Mutex<Vec<(f64, Vec<Detection>)>>>;
        let results: Results = Arc::default();
        let thread = {
            let (worker, results) = (Arc::clone(&worker), Arc::clone(&results));
            std::thread::spawn(move || {
                worker.run(&|ts, _, found| {
                    std::thread::sleep(Duration::from_millis(50));
                    results.lock().unwrap().push((ts, found));
                    Ok(())
                })
            })
        };
        assert!(wait(|| worker.ready()));
        assert!(worker.submit(1.0, Bytes::from_static(b"x")));
        assert!(
            !worker.submit(2.0, Bytes::from_static(b"x")) || results.lock().unwrap().is_empty()
        );
        assert!(wait(|| !results.lock().unwrap().is_empty()));
        worker.stop();
        thread.join().unwrap();
        let results = results.lock().unwrap();
        assert_eq!(results[0].0, 1.0);
        assert_eq!(results[0].1[0].label, "person");
    }

    #[test]
    fn a_failing_result_handler_does_not_kill_the_detector_thread() {
        let dir = tempfile::tempdir().unwrap();
        let worker = Arc::new(DetectionWorker::with_loader(
            settings(dir.path(), "alert"),
            FakeDetector::loader(Vec::new()),
        ));
        let calls: Arc<Mutex<Vec<f64>>> = Arc::default();
        let thread = {
            let (worker, calls) = (Arc::clone(&worker), Arc::clone(&calls));
            std::thread::spawn(move || {
                worker.run(&|ts, _, _| {
                    calls.lock().unwrap().push(ts);
                    if ts == 1.0 {
                        panic!("database is locked");
                    }
                    Err(Error::internal("database is locked"))
                })
            })
        };
        for ts in [1.0, 2.0, 3.0] {
            assert!(wait(|| worker.submit(ts, Bytes::from_static(b"x"))));
            assert!(wait(|| calls.lock().unwrap().contains(&ts)));
        }
        worker.stop();
        thread.join().unwrap();
        assert_eq!(*calls.lock().unwrap(), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn the_model_is_dropped_when_detection_turns_off_and_reloaded_after() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = settings(dir.path(), "alert");
        let loads: Arc<Mutex<usize>> = Arc::default();
        let counter = Arc::clone(&loads);
        let worker = Arc::new(DetectionWorker::with_loader(
            Arc::clone(&cfg),
            Box::new(move |_: &Path| {
                *counter.lock().unwrap() += 1;
                Ok(Box::new(FakeDetector::always(Vec::new())) as Box<dyn Detector>)
            }),
        ));
        let thread = {
            let worker = Arc::clone(&worker);
            std::thread::spawn(move || worker.run(&|_, _, _| Ok(())))
        };
        assert!(wait(|| worker.ready()));
        cfg.write().unwrap().detection = "off".into();
        worker.poke();
        assert!(wait(|| worker.state() == STATE_IDLE));
        assert!(!worker.submit(1.0, Bytes::new()));
        cfg.write().unwrap().detection = "strict".into();
        worker.poke();
        assert!(wait(|| worker.ready()));
        worker.stop();
        thread.join().unwrap();
        assert_eq!(*loads.lock().unwrap(), 2);
    }

    #[test]
    fn zones_filter_by_the_centre_of_the_box() {
        let inside = Detection::new("person", 0.9, [0.1, 0.1, 0.2, 0.2]);
        let outside = Detection::new("person", 0.9, [0.7, 0.7, 0.2, 0.2]);
        let zone = vec![vec![0.0, 0.0, 0.5, 0.5]];
        assert!(in_zones(&inside, &zone) && !in_zones(&outside, &zone));
        assert!(in_zones(&outside, &[]));
        let straddling = Detection::new("person", 0.9, [0.4, 0.4, 0.4, 0.4]);
        assert!(!in_zones(&straddling, &zone));
    }
}
