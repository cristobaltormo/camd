import re
import time
from dataclasses import dataclass
from typing import Any

from .config import Config
from .db import Database
from .detect import COCO_LABELS

HHMM = re.compile(r"^([01]\d|2[0-3]):[0-5]\d$")
DETECTION_MODES = ("off", "alert", "strict")
LANGUAGES = ("en", "es")
MAX_LABELS = 12


@dataclass(frozen=True)
class Pref:
    kind: str
    low: float = 0
    high: float = 0
    options: tuple[str, ...] = ()


def number(low: float, high: float, kind: str = "float") -> Pref:
    return Pref(kind, low, high)


SPEC: dict[str, Pref] = {
    "pre_roll_s": number(0, 10),
    "post_roll_s": number(3, 120),
    "max_event_s": number(15, 600),
    "max_disk_gb": number(0.5, 500),
    "presence_grace_s": number(30, 3600),
    "push_cooldown_s": number(10, 7200),
    "unconfirmed_keep_h": number(1, 720),
    "detect_confidence": number(0.2, 0.9),
    "notify_motion": Pref("bool"),
    "notify_offline": Pref("bool"),
    "quiet_enabled": Pref("bool"),
    "quiet_start": Pref("hhmm"),
    "quiet_end": Pref("hhmm"),
    "camera_enabled": Pref("bool"),
    "camera_vflip": Pref("bool"),
    "camera_hmirror": Pref("bool"),
    "camera_brightness": number(-2, 2, "int"),
    "camera_contrast": number(-2, 2, "int"),
    "camera_quality": number(0, 63, "int"),
    "camera_framesize": Pref("choice_int", options=("0", "5", "8", "9", "10")),
    "detection": Pref("choice", options=DETECTION_MODES),
    "language": Pref("choice", options=LANGUAGES),
    "detect_labels": Pref("labels", options=COCO_LABELS),
}
CAMERA_KEYS = frozenset(key for key in SPEC if key.startswith("camera_") and key != "camera_enabled")
DETECTION_KEYS = frozenset({"detection", "detect_labels", "detect_confidence"})


def validate(key: str, value: Any) -> Any:
    spec = SPEC[key]
    if spec.kind == "bool":
        if not isinstance(value, bool):
            raise ValueError(f"{key} must be true or false")
        return value
    if spec.kind == "hhmm":
        if not isinstance(value, str) or not HHMM.match(value):
            raise ValueError(f"{key} must be HH:MM")
        return value
    if spec.kind == "choice":
        if value not in spec.options:
            raise ValueError(f"{key} must be one of {', '.join(spec.options)}")
        return value
    if spec.kind == "choice_int":
        if isinstance(value, bool) or str(value) not in spec.options:
            raise ValueError(f"{key} must be one of {', '.join(spec.options)}")
        return int(value)
    if spec.kind == "labels":
        if not isinstance(value, list) or not 1 <= len(value) <= MAX_LABELS or any(v not in spec.options for v in value):
            raise ValueError(f"{key} must list between 1 and {MAX_LABELS} known object names")
        return list(dict.fromkeys(value))
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{key} must be a number")
    if not spec.low <= value <= spec.high:
        raise ValueError(f"{key} must be between {spec.low} and {spec.high}")
    return int(value) if spec.kind == "int" else float(value)


def load(cfg: Config, db: Database) -> None:
    for key, value in db.get_setting("prefs", {}).items():
        if key in SPEC:
            try:
                setattr(cfg, key, validate(key, value))
            except ValueError:
                continue


def update(cfg: Config, db: Database, patch: dict[str, Any]) -> set[str]:
    clean = {}
    for key, value in patch.items():
        if key not in SPEC:
            raise ValueError(f"unknown setting: {key}")
        clean[key] = validate(key, value)
    stored = db.get_setting("prefs", {})
    stored.update(clean)
    db.set_setting("prefs", stored)
    for key, value in clean.items():
        setattr(cfg, key, value)
    return set(clean)


def snapshot(cfg: Config) -> dict[str, Any]:
    return {key: getattr(cfg, key) for key in SPEC}


def minutes(hhmm: str) -> int:
    return int(hhmm[:2]) * 60 + int(hhmm[3:])


def in_quiet_hours(cfg: Config, now: float | None = None) -> bool:
    if not cfg.quiet_enabled:
        return False
    local = time.localtime(now if now is not None else time.time())
    current = local.tm_hour * 60 + local.tm_min
    start, end = minutes(cfg.quiet_start), minutes(cfg.quiet_end)
    if start == end:
        return False
    return start <= current < end if start < end else current >= start or current < end
