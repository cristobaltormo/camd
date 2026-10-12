from dataclasses import dataclass, field

import cv2
import numpy as np

WARMUP_FRAMES = 20
GLOBAL_CHANGE_FRACTION = 0.5
BG_ALPHA = 0.05
BG_ALPHA_DURING_MOTION = 0.01


@dataclass
class MotionSettings:
    sensitivity: int = 5
    ignore_rects: list[list[float]] = field(default_factory=list)

    @property
    def pixel_threshold(self) -> int:
        return max(8, 40 - 3 * self.sensitivity)

    @property
    def min_area_fraction(self) -> float:
        return 0.0015 * (11 - self.sensitivity)


@dataclass
class MotionResult:
    motion: bool
    score: float


class MotionDetector:
    """Background-subtraction motion detection on downscaled grayscale frames."""

    def __init__(self, settings: MotionSettings | None = None):
        self.settings = settings or MotionSettings()
        self._bg: np.ndarray | None = None
        self._seen = 0
        self._mask: np.ndarray | None = None
        self._mask_shape: tuple[int, int] | None = None
        self._mask_key: tuple | None = None

    def update_settings(self, settings: MotionSettings) -> None:
        self.settings = settings
        self._mask = None

    def reset(self) -> None:
        self._bg = None
        self._seen = 0

    def _ignore_mask(self, shape: tuple[int, int]) -> np.ndarray | None:
        key = (shape, tuple(map(tuple, self.settings.ignore_rects)))
        if self._mask is not None and self._mask_key == key:
            return self._mask
        if not self.settings.ignore_rects:
            self._mask, self._mask_key = None, key
            return None
        h, w = shape
        mask = np.full(shape, 255, dtype=np.uint8)
        for x, y, rw, rh in self.settings.ignore_rects:
            x0, y0 = int(max(0, x) * w), int(max(0, y) * h)
            x1, y1 = int(min(1, x + rw) * w), int(min(1, y + rh) * h)
            mask[y0:y1, x0:x1] = 0
        self._mask, self._mask_key = mask, key
        return mask

    def process(self, jpeg: bytes) -> MotionResult:
        gray = cv2.imdecode(np.frombuffer(jpeg, np.uint8), cv2.IMREAD_REDUCED_GRAYSCALE_2)
        if gray is None:
            return MotionResult(False, 0.0)
        gray = cv2.GaussianBlur(gray, (21, 21), 0)

        if self._bg is None:
            self._bg = gray.astype(np.float32)
            self._seen = 1
            return MotionResult(False, 0.0)

        bg8 = cv2.convertScaleAbs(self._bg)
        diff = cv2.absdiff(gray, bg8)
        _, changed = cv2.threshold(diff, self.settings.pixel_threshold, 255, cv2.THRESH_BINARY)

        mask = self._ignore_mask(gray.shape)
        if mask is not None:
            changed = cv2.bitwise_and(changed, mask)

        total = changed.size
        fraction = float(np.count_nonzero(changed)) / total

        if fraction > GLOBAL_CHANGE_FRACTION:
            self._bg = gray.astype(np.float32)
            self._seen = 1
            return MotionResult(False, 0.0)

        self._seen += 1
        if self._seen <= WARMUP_FRAMES:
            cv2.accumulateWeighted(gray, self._bg, 0.2)
            return MotionResult(False, 0.0)

        changed = cv2.dilate(changed, None, iterations=2)
        contours, _ = cv2.findContours(changed, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
        largest = max((cv2.contourArea(c) for c in contours), default=0.0)
        score = largest / total
        motion = score >= self.settings.min_area_fraction

        cv2.accumulateWeighted(gray, self._bg, BG_ALPHA_DURING_MOTION if motion else BG_ALPHA)
        return MotionResult(motion, score)
