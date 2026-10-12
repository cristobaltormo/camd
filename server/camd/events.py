import logging
import shutil
import subprocess
import threading
import time
from collections import deque
from collections.abc import Callable
from pathlib import Path

from .config import Config
from .db import Database

log = logging.getLogger("camd.events")

START_STREAK = 3
MIN_FPS = 1.0
MAX_FPS = 30.0


class EventRecorder:
    """Turns a motion/armed signal per frame into stored events with a clip and a thumbnail."""

    def __init__(
        self,
        cfg: Config,
        db: Database,
        on_start: Callable[[int, float], None] | None = None,
        on_finish: Callable[[int], None] | None = None,
    ):
        self.cfg = cfg
        self.db = db
        self.on_start = on_start or (lambda *_: None)
        self.on_finish = on_finish or (lambda *_: None)
        self._preroll: deque[tuple[float, bytes, float]] = deque()
        self._frames: list[tuple[float, bytes, float]] = []
        self._streak = 0
        self._event_id: int | None = None
        self._last_motion = 0.0
        self._start = 0.0
        self._workers: list[threading.Thread] = []

    @property
    def recording(self) -> bool:
        return self._event_id is not None

    def feed(self, ts: float, jpeg: bytes, motion: bool, score: float, armed: bool) -> None:
        item = (ts, jpeg, score)

        if self._event_id is None:
            self._preroll.append(item)
            while self._preroll and ts - self._preroll[0][0] > self.cfg.pre_roll_s:
                self._preroll.popleft()
            self._streak = self._streak + 1 if (armed and motion) else 0
            if self._streak >= START_STREAK:
                self._begin(ts)
            return

        self._frames.append(item)
        if motion:
            self._last_motion = ts
        if not armed:
            self._end(ts)
        elif ts - self._last_motion > self.cfg.post_roll_s:
            self._end(ts)
        elif ts - self._start > self.cfg.max_event_s:
            self._end(ts)

    def _begin(self, ts: float) -> None:
        self._frames = list(self._preroll)
        self._preroll.clear()
        self._start = self._frames[0][0] if self._frames else ts
        self._last_motion = ts
        self._streak = 0
        cur = self.db.execute(
            "INSERT INTO events(start_ts, status) VALUES(?, 'recording')", (self._start,)
        )
        self._event_id = cur.lastrowid
        log.info("event %s started", self._event_id)
        self.on_start(self._event_id, self._start)

    def _end(self, ts: float) -> None:
        event_id, frames, start = self._event_id, self._frames, self._start
        self._event_id = None
        self._frames = []
        self._streak = 0
        worker = threading.Thread(
            target=self._finalize, args=(event_id, frames, start), daemon=True, name=f"encode-{event_id}"
        )
        self._workers = [w for w in self._workers if w.is_alive()] + [worker]
        worker.start()

    def flush(self, timeout: float = 30.0) -> None:
        for worker in list(self._workers):
            worker.join(timeout)

    def close_open_event(self) -> None:
        if self._event_id is not None:
            self._end(time.time())

    def _finalize(self, event_id: int, frames: list[tuple[float, bytes, float]], start: float) -> None:
        try:
            if len(frames) < 2:
                self.db.execute("DELETE FROM events WHERE id=?", (event_id,))
                return
            end_ts = frames[-1][0]
            duration = max(end_ts - frames[0][0], 0.1)
            fps = min(max((len(frames) - 1) / duration, MIN_FPS), MAX_FPS)
            day = time.strftime("%Y/%m/%d", time.localtime(start))
            folder = self.cfg.clips_dir / day
            folder.mkdir(parents=True, exist_ok=True)
            clip = folder / f"{event_id}.mp4"
            thumb = folder / f"{event_id}.jpg"

            best = max(frames, key=lambda f: f[2])
            thumb.write_bytes(best[1])
            self._encode(frames, fps, clip)
            size = clip.stat().st_size + thumb.stat().st_size
            self.db.execute(
                "UPDATE events SET end_ts=?, status='done', score=?, clip_path=?, thumb_path=?, size_bytes=? WHERE id=?",
                (end_ts, best[2], str(clip), str(thumb), size, event_id),
            )
            log.info("event %s done: %.1fs, %d frames, %.0f KB", event_id, duration, len(frames), size / 1024)
        except Exception:
            log.exception("event %s failed", event_id)
            self.db.execute("UPDATE events SET status='failed', end_ts=? WHERE id=?", (time.time(), event_id))
        finally:
            self.on_finish(event_id)

    @staticmethod
    def _encode(frames: list[tuple[float, bytes, float]], fps: float, out: Path) -> None:
        ffmpeg = shutil.which("ffmpeg")
        if ffmpeg is None:
            raise RuntimeError("ffmpeg not installed")
        proc = subprocess.Popen(
            [
                ffmpeg, "-hide_banner", "-loglevel", "error", "-y",
                "-f", "image2pipe", "-framerate", f"{fps:.3f}", "-c:v", "mjpeg", "-i", "-",
                "-c:v", "libx264", "-preset", "veryfast", "-crf", "28", "-pix_fmt", "yuv420p",
                "-movflags", "+faststart", str(out),
            ],
            stdin=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        try:
            for _, jpeg, _ in frames:
                proc.stdin.write(jpeg)
            proc.stdin.close()
        except BrokenPipeError:
            pass
        err = proc.stderr.read().decode(errors="replace")
        if proc.wait() != 0:
            raise RuntimeError(f"ffmpeg failed: {err.strip()}")


def purge(cfg: Config, db: Database) -> int:
    """Delete events older than the retention window, then oldest ones while over the disk cap."""
    removed = 0
    cutoff = time.time() - cfg.retention_days * 86400
    rows = db.query("SELECT id, clip_path, thumb_path FROM events WHERE start_ts < ? AND status != 'recording'", (cutoff,))
    for row in rows:
        remove_event(db, row)
        removed += 1

    cap = int(cfg.max_disk_gb * 1024**3)
    while True:
        total = db.one("SELECT COALESCE(SUM(size_bytes), 0) AS s FROM events")["s"]
        if total <= cap:
            break
        oldest = db.one("SELECT id, clip_path, thumb_path FROM events WHERE status != 'recording' ORDER BY start_ts LIMIT 1")
        if oldest is None:
            break
        remove_event(db, oldest)
        removed += 1
    return removed


def remove_event(db: Database, row) -> None:
    for key in ("clip_path", "thumb_path"):
        if row[key]:
            Path(row[key]).unlink(missing_ok=True)
    db.execute("DELETE FROM events WHERE id=?", (row["id"],))
