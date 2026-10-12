#!/usr/bin/env python3
"""Temporary LAN-only live view to aim and focus the camera by hand.

Reads CAM_IP, CAM_TOKEN and PAGE_SECRET from the environment, serves the stream
under a secret path and shuts itself down after TTL seconds.
"""
import os
import threading
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

CAM = os.environ["CAM_IP"]
TOKEN = os.environ["CAM_TOKEN"]
SECRET = os.environ["PAGE_SECRET"]
TTL = int(os.environ.get("TTL", "1200"))
PORT = int(os.environ.get("PORT", "8099"))

PAGE = """<!doctype html><meta name=viewport content="width=device-width,initial-scale=1">
<title>Ajuste de camara</title>
<style>body{margin:0;background:#111;color:#ddd;font:16px system-ui}
img{width:100%;display:block}nav{display:flex;gap:8px;padding:10px;flex-wrap:wrap}
button{padding:10px 14px;font-size:16px;border:0;border-radius:8px;background:#2f6fed;color:#fff}</style>
<img src="stream">
<nav>
<button onclick="c('vflip',1)">Voltear vertical</button>
<button onclick="c('vflip',0)">Normal vertical</button>
<button onclick="c('hmirror',1)">Espejo</button>
<button onclick="c('hmirror',0)">Sin espejo</button>
</nav>
<script>function c(v,x){fetch('ctl?var='+v+'&val='+x)}</script>"""


def upstream(path):
    req = urllib.request.Request(f"http://{CAM}{path}", headers={"Authorization": f"Bearer {TOKEN}"})
    return urllib.request.urlopen(req, timeout=10)


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        prefix = f"/{SECRET}/"
        if not self.path.startswith(prefix):
            self.send_error(404)
            return
        rest = self.path[len(prefix):]
        if rest == "":
            body = PAGE.encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif rest == "stream":
            try:
                with urllib.request.urlopen(
                    urllib.request.Request(f"http://{CAM}:81/stream", headers={"Authorization": f"Bearer {TOKEN}"}),
                    timeout=10,
                ) as r:
                    self.send_response(200)
                    self.send_header("Content-Type", r.headers["Content-Type"])
                    self.end_headers()
                    while chunk := r.read(4096):
                        self.wfile.write(chunk)
            except (OSError, ValueError):
                pass
        elif rest.startswith("ctl?"):
            try:
                upstream("/control?" + rest[4:]).read()
                self.send_response(204)
            except OSError:
                self.send_response(502)
            self.end_headers()
        else:
            self.send_error(404)


server = ThreadingHTTPServer(("0.0.0.0", PORT), Handler)
threading.Timer(TTL, server.shutdown).start()
server.serve_forever()
