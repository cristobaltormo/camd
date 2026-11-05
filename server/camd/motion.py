from dataclasses import dataclass, field

import cv2
import numpy as np

WARMUP_FRAMES = 20
GLOBAL_CHANGE_FRACTION = 0.5
BACKGROUND_RATE = 0.05
BACKGROUND_RATE_DURING_MOTION = 0.01
WARMUP_RATE = 0.2
BLUR_KERNEL = (21, 21)
DILATE_ITERATIONS = 2


@dataclass
class MotionSettings:
    sensitivity: int = 5
    ignore_rects: list[list[float]] = field(default_factory=list)
    alert_rects: list[list[float]] = field(default_factory=list)

    @property
    def pixel_threshold(self) -> int:
        return max(8, 40 - 3 * self.sensitivity)

    @property
    def min_area_fraction(self) -> float:
        return 0.0015 * (11 - self.sensitivity)


@dataclass(frozen=True)
class MotionResult:
    motion: bool
    score: float


QUIET = MotionResult(False, 0.0)


class MotionDetector:
    def __init__(self, settings: MotionSettings | None = None):
        self.settings = settings or MotionSettings()
        self._background: np.ndarray | None = None
        self._seen = 0
        self._mask: np.ndarray | None = None
        self._mask_key: tuple | None = None

    def update_settings(self, settings: MotionSettings) -> None:
        self.settings = settings
        self._mask = None
        self._mask_key = None

    def reset(self) -> None:
        self._background = None
        self._seen = 0

    def _ignore_mask(self, shape: tuple[int, int]) -> np.ndarray | None:
        key = (shape, tuple(map(tuple, self.settings.ignore_rects)))
        if self._mask_key == key:
            return self._mask
        self._mask_key = key
        if not self.settings.ignore_rects:
            self._mask = None
            return None
        height, width = shape
        mask = np.full(shape, 255, dtype=np.uint8)
        for x, y, w, h in self.settings.ignore_rects:
            mask[int(max(0, y) * height) : int(min(1, y + h) * height), int(max(0, x) * width) : int(min(1, x + w) * width)] = 0
        self._mask = mask
        return mask

    def _restart(self, gray: np.ndarray) -> MotionResult:
        self._background = gray.astype(np.float32)
        self._seen = 1
        return QUIET

    def process(self, jpeg: bytes) -> MotionResult:
        gray = cv2.imdecode(np.frombuffer(jpeg, np.uint8), cv2.IMREAD_REDUCED_GRAYSCALE_2)
        if gray is None:
            return QUIET
        gray = cv2.GaussianBlur(gray, BLUR_KERNEL, 0)
        if self._background is None or self._background.shape != gray.shape:
            return self._restart(gray)

        gain = float(np.median(gray - self._background))
        reference = np.clip(self._background + gain, 0, 255).astype(np.uint8)
        changed = cv2.threshold(cv2.absdiff(gray, reference), self.settings.pixel_threshold, 255, cv2.THRESH_BINARY)[1]
        mask = self._ignore_mask(gray.shape)
        if mask is not None:
            changed = cv2.bitwise_and(changed, mask)

        if np.count_nonzero(changed) / changed.size > GLOBAL_CHANGE_FRACTION:
            return self._restart(gray)

        self._seen += 1
        if self._seen <= WARMUP_FRAMES:
            cv2.accumulateWeighted(gray, self._background, WARMUP_RATE)
            return QUIET

        contours, _ = cv2.findContours(cv2.dilate(changed, None, iterations=DILATE_ITERATIONS), cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
        score = max((cv2.contourArea(c) for c in contours), default=0.0) / changed.size
        motion = score >= self.settings.min_area_fraction
        cv2.accumulateWeighted(gray, self._background, BACKGROUND_RATE_DURING_MOTION if motion else BACKGROUND_RATE)
        return MotionResult(motion, score)
