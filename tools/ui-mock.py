"""Fake API for the PWA so the UI can be screenshotted without the camera: python tools/ui-mock.py, then open http://127.0.0.1:8199/#/live."""
import json, sys, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlparse, parse_qs

WEB = Path(__file__).resolve().parent.parent / "server" / "camd" / "web"
now = time.time()
EVENTS = [
    {"id": i, "start": now - d, "end": now - d + dur, "duration": dur, "status": st, "score": 5, "size": 1000000}
    for i, (d, dur, st) in enumerate([(600, 23, "done"), (4000, 61, "done"), (9000, 14, "done"), (40000, 35, "failed"), (90000, 48, "done"), (95000, 12, "done")], 1)
]
SETTINGS = {"sensitivity": 5, "ignore_rects": [[.1, .1, .2, .2]], "retention_days": 14, "pre_roll_s": 3.0, "post_roll_s": 10.0, "max_event_s": 90.0, "max_disk_gb": 6.0, "presence_grace_s": 300.0, "push_cooldown_s": 60.0, "notify_motion": True, "notify_offline": True, "quiet_enabled": True, "quiet_start": "23:00", "quiet_end": "07:00", "camera_vflip": False, "camera_hmirror": False, "camera_brightness": 0, "camera_contrast": 0, "camera_quality": 0}
STATUS = {"version": "1.1.0", 
    "armed": True, "reason": "away", "mode": "auto", "snooze_until": 0, "privacy": False,
    "presence": {"configured": True, "home": False},
    "camera": {"online": True, "standby": False, "rssi": -52, "fw": "0.2.0", "uptime_s": 190000},
    "motion": False, "recording": False,
    "last_event": {"id": 1, "start_ts": now - 600, "status": "done"},
    "push_subscriptions": 0, "notifich": True,
    "storage": {"events": 6, "bytes": 612000000, "cap_bytes": 6 * 1024**3},
}

class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def send(self, body, ctype="application/json", code=200):
        if isinstance(body, (dict, list)): body = json.dumps(body).encode()
        self.send_response(code); self.send_header("Content-Type", ctype); self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def do_GET(self):
        u = urlparse(self.path); p = u.path; q = parse_qs(u.query)
        if p == "/api/auth/state": return self.send({"logged_in": "tab=login" not in self.headers.get("Referer", ""), "enrolled": True})
        if p == "/api/status": return self.send(STATUS)
        if p == "/api/events": return self.send({"events": EVENTS})
        if p == "/api/settings": return self.send(SETTINGS)
        if p == "/api/events/count": return self.send({"count": 2})
        if p == "/api/auth/devices": return self.send({"credentials": [{"id": "a", "name": "iPhone · Safari"}], "sessions": [{"id": 1, "user_agent": "Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit Version/27 Mobile Safari/604.1"}, {"id": 2, "user_agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X) Chrome/120 Safari/537"}], "current_session": 1})
        if p == "/api/push/key": return self.send({"key": "x"})
        if p in ("/api/snapshot", "/api/live") or p.endswith("/thumb"): return self.send((WEB / "icon-512.png").read_bytes(), "image/png")
        if p == "/": return self.send((WEB / "index.html").read_bytes(), "text/html")
        f = WEB / p.lstrip("/")
        if f.is_file():
            ct = {".js": "text/javascript", ".css": "text/css", ".png": "image/png", ".webmanifest": "application/json", ".svg": "image/svg+xml"}.get(f.suffix, "application/octet-stream")
            return self.send(f.read_bytes(), ct)
        self.send({"detail": "nf"}, code=404)
ThreadingHTTPServer(("127.0.0.1", 8199), H).serve_forever()
