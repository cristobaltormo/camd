import logging
import threading
import time

from . import prefs
from .arming import Arming
from .config import Config
from .db import Database
from .detect import Detection, DetectionWorker, in_zones
from .events import EventRecorder, event_json
from .grabber import CameraClient, Grabber
from .messages import label_names, translate
from .motion import MotionDetector, MotionSettings
from .mqtt import Commands, Mqtt
from .photos import Photos
from .presence import Presence
from .push import Notifier
from .storage import PURGE_INTERVAL_S, purge

log = logging.getLogger("camd.runtime")

OFFLINE_ALERT_S = 300
DECISION_CACHE_S = 1.0
MAINTENANCE_INTERVAL_S = 10
CAMERA_STATUS_INTERVAL_S = 30
DETECT_ACTIVE_INTERVAL_S = 0.7
DETECT_IDLE_INTERVAL_S = 2.5
DETECTIONS_FRESH_S = 3.0


class Runtime:
    def __init__(self, cfg: Config, db: Database):
        self.cfg = cfg
        self.db = db
        stored_retention = db.get_setting("retention_days")
        if stored_retention:
            cfg.retention_days = int(stored_retention)
        prefs.load(cfg, db)

        self.presence = Presence(cfg.presence_devices, cfg.presence_grace_s)
        self.arming = Arming(db, self.presence)
        self.photos = Photos(cfg, db)
        self.notifier = Notifier(cfg, db)
        self.mqtt = Mqtt(cfg, Commands(self._external_presence, self.set_mode, self.set_privacy))
        self.detector = MotionDetector(self.load_motion_settings())
        self.analyzer = DetectionWorker(cfg, self._on_detections)
        self.recorder = EventRecorder(cfg, db, self._analysis_ready, self._event_started, self._event_confirmed, self._event_finished)
        self.camera = CameraClient(cfg.camera_host, cfg.camera_token, cfg.camera_api_port, cfg.camera_stream_port)
        self.grabber = Grabber(self.camera, self._on_frame, self.apply_camera)

        self.motion_now = False
        self.last_detections: tuple[float, list[Detection]] = (0.0, [])
        self.camera_info: dict | None = None
        self._started_at = time.time()
        self._enabled_at = 0.0
        self._decision = (False, "init")
        self._decision_at = 0.0
        self._last_push = 0.0
        self._last_detect = 0.0
        self._offline_alerted = False
        self._halt = threading.Event()
        self._maintenance = threading.Thread(target=self._maintain, daemon=True, name="maintenance")

    def load_motion_settings(self) -> MotionSettings:
        raw = self.db.get_setting("motion", {})
        return MotionSettings(
            sensitivity=int(raw.get("sensitivity", 5)),
            ignore_rects=[list(map(float, rect)) for rect in raw.get("ignore_rects", [])],
            alert_rects=[list(map(float, rect)) for rect in raw.get("alert_rects", [])],
        )

    def save_motion_settings(self, sensitivity: int, ignore_rects: list[list[float]], alert_rects: list[list[float]]) -> None:
        self.db.set_setting("motion", {"sensitivity": sensitivity, "ignore_rects": ignore_rects, "alert_rects": alert_rects})
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
        if cfg.camera_framesize:
            controls["framesize"] = cfg.camera_framesize
        return self.camera.apply_controls(controls)

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
        if changed & prefs.DETECTION_KEYS:
            self.analyzer.poke()

    def start(self) -> None:
        if not self.cfg.camera_host or not self.cfg.camera_token:
            log.warning("camera_host or the camera token is empty: set camera_host and CAMD_CAMERA_TOKEN")
        self.recorder.recover()
        self.presence.start()
        self.mqtt.start()
        self.analyzer.start()
        self.grabber.set_enabled(self.cfg.camera_enabled)
        if self.arming.privacy and self.cfg.camera_enabled:
            self.grabber.set_standby(True)
        self.grabber.start()
        self._maintenance.start()

    def stop(self) -> None:
        self._halt.set()
        self.grabber.stop()
        self.analyzer.stop()
        self.presence.stop()
        self.recorder.close_open_event()
        self.recorder.flush()
        self.mqtt.stop()

    def set_mode(self, mode: str) -> None:
        self.arming.set_mode(mode)
        self.arming.snooze(0)
        self._decision_at = 0.0

    def set_privacy(self, on: bool) -> bool:
        if not self.grabber.set_standby(on):
            return False
        self.arming.set_privacy(on)
        self._decision_at = 0.0
        if on:
            self.recorder.close_open_event()
            self.detector.reset()
        return True

    def _external_presence(self, home: bool | None) -> None:
        self.presence.external = home
        if home:
            self.presence.mark_seen()

    def _analysis_ready(self) -> bool:
        return self.cfg.detection != "off" and self.analyzer.ready

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
            self._request_analysis(ts, jpeg, result.motion)
            self.recorder.feed(ts, jpeg, result.motion, result.score, self._armed())
        except Exception:
            log.exception("frame pipeline error")

    def _request_analysis(self, ts: float, jpeg: bytes, motion: bool) -> None:
        if not self._analysis_ready() or not self._armed():
            return
        if motion:
            gap = DETECT_ACTIVE_INTERVAL_S
        elif self.recorder.recording:
            gap = DETECT_IDLE_INTERVAL_S
        else:
            return
        if ts - self._last_detect >= gap and self.analyzer.submit(ts, jpeg):
            self._last_detect = ts

    def _on_detections(self, ts: float, jpeg: bytes, found: list[Detection]) -> None:
        zones = self.detector.settings.alert_rects
        found = [d for d in found if in_zones(d, zones)]
        self.last_detections = (ts, found)
        self.mqtt.publish("object", "ON" if found else "OFF")
        self.recorder.observe(ts, jpeg, found)

    def _may_notify(self, now: float) -> bool:
        if not self.cfg.notify_motion or prefs.in_quiet_hours(self.cfg, now):
            return False
        if now - self._last_push < self.cfg.push_cooldown_s:
            return False
        self._last_push = now
        return True

    def _notify(self, title: str, body: str, event_id: int, tag: str) -> None:
        threading.Thread(target=self.notifier.send, args=(title, body, f"/#/events/{event_id}", tag), daemon=True, name="notify").start()

    def _event_started(self, event_id: int, start_ts: float, analyzed: bool) -> None:
        self.mqtt.publish("motion", "ON")
        if analyzed or not self._may_notify(time.time()):
            return
        language = self.cfg.language
        self._notify(
            translate(language, "motion_title"),
            translate(language, "at_time", time=time.strftime("%H:%M", time.localtime(start_ts))),
            event_id,
            "motion",
        )

    def _event_confirmed(self, event_id: int, start_ts: float, labels: list[str]) -> None:
        self.mqtt.publish("object", "ON")
        if not self._may_notify(time.time()):
            return
        language = self.cfg.language
        title = translate(language, "object_title", labels=label_names(language, labels))
        self._notify(title, translate(language, "at_time", time=time.strftime("%H:%M", time.localtime(start_ts))), event_id, "motion")

    def _event_finished(self, _event_id: int) -> None:
        if not self.recorder.recording:
            self.mqtt.publish("motion", "OFF")
            self.mqtt.publish("object", "OFF")
        purge(self.cfg, self.db)

    def _maintain(self) -> None:
        last_purge = last_status = 0.0
        while not self._halt.wait(MAINTENANCE_INTERVAL_S):
            try:
                now = time.time()
                armed, _ = self.arming.decision()
                self.mqtt.publish("armed", "ON" if armed else "OFF")
                self.mqtt.publish("mode", self.arming.mode)
                self.mqtt.publish("privacy", "ON" if self.arming.privacy else "OFF")
                self.mqtt.publish("camera", "online" if self.grabber.online else "offline")

                if self.cfg.camera_enabled and now - last_status >= CAMERA_STATUS_INTERVAL_S and (self.grabber.online or self.grabber.standby):
                    last_status = now
                    self.camera_info = self.camera.status()
                    self._enforce_standby()

                self.recorder.tick()
                self._check_offline()
                if now - last_purge > PURGE_INTERVAL_S:
                    purge(self.cfg, self.db)
                    last_purge = now
            except Exception:
                log.exception("maintenance cycle failed")

    def _enforce_standby(self) -> None:
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
        language = self.cfg.language
        if self.grabber.standby or silent_for < OFFLINE_ALERT_S:
            if self.grabber.online and self._offline_alerted:
                self._offline_alerted = False
                if self.cfg.notify_offline:
                    self.notifier.send(translate(language, "back_title"), translate(language, "back_body"), "/", "offline")
            return
        if not self._offline_alerted:
            self._offline_alerted = True
            if self.cfg.notify_offline:
                self.notifier.send(translate(language, "offline_title"), translate(language, "offline_body"), "/", "offline")

    def status(self, version: str, build: str) -> dict:
        armed, reason = self.arming.decision()
        last = self.db.one("SELECT * FROM events WHERE status='done' AND (analyzed=0 OR confirmed=1) ORDER BY start_ts DESC LIMIT 1")
        info = self.camera_info or {}
        usage = self.db.row("SELECT COUNT(*) AS n, COALESCE(SUM(size_bytes), 0) AS bytes FROM events")
        photo_count, photo_bytes = self.photos.usage()
        detected_at, detected = self.last_detections
        fresh = time.time() - detected_at < DETECTIONS_FRESH_S
        return {
            "armed": armed,
            "reason": reason,
            "mode": self.arming.mode,
            "snooze_until": self.arming.snooze_until,
            "privacy": self.arming.privacy,
            "presence": {"configured": self.presence.configured, "home": self.presence.is_home},
            "camera": {
                "enabled": self.cfg.camera_enabled,
                "online": self.grabber.online,
                "standby": self.grabber.standby,
                "rssi": info.get("rssi"),
                "fw": info.get("fw"),
                "uptime_s": info.get("uptime_s"),
                "fps": round(self.grabber.fps, 1),
            },
            "motion": self.motion_now,
            "recording": self.recorder.recording,
            "last_event": event_json(last) if last else None,
            "detection": {
                "mode": self.cfg.detection,
                "state": self.analyzer.state if self.cfg.detection != "off" else "off",
                "error": self.analyzer.error,
                "inference_ms": round(self.analyzer.last_ms, 1),
                "items": [d.as_dict() for d in detected] if fresh else [],
            },
            "push_subscriptions": self.notifier.web.count(),
            "channels": [channel.name for channel in self.notifier.channels],
            "version": version,
            "build": build,
            "storage": {
                "events": usage["n"],
                "bytes": usage["bytes"],
                "cap_bytes": int(self.cfg.max_disk_gb * 1024**3),
                "photos": photo_count,
                "photo_bytes": photo_bytes,
            },
            "clip": self.recorder.manual_state(),
        }
