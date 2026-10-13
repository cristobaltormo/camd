import hashlib
import logging
import threading
import time
import urllib.request
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

import cv2
import numpy as np

log = logging.getLogger("camd.detect")

MODEL_URL = "https://github.com/opencv/opencv_zoo/raw/main/models/object_detection_nanodet/object_detection_nanodet_2022nov.onnx"
MODEL_SHA256 = "4b82da9944b88577175ee23a459dce2e26e6e4be573def65b1055dc2d9720186"
MODEL_NAME = "nanodet-plus-m.onnx"

INPUT_SIZE = 416
STRIDES = (8, 16, 32)
REG_BINS = 8
MEAN = np.array([103.53, 116.28, 123.675], np.float32)
STD = np.array([57.375, 57.12, 58.395], np.float32)
NMS_IOU = 0.5

COCO_LABELS = (
    "person",
    "bicycle",
    "car",
    "motorcycle",
    "airplane",
    "bus",
    "train",
    "truck",
    "boat",
    "traffic light",
    "fire hydrant",
    "stop sign",
    "parking meter",
    "bench",
    "bird",
    "cat",
    "dog",
    "horse",
    "sheep",
    "cow",
    "elephant",
    "bear",
    "zebra",
    "giraffe",
    "backpack",
    "umbrella",
    "handbag",
    "tie",
    "suitcase",
    "frisbee",
    "skis",
    "snowboard",
    "sports ball",
    "kite",
    "baseball bat",
    "baseball glove",
    "skateboard",
    "surfboard",
    "tennis racket",
    "bottle",
    "wine glass",
    "cup",
    "fork",
    "knife",
    "spoon",
    "bowl",
    "banana",
    "apple",
    "sandwich",
    "orange",
    "broccoli",
    "carrot",
    "hot dog",
    "pizza",
    "donut",
    "cake",
    "chair",
    "couch",
    "potted plant",
    "bed",
    "dining table",
    "toilet",
    "tv",
    "laptop",
    "mouse",
    "remote",
    "keyboard",
    "cell phone",
    "microwave",
    "oven",
    "toaster",
    "sink",
    "refrigerator",
    "book",
    "clock",
    "vase",
    "scissors",
    "teddy bear",
    "hair drier",
    "toothbrush",
)
DEFAULT_LABELS = ("person", "cat", "dog")


@dataclass(frozen=True)
class Detection:
    label: str
    confidence: float
    box: tuple[float, float, float, float]

    def as_dict(self) -> dict:
        return {"label": self.label, "confidence": round(self.confidence, 3), "box": [round(v, 4) for v in self.box]}


def ensure_model(directory: Path) -> Path:
    path = directory / MODEL_NAME
    if path.is_file() and _sha256(path) == MODEL_SHA256:
        return path
    directory.mkdir(parents=True, exist_ok=True)
    partial = path.with_suffix(".part")
    log.info("downloading object detection model")
    with urllib.request.urlopen(MODEL_URL, timeout=60) as response, partial.open("wb") as out:
        while chunk := response.read(1 << 16):
            out.write(chunk)
    if _sha256(partial) != MODEL_SHA256:
        partial.unlink()
        raise RuntimeError("downloaded model does not match the expected checksum")
    partial.replace(path)
    return path


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as fh:
        while chunk := fh.read(1 << 20):
            digest.update(chunk)
    return digest.hexdigest()


