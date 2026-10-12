import http.client
import json
import logging
import threading
import time
from collections.abc import Callable, Iterator

log = logging.getLogger("camd.grabber")

FrameCallback = Callable[[float, bytes], None]

RETRY_STEPS = (1.0, 2.0, 3.0, 5.0, 10.0, 20.0, 30.0)
ONLINE_WINDOW_S = 10.0
MAX_FRAME_BYTES = 2_000_000
FPS_SMOOTHING = 0.1


def retry_delay(failures: int) -> float:
    return RETRY_STEPS[min(max(failures, 1), len(RETRY_STEPS)) - 1]


class CameraClient:
    def __init__(self, host: str, token: str, api_port: int = 80, stream_port: int = 81):
        self.host = host
        self.api_port = api_port
        self.stream_port = stream_port
        self.headers = {"Authorization": f"Bearer {token}"}

    def get(self, path: str, timeout: float = 10.0) -> tuple[int, bytes]:
        conn = http.client.HTTPConnection(self.host, self.api_port, timeout=timeout)
        try:
            conn.request("GET", path, headers=self.headers)
            response = conn.getresponse()
            return response.status, response.read()
        finally:
            conn.close()

    def set_standby(self, on: bool) -> bool:
        try:
            return self.get(f"/control?var=standby&val={int(on)}")[0] == 200
        except OSError as exc:
            log.warning("standby request failed: %s", exc)
            return False

    def apply_controls(self, controls: dict[str, int]) -> dict[str, bool]:
        results = {}
        for name, value in controls.items():
            try:
                results[name] = self.get(f"/control?var={name}&val={int(value)}")[0] == 200
            except OSError as exc:
                log.warning("control %s failed: %s", name, exc)
                results[name] = False
        return results

    def status(self) -> dict | None:
        try:
            code, body = self.get("/status", timeout=5.0)
            return json.loads(body) if code == 200 else None
        except (OSError, ValueError):
            return None

    def open_stream(self) -> http.client.HTTPConnection:
        return http.client.HTTPConnection(self.host, self.stream_port, timeout=6.0)


class Grabber(threading.Thread):
    def __init__(self, camera: CameraClient, on_frame: FrameCallback, on_connect: Callable[[], None] = lambda: None):
        super().__init__(daemon=True, name="grabber")
        self.camera = camera
        self.on_frame = on_frame
        self.on_connect = on_connect
        self.failures = 0
        self.viewers = 0
        self.fps = 0.0
        self._halt = threading.Event()
        self._kick = threading.Event()
        self._frame_ready = threading.Condition()
        self._latest: tuple[float, bytes] | None = None
        self._sequence = 0
        self._standby = False
        self._enabled = True

    @property
    def latest(self) -> tuple[float, bytes] | None:
        return self._latest

    @property
    def online(self) -> bool:
        latest = self._latest
        return self._enabled and latest is not None and time.time() - latest[0] < ONLINE_WINDOW_S

    @property
    def standby(self) -> bool:
        return self._standby

    @property
    def enabled(self) -> bool:
        return self._enabled

    def set_enabled(self, on: bool) -> None:
        self._enabled = on
        if on:
            self.failures = 0
            self._kick.set()
        else:
            self._latest = None
            self.fps = 0.0

    def set_standby(self, on: bool) -> bool:
        if not self.camera.set_standby(on):
            return False
        self._standby = on
        if on:
            self._latest = None
            self.fps = 0.0
        else:
            self._kick.set()
        return True

    def stop(self) -> None:
        self._halt.set()
        self._kick.set()

    def frames(self, max_wait: float = 5.0, first_wait: float = 15.0) -> Iterator[bytes]:
        last = self._sequence
        wait = first_wait
        while not self._halt.is_set():
            with self._frame_ready:
                if not self._frame_ready.wait_for(lambda seen=last: self._sequence != seen, timeout=wait):
                    return
                wait = max_wait
                last = self._sequence
                frame = self._latest
            if frame:
                yield frame[1]

    def run(self) -> None:
        while not self._halt.is_set():
            if self._standby or not self._enabled:
                self._kick.wait(1)
                self._kick.clear()
                continue
            try:
                self._consume()
                self.failures = 0
            except (OSError, http.client.HTTPException, ValueError) as exc:
                self.failures += 1
                if self.failures in (1, 5) or self.failures % 20 == 0:
                    log.warning("camera unreachable (%d failures): %s", self.failures, exc)
            if self._kick.wait(retry_delay(self.failures)):
                self._kick.clear()
                self.failures = 0

    def _streaming(self) -> bool:
        return not self._halt.is_set() and not self._standby and self._enabled

    def _consume(self) -> None:
        conn = self.camera.open_stream()
        try:
            conn.request("GET", "/stream", headers=self.camera.headers)
            response = conn.getresponse()
            if response.status != 200:
                raise ValueError(f"stream status {response.status}")
            log.info("stream connected")
            threading.Thread(target=self.on_connect, daemon=True, name="on-connect").start()
            while self._streaming():
                length = self._next_frame_length(response)
                data = response.read(length)
                if len(data) != length:
                    raise ValueError("short frame")
                self._publish(time.time(), data)
        finally:
            conn.close()

    @staticmethod
    def _next_frame_length(response) -> int:
        while True:
            line = response.readline()
            if not line:
                raise ValueError("stream closed")
            if line.strip() == b"--frame":
                break
        length = None
        while True:
            line = response.readline()
            if not line:
                raise ValueError("stream closed")
            line = line.strip()
            if not line:
                break
            if line.lower().startswith(b"content-length:"):
                length = int(line.split(b":", 1)[1])
        if length is None or not 0 < length <= MAX_FRAME_BYTES:
            raise ValueError("bad frame length")
        return length

    def _publish(self, ts: float, data: bytes) -> None:
        if not (self._enabled and not self._standby):
            return
        previous = self._latest
        if previous is not None and ts > previous[0]:
            self.fps += FPS_SMOOTHING * (1.0 / (ts - previous[0]) - self.fps)
        with self._frame_ready:
            self._latest = (ts, data)
            self._sequence += 1
            self._frame_ready.notify_all()
        self.on_frame(ts, data)
