import json
import os
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

WEB = Path(__file__).resolve().parent.parent / "server" / "camd" / "web"
SCENES = Path(os.environ["UI_MOCK_SCENES"]) if os.environ.get("UI_MOCK_SCENES") else None
FALLBACK = (WEB / "icon-512.png").read_bytes()
LOCK = threading.Lock()
NOW = time.time()
WAKE_DELAY_S = 2.0
PERSON_BOX, PERSON_CONFIDENCE = [0.37, 0.27, 0.16, 0.67], 0.87
if SCENES and (SCENES / "detections.json").is_file():
    FOUND = json.loads((SCENES / "detections.json").read_text())[0]
    PERSON_BOX, PERSON_CONFIDENCE = FOUND["box"], FOUND["confidence"]
SECOND_LABEL = "person" if SCENES else "cat"
EXTRA = {"labels": ("person",), "analyzed": True} if SCENES else {}
THUMBS = {1: "full", 2: "centre", 3: "full", 5: "centre", 6: "right", 7: "shelf"}
PHOTO_THUMBS = {1: "full", 2: "centre", 3: "right"}


def scene(name):
    if SCENES and (SCENES / f"{name}.jpg").is_file():
        return (SCENES / f"{name}.jpg").read_bytes(), "image/jpeg"
    return FALLBACK, "image/png"


DETECTING = bool(os.environ.get("UI_MOCK_DETECT"))
LABELS = ["person", "bicycle", "car", "cat", "dog", "bird", "chair", "bottle"]
CONTENT_TYPES = {".js": "text/javascript", ".css": "text/css", ".png": "image/png", ".svg": "image/svg+xml"}


def event(event_id, age, duration, status="done", kind="motion", keep=False, labels=(), analyzed=False):
    boxes = [{"label": labels[0], "confidence": PERSON_CONFIDENCE, "box": PERSON_BOX}] if labels else []
    return {
        "id": event_id,
        "start": NOW - age,
        "end": NOW - age + duration,
        "duration": duration,
        "status": status,
        "score": 0.05,
        "size": 1_000_000,
        "kind": kind,
        "keep": keep,
        "analyzed": analyzed,
        "labels": list(labels),
        "detections": boxes,
    }


STATE = {
    "privacy": False,
    "enabled": True,
    "wake_at": 0.0,
    "mode": "auto",
    "snooze_until": 0,
    "clip": {"active": False, "id": None, "until": 0, "started": 0},
    "events": [
        event(1, 600, 23, labels=("person",), analyzed=True),
        event(2, 4000, 61, kind="manual", keep=True),
        event(3, 9000, 14, labels=(SECOND_LABEL,), analyzed=True),
        event(4, 40000, 35, status="failed"),
        event(5, 90000, 48, **EXTRA),
        event(6, 95000, 12, **EXTRA),
        event(7, 300, 9, analyzed=True),
    ],
    "photos": [{"id": i, "ts": NOW - age, "size": 90000} for i, age in enumerate([300, 7000, 80000], 1)],
    "settings": {
        "sensitivity": 5,
        "ignore_rects": [[0.1, 0.1, 0.2, 0.2]],
        "alert_rects": [],
        "retention_days": 14,
        "pre_roll_s": 3.0,
        "post_roll_s": 10.0,
        "max_event_s": 90.0,
        "max_disk_gb": 6.0,
        "presence_grace_s": 300.0,
        "push_cooldown_s": 60.0,
        "unconfirmed_keep_h": 24.0,
        "detect_confidence": 0.5,
        "notify_motion": True,
        "notify_offline": True,
        "quiet_enabled": True,
        "quiet_start": "23:00",
        "quiet_end": "07:00",
        "detection": "alert",
        "detect_labels": ["person", "cat", "dog"],
        "language": "en",
        "camera_enabled": True,
        "camera_vflip": False,
        "camera_hmirror": False,
        "camera_brightness": 0,
        "camera_contrast": 0,
        "camera_quality": 0,
        "available_labels": LABELS,
    },
}
NEXT_ID = {"photo": 100}