def _anchors(stride: int) -> np.ndarray:
    cells = INPUT_SIZE // stride + (INPUT_SIZE % stride > 0)
    xs, ys = np.meshgrid(np.arange(cells) * stride, np.arange(cells) * stride)
    return (np.stack([xs.ravel(), ys.ravel()], axis=1) + stride // 2).astype(np.float32)


@dataclass(frozen=True)
class Letterbox:
    scale: float
    pad_x: int
    pad_y: int
    width: int
    height: int

    @classmethod
    def fit(cls, width: int, height: int) -> "Letterbox":
        scale = INPUT_SIZE / max(width, height)
        return cls(scale, (INPUT_SIZE - round(width * scale)) // 2, (INPUT_SIZE - round(height * scale)) // 2, width, height)

    def apply(self, frame: np.ndarray) -> np.ndarray:
        new_w, new_h = round(self.width * self.scale), round(self.height * self.scale)
        canvas = np.zeros((INPUT_SIZE, INPUT_SIZE, 3), np.float32)
        canvas[self.pad_y : self.pad_y + new_h, self.pad_x : self.pad_x + new_w] = cv2.resize(frame, (new_w, new_h), interpolation=cv2.INTER_AREA)
        return cv2.dnn.blobFromImage((canvas - MEAN) / STD)


def pair_levels(outputs: list[np.ndarray]) -> list[tuple[np.ndarray, np.ndarray]]:
    levels: dict[int, dict[str, np.ndarray]] = {}
    for tensor in outputs:
        role = "scores" if tensor.shape[-1] == len(COCO_LABELS) else "distances"
        levels.setdefault(tensor.shape[1], {})[role] = tensor
    ordered = [levels[cells] for cells in sorted(levels, reverse=True)]
    if len(ordered) != len(STRIDES) or any(len(level) != 2 for level in ordered):
        raise ValueError("unexpected outputs from the detection model")
    return [(level["scores"], level["distances"]) for level in ordered]


def decode(
    outputs: list[np.ndarray], class_ids: np.ndarray, min_confidence: float, anchors: list[np.ndarray], box: Letterbox
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    boxes, scores, classes = [], [], []
    bin_ids = np.arange(REG_BINS, dtype=np.float32)
    for stride, level_anchors, (cls, reg) in zip(STRIDES, anchors, pair_levels(outputs), strict=True):
        cls = cls[0][:, class_ids]
        best = cls.max(axis=1)
        keep = np.flatnonzero(best >= min_confidence)
        if keep.size == 0:
            continue
        bins = reg[0][keep].reshape(-1, 4, REG_BINS)
        bins = np.exp(bins - bins.max(axis=2, keepdims=True))
        bins /= bins.sum(axis=2, keepdims=True)
        distance = (bins @ bin_ids) * stride
        cx, cy = level_anchors[keep, 0], level_anchors[keep, 1]
        x1 = (cx - distance[:, 0] - box.pad_x) / box.scale
        y1 = (cy - distance[:, 1] - box.pad_y) / box.scale
        x2 = (cx + distance[:, 2] - box.pad_x) / box.scale
        y2 = (cy + distance[:, 3] - box.pad_y) / box.scale
        boxes.append(np.stack([x1, y1, x2 - x1, y2 - y1], axis=1))
        scores.append(best[keep])
        classes.append(cls[keep].argmax(axis=1))
    if not boxes:
        return np.empty((0, 4)), np.empty(0), np.empty(0, dtype=np.intp)
    return np.concatenate(boxes), np.concatenate(scores), np.concatenate(classes)


class ObjectDetector:
    def __init__(self, model_path: Path, labels: tuple[str, ...] = DEFAULT_LABELS, min_confidence: float = 0.5):
        self.net = cv2.dnn.readNetFromONNX(str(model_path))
        self.output_names = self.net.getUnconnectedOutLayersNames()
        self.anchors = [_anchors(s) for s in STRIDES]
        self.min_confidence = min_confidence
        self.labels: tuple[str, ...] = ()
        self.class_ids = np.empty(0, dtype=np.intp)
        self.set_labels(labels)

    def set_labels(self, labels: tuple[str, ...]) -> None:
        unknown = [name for name in labels if name not in COCO_LABELS]
        if unknown:
            raise ValueError(f"unknown labels: {', '.join(unknown)}")
        self.labels = tuple(labels)
        self.class_ids = np.array([COCO_LABELS.index(name) for name in labels], dtype=np.intp)

    def detect(self, jpeg: bytes) -> list[Detection]:
        frame = cv2.imdecode(np.frombuffer(jpeg, np.uint8), cv2.IMREAD_COLOR)
        return self.detect_image(frame) if frame is not None else []

    def detect_image(self, frame: np.ndarray) -> list[Detection]:
        height, width = frame.shape[:2]
        box = Letterbox.fit(width, height)
        self.net.setInput(box.apply(frame))
        outputs = self.net.forward(self.output_names)
        boxes, scores, classes = decode(outputs, self.class_ids, self.min_confidence, self.anchors, box)
        if not len(scores):
            return []
        kept = cv2.dnn.NMSBoxesBatched(boxes.tolist(), scores.tolist(), classes.tolist(), self.min_confidence, NMS_IOU)
        found = []
        for i in np.asarray(kept).ravel():
            x, y, w, h = boxes[i]
            x0, y0 = max(0.0, x / width), max(0.0, y / height)
            x1, y1 = min(1.0, (x + w) / width), min(1.0, (y + h) / height)
            if x1 > x0 and y1 > y0:
                found.append(Detection(self.labels[classes[i]], float(scores[i]), (float(x0), float(y0), float(x1 - x0), float(y1 - y0))))
        return sorted(found, key=lambda d: -d.confidence)


class DetectionSettings(Protocol):
    detection: str
    detect_labels: list[str]
    detect_confidence: float
    models_dir: Path


ResultCallback = Callable[[float, bytes, list[Detection]], None]
RETRY_AFTER_S = 300


class DetectionWorker(threading.Thread):
    def __init__(self, cfg: DetectionSettings, on_result: ResultCallback):
        super().__init__(daemon=True, name="detector")
        self.cfg = cfg
        self.on_result = on_result
        self.state = "idle"
        self.error = ""
        self.last_ms = 0.0
        self._detector: ObjectDetector | None = None
        self._job: tuple[float, bytes] | None = None
        self._wake = threading.Event()
        self._halt = threading.Event()
        self._retry_at = 0.0
        self._lock = threading.Lock()

    @property
    def ready(self) -> bool:
        return self.state == "ready"

    def submit(self, ts: float, jpeg: bytes) -> bool:
        with self._lock:
            if self._job is not None or not self.ready:
                return False
            self._job = (ts, jpeg)
        self._wake.set()
        return True

    def poke(self) -> None:
        self._wake.set()

    def stop(self) -> None:
        self._halt.set()
        self._wake.set()

    def _load(self) -> None:
        if self.cfg.detection == "off" or time.time() < self._retry_at:
            return
        self.state = "loading"
        try:
            path = ensure_model(self.cfg.models_dir)
            self._detector = ObjectDetector(path, tuple(self.cfg.detect_labels), self.cfg.detect_confidence)
            self.state, self.error = "ready", ""
            log.info("object detector ready")
        except Exception as exc:
            self.state, self.error = "error", str(exc)
            self._retry_at = time.time() + RETRY_AFTER_S
            log.warning("object detector unavailable: %s", exc)

    def _sync_settings(self, detector: ObjectDetector) -> None:
        labels = tuple(self.cfg.detect_labels)
        if detector.labels != labels:
            detector.set_labels(labels)
        detector.min_confidence = self.cfg.detect_confidence

    def run(self) -> None:
        while not self._halt.is_set():
            if self._detector is None:
                self._load()
            self._wake.wait(30 if self._detector is None else None)
            self._wake.clear()
            with self._lock:
                job, self._job = self._job, None
            if job is None or self._detector is None:
                continue
            started = time.perf_counter()
            try:
                self._sync_settings(self._detector)
                found = self._detector.detect(job[1])
            except Exception:
                log.exception("detection failed")
                continue
            self.last_ms = (time.perf_counter() - started) * 1000
            try:
                self.on_result(job[0], job[1], found)
            except Exception:
                log.exception("detection result handler failed")
