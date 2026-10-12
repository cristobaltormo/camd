import http.client
import logging
import threading
import time
from collections.abc import Callable, Iterator

log = logging.getLogger("camd.grabber")

FrameCallback = Callable[[float, bytes], None]

RETRY_STEPS = (1.0, 2.0, 3.0, 5.0, 10.0, 20.0, 30.0)


def retry_delay(failures: int) -> float:
    """Short retries right after a drop, then one attempt every 30 s while the camera stays away."""
    return RETRY_STEPS[min(max(failures, 1), len(RETRY_STEPS)) - 1]


class Grabber(threading.Thread):
    """Single consumer of the camera stream, fanning frames out to the pipeline and live viewers."""

    def __init__(self, host: str, token: str, on_frame: FrameCallback, on_connect: Callable[[], None] | None = None):
        super().__init__(daemon=True, name="grabber")
        self.host = host
        self.token = token
        self.on_frame = on_frame
        self.on_connect = on_connect or (lambda: None)
        self._halt = threading.Event()
        self._kick = threading.Event()
        self._cond = threading.Condition()
        self._latest: tuple[float, bytes] | None = None
        self._seq = 0
        self._standby = False
        self._enabled = True
        self.failures = 0
        self.viewers = 0

    @property
    def latest(self) -> tuple[float, bytes] | None:
        return self._latest

    @property
    def online(self) -> bool:
        latest = self._latest
        return self._enabled and latest is not None and time.time() - latest[0] < 10

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

    def stop(self) -> None:
        self._halt.set()
        self._kick.set()

    def _headers(self) -> dict[str, str]:
        return {"Authorization": f"Bearer {self.token}"}

    def set_standby(self, on: bool) -> bool:
        try:
            conn = http.client.HTTPConnection(self.host, 80, timeout=10)
            conn.request("GET", f"/control?var=standby&val={int(on)}", headers=self._headers())
            ok = conn.getresponse().status == 200
            conn.close()
        except OSError as exc:
            log.warning("standby request failed: %s", exc)
            return False
        if ok:
            self._standby = on
            if on:
                self._latest = None
            else:
                self._kick.set()
        return ok

    def apply_controls(self, controls: dict[str, int]) -> dict[str, bool]:
        results = {}
        for var, value in controls.items():
            try:
                conn = http.client.HTTPConnection(self.host, 80, timeout=10)
                conn.request("GET", f"/control?var={var}&val={int(value)}", headers=self._headers())
                results[var] = conn.getresponse().status == 200
                conn.close()
            except OSError as exc:
                log.warning("control %s failed: %s", var, exc)
                results[var] = False
        return results

    def fetch_status(self) -> dict | None:
        import json

        try:
            conn = http.client.HTTPConnection(self.host, 80, timeout=5)
            conn.request("GET", "/status", headers=self._headers())
            resp = conn.getresponse()
            body = resp.read()
            conn.close()
            return json.loads(body) if resp.status == 200 else None
        except (OSError, ValueError):
            return None

    def frames(self, max_wait: float = 5.0, first_wait: float = 15.0) -> Iterator[bytes]:
        """Yield each new frame as it arrives; ends when no frame shows up for max_wait seconds (first_wait for the first one)."""
        last = self._seq
        wait = first_wait
        while not self._halt.is_set():
            with self._cond:
                if not self._cond.wait_for(lambda: self._seq != last, timeout=wait):
                    return
                wait = max_wait
                last = self._seq
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

    def _consume(self) -> None:
        conn = http.client.HTTPConnection(self.host, 81, timeout=6)
        try:
            conn.request("GET", "/stream", headers=self._headers())
            resp = conn.getresponse()
            if resp.status != 200:
                raise ValueError(f"stream status {resp.status}")
            log.info("stream connected")
            threading.Thread(target=self.on_connect, daemon=True, name="on-connect").start()
            while not self._halt.is_set() and not self._standby and self._enabled:
                length = self._read_part_length(resp)
                data = resp.read(length)
                if len(data) != length:
                    raise ValueError("short frame")
                self._publish(time.time(), data)
        finally:
            conn.close()

    @staticmethod
    def _read_part_length(resp) -> int:
        while True:
            line = resp.readline()
            if not line:
                raise ValueError("stream closed")
            if line.strip() == b"--frame":
                break
        length = None
        while True:
            line = resp.readline()
            if not line:
                raise ValueError("stream closed")
            line = line.strip()
            if not line:
                break
            if line.lower().startswith(b"content-length:"):
                length = int(line.split(b":", 1)[1])
        if length is None or length <= 0 or length > 2_000_000:
            raise ValueError("bad frame length")
        return length

    def _publish(self, ts: float, data: bytes) -> None:
        if not self._enabled or self._standby:
            return
        with self._cond:
            self._latest = (ts, data)
            self._seq += 1
            self._cond.notify_all()
        self.on_frame(ts, data)
