import argparse
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

STATE = {"standby": False, "vflip": 0, "hmirror": 0, "quality": 12, "brightness": 0, "contrast": 0}
BOUNDARY = b"\r\n--frame\r\n"


def load_frames(folder: Path) -> list[bytes]:
    frames = [path.read_bytes() for path in sorted(folder.glob("*.jpg"))]
    if not frames:
        raise SystemExit(f"no .jpg files in {folder}")
    return frames


def make_handler(frames: list[bytes], token: str, fps: float, started: float):
    clock = {"index": 0}

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_):
            return

        def reply(self, code: int, body: bytes, kind: str = "text/plain") -> None:
            self.send_response(code)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def authorised(self, query: dict) -> bool:
            header = self.headers.get("Authorization", "")
            return header == f"Bearer {token}" or query.get("token", [""])[0] == token

        def next_frame(self) -> bytes:
            frame = frames[clock["index"] % len(frames)]
            clock["index"] += 1
            return frame

        def do_GET(self):
            url = urlparse(self.path)
            query = parse_qs(url.query)
            if url.path == "/health":
                return self.reply(200, b"ok")
            if not self.authorised(query):
                return self.reply(401, b"unauthorized")
            if url.path == "/status":
                body = {"fw": "fake", "uptime_s": int(time.time() - started), "rssi": -50, "standby": int(STATE["standby"]), **STATE}
                return self.reply(200, json.dumps(body).encode(), "application/json")
            if url.path == "/control":
                name, value = query.get("var", [""])[0], int(query.get("val", ["0"])[0])
                if name == "standby":
                    STATE["standby"] = bool(value)
                elif name in STATE:
                    STATE[name] = value
                else:
                    return self.reply(400, b"unknown setting")
                return self.reply(200, b"ok")
            if url.path == "/capture":
                return self.reply(503, b"standby") if STATE["standby"] else self.reply(200, self.next_frame(), "image/jpeg")
            if url.path == "/stream":
                return self.stream()
            return self.reply(404, b"not found")

        def stream(self) -> None:
            if STATE["standby"]:
                return self.reply(503, b"standby")
            self.send_response(200)
            self.send_header("Content-Type", "multipart/x-mixed-replace;boundary=frame")
            self.send_header("Transfer-Encoding", "chunked")
            self.end_headers()
            try:
                while not STATE["standby"]:
                    frame = self.next_frame()
                    part = BOUNDARY + b"Content-Type: image/jpeg\r\nContent-Length: %d\r\n\r\n" % len(frame) + frame
                    self.wfile.write(b"%x\r\n%s\r\n" % (len(part), part))
                    self.wfile.flush()
                    time.sleep(1 / fps)
            except OSError:
                return

    return Handler


def serve(port: int, handler) -> None:
    ThreadingHTTPServer(("0.0.0.0", port), handler).serve_forever()


def main() -> None:
    parser = argparse.ArgumentParser(description="Pretend to be an ESP32-CAM by looping the JPEG files of a folder")
    parser.add_argument("frames", type=Path, help="folder with .jpg files, played in name order")
    parser.add_argument("--token", default="test")
    parser.add_argument("--fps", type=float, default=10.0)
    parser.add_argument("--api-port", type=int, default=8081)
    parser.add_argument("--stream-port", type=int, default=8082)
    args = parser.parse_args()

    handler = make_handler(load_frames(args.frames), args.token, args.fps, time.time())
    threading.Thread(target=serve, args=(args.api_port, handler), daemon=True).start()
    print(f"fake camera: api :{args.api_port}, stream :{args.stream_port}, token '{args.token}'", flush=True)
    serve(args.stream_port, handler)


if __name__ == "__main__":
    main()