def status():
    online = STATE["enabled"] and not STATE["privacy"] and time.time() >= STATE["wake_at"]
    clip = STATE["clip"]
    if clip["active"] and time.time() >= clip["until"]:
        clip.update(active=False, id=None)
    visible = [e for e in STATE["events"] if not (e["analyzed"] and not e["labels"])]
    return {
        "version": "mock",
        "armed": True,
        "reason": "away",
        "mode": STATE["mode"],
        "snooze_until": STATE["snooze_until"],
        "privacy": STATE["privacy"],
        "presence": {"configured": True, "home": False},
        "camera": {"enabled": STATE["enabled"], "online": online, "standby": STATE["privacy"], "rssi": -52, "fw": "mock", "uptime_s": 190000, "fps": 12.4},
        "motion": DETECTING,
        "recording": clip["active"],
        "last_event": visible[0] if visible else None,
        "detection": {
            "mode": STATE["settings"]["detection"],
            "state": "ready",
            "error": "",
            "inference_ms": 48.0,
            "items": [{"label": "person", "confidence": PERSON_CONFIDENCE, "box": PERSON_BOX}] if DETECTING else [],
        },
        "push_subscriptions": 0,
        "channels": ["ntfy"],
        "build": "dev",
        "storage": {"events": len(STATE["events"]), "bytes": 612_000_000, "cap_bytes": 6 * 1024**3, "photos": len(STATE["photos"]), "photo_bytes": 270000},
        "clip": clip,
    }


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        return

    def send(self, body, content_type="application/json", code=200):
        if isinstance(body, (dict, list)):
            body = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def payload(self):
        length = int(self.headers.get("Content-Length") or 0)
        return json.loads(self.rfile.read(length) or b"{}") if length else {}

    def do_GET(self):
        parsed = urlparse(self.path)
        path = parsed.path
        query = {key: float(values[0]) for key, values in parse_qs(parsed.query).items() if key in ("after", "before")}
        with LOCK:
            routes = {
                "/api/auth/state": {"logged_in": True, "passkeys": True, "password": False, "site_name": "Bedroom", "language": "en"},
                "/api/status": status(),
                "/api/events": {"events": [e for e in STATE["events"] if query.get("after", 0) <= e["start"] < query.get("before", 1e18)]},
                "/api/events/count": {"count": 2},
                "/api/photos": {"photos": STATE["photos"]},
                "/api/settings": STATE["settings"],
                "/api/push/key": {"key": "x"},
                "/api/auth/devices": {
                    "credentials": [{"id": "a", "name": "iPhone · Safari"}],
                    "sessions": [{"id": 1, "user_agent": "Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) Version/27 Mobile Safari/604.1"}],
                    "current_session": 1,
                    "password": False,
                },
            }
            if path in routes:
                return self.send(routes[path])
            if path == "/api/live" and (STATE["privacy"] or not STATE["enabled"]):
                return self.send({"detail": "camera in standby"}, code=503)
        if path in ("/api/snapshot", "/api/live"):
            return self.send(*scene("live"))
        if path.endswith("/img"):
            return self.send(*scene("live"))
        if path.endswith("/thumb"):
            parts = path.split("/")
            names = PHOTO_THUMBS if parts[2] == "photos" else THUMBS
            return self.send(*scene(names.get(int(parts[3]), "full")))
        if path.endswith("/clip"):
            clip = SCENES / "clip.webm" if SCENES else None
            if clip and clip.is_file():
                return self.send(clip.read_bytes(), "video/webm")
            return self.send(b"", "video/mp4")
        if path == "/":
            return self.send((WEB / "index.html").read_text().replace("{{site_name}}", "Bedroom").replace("{{lang}}", "en").encode(), "text/html")
        file = WEB / path.lstrip("/")
        if file.is_file():
            return self.send(file.read_bytes(), CONTENT_TYPES.get(file.suffix, "application/octet-stream"))
        self.send({"detail": "not found"}, code=404)

    def do_POST(self):
        path, body = urlparse(self.path).path, self.payload()
        with LOCK:
            handled = self.post(path, body)
        self.send(handled if handled is not None else {"ok": True})

    def post(self, path, body):
        if path == "/api/privacy":
            STATE["privacy"] = body["on"]
            if not body["on"]:
                STATE["wake_at"] = time.time() + WAKE_DELAY_S
        elif path == "/api/mode":
            STATE["mode"] = body["mode"]
        elif path == "/api/snooze":
            STATE["snooze_until"] = time.time() + body["minutes"] * 60 if body["minutes"] else 0
        elif path == "/api/clips/start":
            STATE["clip"] = {"active": True, "id": 99, "until": time.time() + body["seconds"], "started": time.time()}
            return STATE["clip"]
        elif path == "/api/clips/stop":
            STATE["clip"].update(active=False, id=None)
        elif path == "/api/photos":
            NEXT_ID["photo"] += 1
            STATE["photos"].insert(0, {"id": NEXT_ID["photo"], "ts": time.time(), "size": 90000})
            return {"id": NEXT_ID["photo"], "ts": time.time(), "size": 90000}
        elif path == "/api/photos/delete":
            STATE["photos"] = [p for p in STATE["photos"] if p["id"] not in body["ids"]]
            return {"removed": len(body["ids"])}
        elif path == "/api/events/delete":
            STATE["events"] = [e for e in STATE["events"] if e["id"] not in body["ids"]]
            return {"removed": len(body["ids"])}
        elif path.startswith("/api/events/") and path.endswith("/keep"):
            for item in STATE["events"]:
                if item["id"] == int(path.split("/")[3]):
                    item["keep"] = body["keep"]
            return {"ok": True, "keep": body["keep"]}
        elif path == "/api/settings":
            if "camera_enabled" in body:
                STATE["enabled"] = body["camera_enabled"]
                if body["camera_enabled"]:
                    STATE["wake_at"] = time.time() + WAKE_DELAY_S
            STATE["settings"].update(body)
        elif path == "/api/push/test":
            return {"sent": 1}
        return None

    def do_DELETE(self):
        parts = urlparse(self.path).path.split("/")
        with LOCK:
            if parts[2:3] == ["events"]:
                STATE["events"] = [e for e in STATE["events"] if e["id"] != int(parts[3])]
            elif parts[2:3] == ["photos"]:
                STATE["photos"] = [p for p in STATE["photos"] if p["id"] != int(parts[3])]
        self.send({"ok": True})


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", 8199), Handler).serve_forever()
