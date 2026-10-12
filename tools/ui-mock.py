"""Stateful fake API for the PWA so the UI can be screenshotted and tested without the camera.

    python tools/ui-mock.py          # serves http://127.0.0.1:8199/
    node --experimental-websocket tools/ui-e2e.mjs
"""
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

WEB = Path(__file__).resolve().parent.parent / "server" / "camd" / "web"
ICON = (WEB / "icon-512.png").read_bytes()
LOCK = threading.Lock()
now = time.time()

STATE = {
    "privacy": False,
    "wake_at": 0.0,
    "mode": "auto",
    "snooze_until": 0,
    "clip": {"active": False, "id": None, "until": 0, "started": 0},
    "events": [
        {"id": i, "start": now - d, "end": now - d + dur, "duration": dur, "status": st, "score": 5, "size": 1000000, "kind": kind, "keep": keep}
        for i, (d, dur, st, kind, keep) in enumerate(
            [(600, 23, "done", "motion", False), (4000, 61, "done", "manual", True), (9000, 14, "done", "motion", False),
             (40000, 35, "failed", "motion", False), (90000, 48, "done", "motion", False), (95000, 12, "done", "motion", False)], 1)
    ],
    "photos": [{"id": i, "ts": now - d, "size": 90000} for i, d in enumerate([300, 7000, 80000], 1)],
    "settings": {
        "sensitivity": 5, "ignore_rects": [[.1, .1, .2, .2]], "retention_days": 14, "pre_roll_s": 3.0, "post_roll_s": 10.0,
        "max_event_s": 90.0, "max_disk_gb": 6.0, "presence_grace_s": 300.0, "push_cooldown_s": 60.0, "notify_motion": True,
        "notify_offline": True, "quiet_enabled": True, "quiet_start": "23:00", "quiet_end": "07:00", "camera_vflip": False,
        "camera_hmirror": False, "camera_brightness": 0, "camera_contrast": 0, "camera_quality": 0,
    },
}
NEXT = {"event": 100, "photo": 100}


def status():
    online = not STATE["privacy"] and time.time() >= STATE["wake_at"]
    clip = STATE["clip"]
    if clip["active"] and time.time() >= clip["until"]:
        clip.update(active=False, id=None)
    last = STATE["events"][0] if STATE["events"] else None
    return {
        "version": "mock", "armed": True, "reason": "away", "mode": STATE["mode"], "snooze_until": STATE["snooze_until"],
        "privacy": STATE["privacy"], "presence": {"configured": True, "home": False},
        "camera": {"online": online, "standby": STATE["privacy"], "rssi": -52, "fw": "0.2.0", "uptime_s": 190000},
        "motion": False, "recording": clip["active"], "last_event": {"id": last["id"], "start_ts": last["start"], "status": "done"} if last else None,
        "push_subscriptions": 0, "notifich": True,
        "storage": {"events": len(STATE["events"]), "bytes": 612000000, "cap_bytes": 6 * 1024**3, "photos": len(STATE["photos"]), "photo_bytes": 270000},
        "clip": clip,
    }


class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass

    def send(self, body, ctype="application/json", code=200):
        if isinstance(body, (dict, list)): body = json.dumps(body).encode()
        self.send_response(code); self.send_header("Content-Type", ctype); self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)

    def body(self):
        n = int(self.headers.get("Content-Length") or 0)
        return json.loads(self.rfile.read(n) or b"{}") if n else {}

    def do_GET(self):
        u = urlparse(self.path); p = u.path; q = parse_qs(u.query)
        with LOCK:
            if p == "/api/auth/state": return self.send({"logged_in": "tab=login" not in self.headers.get("Referer", ""), "enrolled": True})
            if p == "/api/status": return self.send(status())
            if p == "/api/events": return self.send({"events": STATE["events"]})
            if p == "/api/events/count": return self.send({"count": 2})
            if p == "/api/photos": return self.send({"photos": STATE["photos"]})
            if p == "/api/settings": return self.send(STATE["settings"])
            if p == "/api/auth/devices":
                return self.send({"credentials": [{"id": "a", "name": "iPhone · Safari"}], "sessions": [{"id": 1, "user_agent": "Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) Version/27 Mobile Safari/604.1"}], "current_session": 1})
            if p == "/api/push/key": return self.send({"key": "x"})
            if p == "/api/live" and STATE["privacy"]: return self.send({"detail": "camera in standby"}, code=503)
        if p in ("/api/snapshot", "/api/live") or p.endswith(("/thumb", "/img")): return self.send(ICON, "image/png")
        if p.endswith("/clip"): return self.send(b"", "video/mp4")
        if p == "/": return self.send((WEB / "index.html").read_bytes(), "text/html")
        f = WEB / p.lstrip("/")
        if f.is_file():
            ct = {".js": "text/javascript", ".css": "text/css", ".png": "image/png", ".svg": "image/svg+xml", ".webmanifest": "application/json"}.get(f.suffix, "application/octet-stream")
            return self.send(f.read_bytes(), ct)
        self.send({"detail": "nf"}, code=404)

    def do_POST(self):
        p = urlparse(self.path).path; b = self.body()
        with LOCK:
            if p == "/api/privacy":
                STATE["privacy"] = b["on"]
                if not b["on"]: STATE["wake_at"] = time.time() + 2.0
                return self.send({"ok": True})
            if p == "/api/mode": STATE["mode"] = b["mode"]; return self.send({"ok": True})
            if p == "/api/snooze": STATE["snooze_until"] = time.time() + b["minutes"] * 60 if b["minutes"] else 0; return self.send({"ok": True})
            if p == "/api/clips/start":
                STATE["clip"] = {"active": True, "id": 99, "until": time.time() + b["seconds"], "started": time.time()}; return self.send(STATE["clip"])
            if p == "/api/clips/stop":
                STATE["clip"].update(active=False, id=None); return self.send({"ok": True})
            if p == "/api/photos":
                NEXT["photo"] += 1
                STATE["photos"].insert(0, {"id": NEXT["photo"], "ts": time.time(), "size": 90000}); return self.send({"id": NEXT["photo"], "ts": time.time(), "size": 90000})
            if p == "/api/photos/delete":
                STATE["photos"] = [x for x in STATE["photos"] if x["id"] not in b["ids"]]; return self.send({"removed": len(b["ids"])})
            if p == "/api/events/delete":
                STATE["events"] = [x for x in STATE["events"] if x["id"] not in b["ids"]]; return self.send({"removed": len(b["ids"])})
            if p.startswith("/api/events/") and p.endswith("/keep"):
                eid = int(p.split("/")[3])
                for ev in STATE["events"]:
                    if ev["id"] == eid: ev["keep"] = b["keep"]
                return self.send({"ok": True, "keep": b["keep"]})
            if p == "/api/settings": STATE["settings"].update(b); return self.send({"ok": True})
            if p == "/api/push/test": return self.send({"sent": 1})
        self.send({"ok": True})

    def do_DELETE(self):
        p = urlparse(self.path).path
        with LOCK:
            if p.startswith("/api/events/"):
                eid = int(p.split("/")[3]); STATE["events"] = [x for x in STATE["events"] if x["id"] != eid]; return self.send({"ok": True})
            if p.startswith("/api/photos/"):
                pid = int(p.split("/")[3]); STATE["photos"] = [x for x in STATE["photos"] if x["id"] != pid]; return self.send({"ok": True})
        self.send({"ok": True})


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", 8199), H).serve_forever()
