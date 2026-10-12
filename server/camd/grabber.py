import http.client
import logging
import threading
import time
from collections.abc import Callable, Iterator

log = logging.getLogger("camd.grabber")

FrameCallback = Callable[[float, bytes], None]


class Grabber(threading.Thread):
    """Single consumer of the camera stream, fanning frames out to the pipeline and live viewers."""

    def __init__(self, host: str, token: str, on_frame: FrameCallback, on_connect: Callable[[], None] | None = None):
        super().__init__(daemon=True, name="grabber")
        self.host = host
        self.token = token
        self.on_frame = on_frame
        self.on_connect = on_connect or (lambda: None)
        self._halt = threading.Event()
        self._cond = threading.Condition()
        self._latest: tuple[float, bytes] | None = None
        self._seq = 0
        self._standby = False
        self.viewers = 0

    @property
    def latest(self) -> tuple[float, bytes] | None:
        return self._latest

    @property
    def online(self) -> bool:
        latest = self._latest
        return latest is not None and time.time() - latest[0] < 10

    @property
    def standby(self) -> bool:
        return self._standby

    def stop(self) -> None:
        self._halt.set()

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

    def frames(self, max_wait: float = 5.0) -> Iterator[bytes]:
        """Yield each new frame as it arrives; ends when no frame shows up for max_wait seconds."""
        last = self._seq
        while not self._halt.is_set():
            with self._cond:
                if not self._cond.wait_for(lambda: self._seq != last, timeout=max_wait):
                    return
                last = self._seq
                frame = self._latest
            if frame:
                yield frame[1]

    def run(self) -> None:
        backoff = 1.0
        while not self._halt.is_set():
            if self._standby:
                time.sleep(1)
                continue
            try:
                self._consume()
                backoff = 1.0
            except (OSError, http.client.HTTPException, ValueError) as exc:
                log.warning("stream error: %s", exc)
            if self._halt.wait(backoff):
                return
            backoff = min(backoff * 2, 5.0)

    def _consume(self) -> None:
        conn = http.client.HTTPConnection(self.host, 81, timeout=10)
        try:
            conn.request("GET", "/stream", headers=self._headers())
            resp = conn.getresponse()
            if resp.status != 200:
                raise ValueError(f"stream status {resp.status}")
            log.info("stream connected")
            threading.Thread(target=self.on_connect, daemon=True, name="on-connect").start()
            while not self._halt.is_set() and not self._standby:
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
        with self._cond:
            self._latest = (ts, data)
            self._seq += 1
            self._cond.notify_all()
        self.on_frame(ts, data)
