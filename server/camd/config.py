import os
import tomllib
from dataclasses import dataclass, field, fields
from pathlib import Path

DEFAULT_PATH = "/etc/camd/config.toml"
ENV_PREFIX = "CAMD_"
TRUE_WORDS = {"1", "true", "yes", "on"}


@dataclass
class Config:
    camera_host: str = "192.168.1.93"
    camera_api_port: int = 80
    camera_stream_port: int = 81
    camera_token: str = ""
    data_dir: Path = Path("/var/lib/camd")
    listen_host: str = "127.0.0.1"
    listen_port: int = 8180
    site_name: str = "Room camera"
    language: str = "en"
    rp_id: str = "localhost"
    origin: str = "http://localhost:8180"
    secure_cookies: bool = False
    trust_proxy: bool = False
    push_subject: str = "https://localhost"
    notifich_url: str = ""
    notifich_key: str = ""
    ntfy_url: str = ""
    ntfy_token: str = ""
    mqtt_host: str = ""
    mqtt_port: int = 1883
    mqtt_topic: str = "camd"
    mqtt_discovery: bool = True
    presence_devices: list[str] = field(default_factory=list)
    presence_grace_s: float = 300.0
    retention_days: int = 14
    max_disk_gb: float = 6.0
    pre_roll_s: float = 3.0
    post_roll_s: float = 10.0
    max_event_s: float = 90.0
    push_cooldown_s: float = 60.0
    notify_motion: bool = True
    notify_offline: bool = True
    quiet_enabled: bool = False
    quiet_start: str = "23:00"
    quiet_end: str = "07:00"
    detection: str = "alert"
    detect_labels: list[str] = field(default_factory=lambda: ["person", "cat", "dog"])
    detect_confidence: float = 0.5
    unconfirmed_keep_h: float = 24.0
    camera_enabled: bool = True
    camera_vflip: bool = False
    camera_hmirror: bool = False
    camera_brightness: int = 0
    camera_contrast: int = 0
    camera_quality: int = 0
    camera_framesize: int = 0

    @property
    def clips_dir(self) -> Path:
        return self.data_dir / "clips"

    @property
    def photos_dir(self) -> Path:
        return self.data_dir / "photos"

    @property
    def spool_dir(self) -> Path:
        return self.data_dir / "spool"

    @property
    def models_dir(self) -> Path:
        return self.data_dir / "models"

    @property
    def db_path(self) -> Path:
        return self.data_dir / "camd.sqlite3"

    @property
    def vapid_path(self) -> Path:
        return self.data_dir / "vapid_private.pem"


def coerce(raw: str, template):
    if isinstance(template, bool):
        return raw.strip().lower() in TRUE_WORDS
    if isinstance(template, Path):
        return Path(raw)
    if isinstance(template, list):
        return [item.strip() for item in raw.split(",") if item.strip()]
    return type(template)(raw)


def load(path: str | None = None) -> Config:
    path = path or os.environ.get("CAMD_CONFIG", DEFAULT_PATH)
    raw: dict = {}
    if Path(path).exists():
        with open(path, "rb") as fh:
            raw = tomllib.load(fh)

    known = {f.name for f in fields(Config)}
    unknown = sorted(set(raw) - known)
    if unknown:
        raise ValueError(f"unknown config keys: {', '.join(unknown)}")

    cfg = Config(**{key: Path(value) if key == "data_dir" else value for key, value in raw.items()})
    for name in known:
        override = os.environ.get(ENV_PREFIX + name.upper())
        if override is not None:
            setattr(cfg, name, coerce(override, getattr(cfg, name)))
    return cfg
