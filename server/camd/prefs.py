import re
import time
from dataclasses import dataclass
from typing import Any

from .config import Config
from .db import Database

HHMM = re.compile(r"^([01]\d|2[0-3]):[0-5]\d$")


@dataclass(frozen=True)
class Pref:
    kind: type
    low: float | None = None
    high: float | None = None


SPEC: dict[str, Pref] = {
    "pre_roll_s": Pref(float, 0, 10),
    "post_roll_s": Pref(float, 3, 120),
    "max_event_s": Pref(float, 15, 600),
    "max_disk_gb": Pref(float, 0.5, 12),
    "presence_grace_s": Pref(float, 30, 3600),
    "push_cooldown_s": Pref(float, 10, 7200),
    "notify_motion": Pref(bool),
    "notify_offline": Pref(bool),
    "quiet_enabled": Pref(bool),
    "quiet_start": Pref(str),
    "quiet_end": Pref(str),
    "camera_vflip": Pref(bool),
    "camera_hmirror": Pref(bool),
    "camera_brightness": Pref(int, -2, 2),
    "camera_contrast": Pref(int, -2, 2),
    "camera_quality": Pref(int, 0, 63),
}
CAMERA_KEYS = {k for k in SPEC if k.startswith("camera_")}


def validate(key: str, value: Any) -> Any:
    spec = SPEC[key]
    if spec.kind is bool:
        if not isinstance(value, bool):
            raise ValueError(f"{key} must be true or false")
        return value
    if spec.kind is str:
        if not isinstance(value, str) or not HHMM.match(value):
            raise ValueError(f"{key} must be HH:MM")
        return value
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{key} must be a number")
    if not spec.low <= value <= spec.high:
        raise ValueError(f"{key} must be between {spec.low} and {spec.high}")
    return spec.kind(value)


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
