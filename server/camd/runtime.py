import logging
import threading
import time

from .arming import Arming
from .config import Config
from .db import Database
from .events import EventRecorder, purge
from . import prefs
from .grabber import Grabber
from .motion import MotionDetector, MotionSettings
from .mqtt import Mqtt
from .photos import Photos
from .presence import Presence
from .push import Push

log = logging.getLogger("camd.runtime")

OFFLINE_ALERT_S = 300
DECISION_CACHE_S = 1.0


class Runtime:
    """Owns every long-lived component and the per-frame pipeline."""

    def __init__(self, cfg: Config, db: Database):
        self.cfg = cfg
        self.db = db
        stored = db.get_setting("retention_days")
        if stored:
            cfg.retention_days = int(stored)
        prefs.load(cfg, db)

        self.presence = Presence(cfg.presence_devices, cfg.presence_grace_s)
        self.arming = Arming(db, self.presence)
        self.photos = Photos(cfg, db)
        self.push = Push(db, cfg.vapid_path, cfg.push_subject, cfg.notifich_url, cfg.notifich_key)
        self.mqtt = Mqtt(cfg, self._external_presence)
        self.detector = MotionDetector(self.load_motion_settings())
        self.recorder = EventRecorder(cfg, db, on_start=self._event_started, on_finish=self._event_finished)
        self.grabber = Grabber(cfg.camera_host, cfg.camera_token, self._on_frame, self.apply_camera)

        self.motion_now = False
        self._started_at = time.time()
        self._enabled_at = 0.0
        self.camera_info: dict | None = None
        self._decision = (False, "init")
        self._decision_at = 0.0
        self._last_push = 0.0
        self._offline_alerted = False
        self._halt = threading.Event()
        self._maintenance = threading.Thread(target=self._maintain, daemon=True, name="maintenance")

    def load_motion_settings(self) -> MotionSettings:
        raw = self.db.get_setting("motion", {})
        return MotionSettings(
            sensitivity=int(raw.get("sensitivity", 5)),
            ignore_rects=[list(map(float, r)) for r in raw.get("ignore_rects", [])],
        )

    def save_motion_settings(self, sensitivity: int, ignore_rects: list[list[float]]) -> None:
        self.db.set_setting("motion", {"sensitivity": sensitivity, "ignore_rects": ignore_rects})
        self.detector.update_settings(self.load_motion_settings())

    def apply_camera(self) -> dict[str, bool]:
        if self.grabber.standby:
            return {}
        cfg = self.cfg
        controls = {
            "vflip": cfg.camera_vflip,
            "hmirror": cfg.camera_hmirror,
            "brightness": cfg.camera_brightness,
            "contrast": cfg.camera_contrast,
        }
        if cfg.camera_quality:
            controls["quality"] = cfg.camera_quality
        return self.grabber.apply_controls(controls)

    def apply_prefs(self, changed: set[str]) -> None:
        self.presence.grace_s = self.cfg.presence_grace_s
        if "camera_enabled" in changed:
            self.grabber.set_enabled(self.cfg.camera_enabled)
            self._enabled_at = time.time()
            if not self.cfg.camera_enabled:
                self.recorder.close_open_event()
                self.detector.reset()
                self._offline_alerted = False
        if changed & prefs.CAMERA_KEYS:
            threading.Thread(target=self.apply_camera, daemon=True, name="apply-camera").start()

    def start(self) -> None:
        self.presence.start()
        self.mqtt.start()
        self.grabber.set_enabled(self.cfg.camera_enabled)
        if self.arming.privacy and self.cfg.camera_enabled:
            self.grabber.set_standby(True)
        self.grabber.start()
        self._maintenance.start()

    def stop(self) -> None:
        self._halt.set()
        self.grabber.stop()
        self.presence.stop()
        self.recorder.close_open_event()
        self.recorder.flush()
        self.mqtt.stop()

    def set_privacy(self, on: bool) -> bool:
        if not self.grabber.set_standby(on):
            return False
        self.arming.set_privacy(on)
        if on:
            self.recorder.close_open_event()
            self.detector.reset()
        return True

    def _external_presence(self, home: bool | None) -> None:
        self.presence.external = home
        if home:
            self.presence.mark_seen()

    def _armed(self) -> bool:
        now = time.time()
        if now - self._decision_at > DECISION_CACHE_S:
            self._decision = self.arming.decision()
            self._decision_at = now
        return self._decision[0]

    def _on_frame(self, ts: float, jpeg: bytes) -> None:
        try:
            result = self.detector.process(jpeg)
            self.motion_now = result.motion
            self.recorder.feed(ts, jpeg, result.motion, result.score, self._armed())
        except Exception:
            log.exception("frame pipeline error")

    def _event_started(self, event_id: int, start_ts: float) -> None:
        self.mqtt.publish("motion", "ON")
        now = time.time()
        if not self.cfg.notify_motion or prefs.in_quiet_hours(self.cfg, now):
            return
        if now - self._last_push < self.cfg.push_cooldown_s:
            return
        self._last_push = now
        threading.Thread(
            target=self.push.send,
            args=("Movimiento en la habitación", "Detectado a las " + time.strftime("%H:%M", time.localtime(start_ts)), f"/#/events/{event_id}", "motion"),
            daemon=True,
        ).start()

    def _event_finished(self, _event_id: int) -> None:
        if not self.recorder.recording:
            self.mqtt.publish("motion", "OFF")
        purge(self.cfg, self.db)

    def _maintain(self) -> None:
        last_purge = 0.0
        while not self._halt.wait(10):
            try:
                armed, reason = self.arming.decision()
                self.mqtt.publish("armed", "ON" if armed else "OFF")
                self.mqtt.publish("mode", self.arming.mode)
                self.mqtt.publish("camera", "online" if self.grabber.online else "offline")

                if self.cfg.camera_enabled and int(time.time()) % 30 < 10 and (self.grabber.online or self.grabber.standby):
                    self.camera_info = self.grabber.fetch_status()
                    self._enforce_standby()

                self.recorder.tick()
                self._check_offline()
                if time.time() - last_purge > 3600:
                    purge(self.cfg, self.db)
                    last_purge = time.time()
            except Exception:
                log.exception("maintenance cycle failed")

    def _enforce_standby(self) -> None:
        """A power-cycled camera wakes up with its sensor on; honour a privacy setting made before that."""
        info = self.camera_info
        if self.grabber.standby and info is not None and not info.get("standby"):
            log.info("camera woke up while privacy is on, putting it back in standby")
            self.grabber.set_standby(True)

    def _check_offline(self) -> None:
        if not self.cfg.camera_enabled:
            self._offline_alerted = False
            return
        latest = self.grabber.latest
        silent_for = time.time() - max(latest[0] if latest else 0.0, self._started_at, self._enabled_at)
        if self.grabber.standby or silent_for < OFFLINE_ALERT_S:
            if self.grabber.online and self._offline_alerted:
                self._offline_alerted = False
                if self.cfg.notify_offline:
                    self.push.send("Cámara de vuelta", "Vuelven a llegar imágenes", "/", "offline")
            return
        if not self._offline_alerted:
            self._offline_alerted = True
            if not self.cfg.notify_offline:
                return
            self.push.send("Cámara sin conexión", "No llegan imágenes desde hace 5 minutos. Si la has desenchufado, desactívala en Ajustes, Cámara.", "/", "offline")
