import os
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

DEFAULT_PATH = "/etc/habitacion-cam/config.toml"


@dataclass
class Config:
    camera_host: str = "192.168.1.93"
    camera_token: str = ""
    data_dir: Path = Path("/var/lib/habitacion-cam")
    listen_host: str = "127.0.0.1"
    listen_port: int = 8180
    rp_id: str = "localhost"
    origin: str = "http://localhost:8180"
    secure_cookies: bool = False
    trust_proxy: bool = False
    push_subject: str = "https://localhost"
    notifich_url: str = ""
    notifich_key: str = ""
    mqtt_host: str = "127.0.0.1"
    mqtt_port: int = 1883
    mqtt_topic: str = "habitacion-cam"
    presence_devices: list[str] = field(default_factory=list)
    presence_grace_s: int = 300
    retention_days: int = 14
    max_disk_gb: float = 6.0
    pre_roll_s: float = 3.0
    post_roll_s: float = 10.0
    max_event_s: float = 90.0
    push_cooldown_s: float = 60.0

    @property
    def clips_dir(self) -> Path:
        return self.data_dir / "clips"

    @property
    def db_path(self) -> Path:
        return self.data_dir / "camd.sqlite3"

    @property
    def vapid_path(self) -> Path:
        return self.data_dir / "vapid_private.pem"


def load(path: str | None = None) -> Config:
    path = path or os.environ.get("CAMD_CONFIG", DEFAULT_PATH)
    raw = {}
    if Path(path).exists():
        with open(path, "rb") as fh:
            raw = tomllib.load(fh)

    cfg = Config()
    for key, value in raw.items():
        if not hasattr(cfg, key):
            raise ValueError(f"unknown config key: {key}")
        setattr(cfg, key, Path(value) if key == "data_dir" else value)
    cfg.camera_token = os.environ.get("CAMD_CAMERA_TOKEN", cfg.camera_token)
    cfg.notifich_key = os.environ.get("CAMD_NOTIFICH_KEY", cfg.notifich_key)
    return cfg
