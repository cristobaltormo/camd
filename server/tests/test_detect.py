import numpy as np
import pytest

from camd import detect
from camd.detect import COCO_LABELS, DetectionWorker, Letterbox, decode


def synthetic_outputs(level, cell, label, score, bins):
    outputs = []
    for stride in detect.STRIDES:
        cells = detect.INPUT_SIZE // stride + (detect.INPUT_SIZE % stride > 0)
        cls = np.zeros((1, cells * cells, len(COCO_LABELS)), np.float32)
        reg = np.zeros((1, cells * cells, 4 * detect.REG_BINS), np.float32)
        if stride == detect.STRIDES[level]:
            cls[0, cell, COCO_LABELS.index(label)] = score
            for side, bin_index in enumerate(bins):
                reg[0, cell, side * detect.REG_BINS + bin_index] = 30.0
        outputs += [cls, reg]
    return outputs


def test_decode_recovers_a_box_from_raw_outputs():
    width, height = 800, 600
    box = Letterbox.fit(width, height)
    cell = 20 * (detect.INPUT_SIZE // 16) + 13
    outputs = synthetic_outputs(1, cell, "person", 0.9, (2, 3, 4, 5))
    anchors = [detect._anchors(s) for s in detect.STRIDES]
    boxes, scores, classes = decode(outputs, np.array([COCO_LABELS.index("person")]), 0.5, anchors, box)
    assert len(scores) == 1 and classes[0] == 0
    cx, cy = anchors[1][cell]
    x, y, w, h = boxes[0]
    assert x == pytest.approx((cx - 2 * 16 - box.pad_x) / box.scale, abs=1)
    assert y == pytest.approx((cy - 3 * 16 - box.pad_y) / box.scale, abs=1)
    assert w == pytest.approx(6 * 16 / box.scale, abs=1) and h == pytest.approx(8 * 16 / box.scale, abs=1)


def test_decode_does_not_depend_on_the_order_of_the_outputs():
    box = Letterbox.fit(800, 600)
    anchors = [detect._anchors(s) for s in detect.STRIDES]
    outputs = synthetic_outputs(2, 40, "cat", 0.8, (3, 3, 3, 3))
    shuffled = [outputs[i] for i in (5, 2, 0, 3, 1, 4)]
    first = decode(outputs, np.array([COCO_LABELS.index("cat")]), 0.5, anchors, box)
    second = decode(shuffled, np.array([COCO_LABELS.index("cat")]), 0.5, anchors, box)
    assert len(first[1]) == 1 and np.allclose(first[0], second[0])


def test_decode_rejects_a_model_with_unexpected_outputs():
    box = Letterbox.fit(800, 600)
    anchors = [detect._anchors(s) for s in detect.STRIDES]
    with pytest.raises(ValueError):
        decode([np.zeros((1, 10, 80), np.float32)], np.array([0]), 0.5, anchors, box)


def test_decode_ignores_low_scores_and_unwanted_classes():
    box = Letterbox.fit(800, 600)
    anchors = [detect._anchors(s) for s in detect.STRIDES]
    outputs = synthetic_outputs(0, 100, "person", 0.3, (1, 1, 1, 1))
    assert not len(decode(outputs, np.array([0]), 0.5, anchors, box)[1])
    outputs = synthetic_outputs(0, 100, "chair", 0.9, (1, 1, 1, 1))
    assert not len(decode(outputs, np.array([0]), 0.5, anchors, box)[1])


def test_letterbox_keeps_the_aspect_ratio():
    box = Letterbox.fit(800, 600)
    assert box.scale == pytest.approx(0.52) and box.pad_x == 0 and box.pad_y == 52
    blob = box.apply(np.zeros((600, 800, 3), np.uint8))
    assert blob.shape == (1, 3, detect.INPUT_SIZE, detect.INPUT_SIZE)


def test_detections_are_serialised_with_rounded_boxes():
    item = detect.Detection("person", 0.87654, (0.123456, 0.2, 0.3, 0.4))
    assert item.as_dict() == {"label": "person", "confidence": 0.877, "box": [0.1235, 0.2, 0.3, 0.4]}


def test_default_labels_are_known_to_the_model():
    assert set(detect.DEFAULT_LABELS) <= set(COCO_LABELS)


def test_model_download_is_verified(tmp_path, monkeypatch):
    payload = b"not the model"

    class Reply:
        def __init__(self):
            self.data = payload

        def read(self, size):
            chunk, self.data = self.data[:size], self.data[size:]
            return chunk

        def __enter__(self):
            return self

        def __exit__(self, *exc):
            return False

    monkeypatch.setattr(detect.urllib.request, "urlopen", lambda *a, **k: Reply())
    with pytest.raises(RuntimeError, match="checksum"):
        detect.ensure_model(tmp_path)
    assert not list(tmp_path.glob("*"))


def test_worker_reports_an_error_when_the_model_cannot_be_loaded(tmp_path, monkeypatch):
    class Settings:
        detection = "alert"
        detect_labels = ["person"]
        detect_confidence = 0.5
        models_dir = tmp_path

    monkeypatch.setattr(detect, "ensure_model", lambda _: (_ for _ in ()).throw(OSError("offline")))
    worker = DetectionWorker(Settings(), lambda *a: None)
    worker._load()
    assert worker.state == "error" and "offline" in worker.error and not worker.ready
    assert worker.submit(0.0, b"") is False


def test_worker_delivers_results_and_skips_busy_frames(tmp_path):
    class Settings:
        detection = "alert"
        detect_labels = ["person"]
        detect_confidence = 0.5
        models_dir = tmp_path

    class Fake:
        labels = ("person",)
        min_confidence = 0.5

        def detect(self, jpeg):
            return [detect.Detection("person", 0.9, (0, 0, 1, 1))]

    results = []
    worker = DetectionWorker(Settings(), lambda ts, jpeg, found: results.append((ts, found)))
    worker._detector, worker.state = Fake(), "ready"
    worker.start()
    assert worker.submit(1.0, b"x")
    for _ in range(100):
        if results:
            break
        import time

        time.sleep(0.02)
    worker.stop()
    assert results and results[0][0] == 1.0 and results[0][1][0].label == "person"
