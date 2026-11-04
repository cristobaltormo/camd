import json
import logging
import shutil
import subprocess
import threading
import time
from collections import deque
from collections.abc import Callable
from dataclasses import dataclass, field
from itertools import islice
from pathlib import Path

from .config import Config
from .db import Database
from .detect import Detection
from .spool import Spool
from .storage import unlink

log = logging.getLogger("camd.events")

START_STREAK = 3
MIN_FPS = 1.0
MAX_FPS = 30.0
STALL_S = 20.0
ENCODER_NICE = 10
DETECTION_RANK_BASE = 1.0
RECENT_DETECTION_S = 4.0


def event_json(row) -> dict:
    end = row["end_ts"]
    return {
        "id": row["id"],
        "start": row["start_ts"],
        "end": end,
        "duration": end - row["start_ts"] if end else None,
        "status": row["status"],
        "score": row["score"],
        "size": row["size_bytes"],
        "kind": row["kind"],
        "keep": bool(row["keep"]),
        "analyzed": bool(row["analyzed"]),
        "labels": json.loads(row["labels"]),
        "detections": json.loads(row["detections"]),
    }


@dataclass
class Recording:
    event_id: int
    spool: Spool
    start: float
    kind: str
    analyzed: bool
    last_activity: float = 0.0
    manual_until: float = 0.0
    peak_score: float = 0.0
    labels: dict[str, float] = field(default_factory=dict)
    best_rank: float = -1.0
    best_jpeg: bytes | None = None
    best_boxes: list[Detection] = field(default_factory=list)
    announced: bool = False

    @property
    def manual(self) -> bool:
        return self.kind == "manual"

    def consider(self, rank: float, jpeg: bytes, boxes: list[Detection] | None = None) -> None:
        if rank > self.best_rank:
            self.best_rank, self.best_jpeg, self.best_boxes = rank, jpeg, boxes or []


class EventRecorder:
    def __init__(
        self,
        cfg: Config,
        db: Database,
        analysis_ready: Callable[[], bool] = lambda: False,
        on_start: Callable[[int, float, bool], None] = lambda *_: None,
        on_confirm: Callable[[int, float, list[str]], None] = lambda *_: None,
        on_finish: Callable[[int], None] = lambda *_: None,
    ):
        self.cfg = cfg
        self.db = db
        self.analysis_ready = analysis_ready
        self.on_start = on_start
        self.on_confirm = on_confirm
        self.on_finish = on_finish
        self._preroll: deque[tuple[float, bytes]] = deque()
        self._current: Recording | None = None
        self._streak = 0
        self._last_frame = 0.0
        self._recent: tuple[float, bytes, list[Detection]] | None = None
        self._lock = threading.RLock()
        self._encoder_slot = threading.Semaphore(1)
        self._workers: list[threading.Thread] = []

    @property
    def recording(self) -> bool:
        return self._current is not None

    def spool_path(self, event_id: int) -> Path:
        return self.cfg.spool_dir / f"{event_id}.spool"

    def feed(self, ts: float, jpeg: bytes, motion: bool, score: float, armed: bool) -> None:
        with self._lock:
            self._last_frame = ts
            rec = self._current
            if rec is None:
                self._idle_frame(ts, jpeg, motion, armed)
            else:
                self._recording_frame(rec, ts, jpeg, motion, score, armed)

    def _idle_frame(self, ts: float, jpeg: bytes, motion: bool, armed: bool) -> None:
        self._preroll.append((ts, jpeg))
        while self._preroll and ts - self._preroll[0][0] > self.cfg.pre_roll_s:
            self._preroll.popleft()
        self._streak = self._streak + 1 if armed and motion else 0
        if self._streak >= START_STREAK:
            self._begin("motion", ts)

    def _recording_frame(self, rec: Recording, ts: float, jpeg: bytes, motion: bool, score: float, armed: bool) -> None:
        rec.spool.append(ts, jpeg)
        rec.peak_score = max(rec.peak_score, score)
        rec.consider(score, jpeg)
        if motion:
            rec.last_activity = ts
        if rec.manual:
            finished = ts >= rec.manual_until
        else:
            finished = not armed or ts - rec.last_activity > self.cfg.post_roll_s or ts - rec.start > self.cfg.max_event_s
        if finished:
            self._end()

    def observe(self, ts: float, jpeg: bytes, detections: list[Detection]) -> None:
        with self._lock:
            rec = self._current
            if not detections:
                return
            if rec is None:
                self._recent = (ts, jpeg, detections)
            elif not rec.manual:
                self._apply(rec, ts, jpeg, detections)

    def _apply_recent(self, rec: Recording, now: float) -> None:
        recent, self._recent = self._recent, None
        if recent is not None and now - recent[0] <= RECENT_DETECTION_S:
            self._apply(rec, *recent)

    def _apply(self, rec: Recording, ts: float, jpeg: bytes, detections: list[Detection]) -> None:
        rec.last_activity = max(rec.last_activity, ts)
        fresh = any(d.label not in rec.labels for d in detections)
        for d in detections:
            rec.labels[d.label] = max(rec.labels.get(d.label, 0.0), d.confidence)
        top = max(detections, key=lambda d: d.confidence)
        rec.consider(DETECTION_RANK_BASE + top.confidence, jpeg, detections)
        if fresh:
            self.db.execute("UPDATE events SET labels=?, confirmed=1 WHERE id=?", (json.dumps(self._ranked_labels(rec)), rec.event_id))
        if not rec.announced:
            rec.announced = True
            self.on_confirm(rec.event_id, rec.start, self._ranked_labels(rec))

    @staticmethod
    def _ranked_labels(rec: Recording) -> list[str]:
        return sorted(rec.labels, key=lambda name: -rec.labels[name])

    def _begin(self, kind: str, now: float) -> Recording:
        preroll = list(self._preroll)
        self._preroll.clear()
        start = preroll[0][0] if preroll else now
        analyzed = kind == "motion" and self.analysis_ready()
        event_id = self.db.execute(
            "INSERT INTO events(start_ts, status, kind, keep, analyzed) VALUES(?, 'recording', ?, ?, ?)",
            (start, kind, int(kind == "manual"), int(analyzed)),
        ).lastrowid
        spool = Spool.create(self.spool_path(event_id))
        for ts, jpeg in preroll:
            spool.append(ts, jpeg)
        rec = Recording(event_id, spool, start, kind, analyzed, last_activity=now)
        self._current = rec
        self._streak = 0
        log.info("event %s started (%s)", event_id, kind)
        if kind == "motion":
            self.on_start(event_id, start, analyzed)
            self._apply_recent(rec, now)
        return rec

    def _end(self) -> None:
        rec, self._current = self._current, None
        self._streak = 0
        rec.spool.close()
        self._spawn(rec)

    def _spawn(self, rec: Recording) -> None:
        worker = threading.Thread(target=self._finalize, args=(rec,), daemon=True, name=f"encode-{rec.event_id}")
        self._workers = [w for w in self._workers if w.is_alive()] + [worker]
        worker.start()

    def flush(self, timeout: float = 30.0) -> None:
        for worker in list(self._workers):
            worker.join(timeout)

    def close_open_event(self) -> None:
        with self._lock:
            if self._current is not None:
                self._end()

    def start_manual(self, seconds: float, now: float | None = None) -> int:
        now = now if now is not None else time.time()
        with self._lock:
            rec = self._current or self._begin("manual", now)
            rec.manual_until = now + seconds
            self._last_frame = max(self._last_frame, now)
            if not rec.manual:
                rec.kind = "manual"
                self.db.execute("UPDATE events SET kind='manual', keep=1 WHERE id=?", (rec.event_id,))
            return rec.event_id

    def stop_manual(self) -> int | None:
        with self._lock:
            rec = self._current
            if rec is None or not rec.manual:
                return None
            self._end()
            return rec.event_id

    def manual_state(self) -> dict:
        with self._lock:
            rec = self._current
            if rec is None or not rec.manual:
                return {"active": False, "id": None, "until": 0, "started": 0}
            return {"active": True, "id": rec.event_id, "until": rec.manual_until, "started": rec.start}

    def tick(self, now: float | None = None) -> None:
        now = now if now is not None else time.time()
        with self._lock:
            rec = self._current
            if rec is not None and ((rec.manual and now >= rec.manual_until) or now - self._last_frame > STALL_S):
                self._end()

    def recover(self) -> None:
        interrupted = {row["id"]: row for row in self.db.query("SELECT id, start_ts, kind FROM events WHERE status='recording'")}
        for event_id, row in interrupted.items():
            path = self.spool_path(event_id)
            spool = Spool.load(path) if path.is_file() else None
            if spool is None or spool.count < 2:
                self.db.execute("DELETE FROM events WHERE id=?", (event_id,))
                unlink(str(path))
                continue
            self._spawn(Recording(event_id, spool, row["start_ts"], row["kind"], analyzed=False))
        if self.cfg.spool_dir.is_dir():
            for leftover in self.cfg.spool_dir.glob("*.spool"):
                if int(leftover.stem) not in interrupted:
                    leftover.unlink(missing_ok=True)

    def _finalize(self, rec: Recording) -> None:
        try:
            if rec.spool.count < 2 or (rec.analyzed and not rec.labels and rec.kind == "motion" and self.cfg.detection == "strict"):
                self.db.execute("DELETE FROM events WHERE id=?", (rec.event_id,))
                return
            folder = self.cfg.clips_dir / time.strftime("%Y/%m/%d", time.localtime(rec.start))
            folder.mkdir(parents=True, exist_ok=True)
            clip, thumb = folder / f"{rec.event_id}.mp4", folder / f"{rec.event_id}.jpg"
            thumb.write_bytes(rec.best_jpeg or self._middle_frame(rec.spool))
            fps = min(max((rec.spool.count - 1) / rec.spool.duration, MIN_FPS), MAX_FPS)
            with self._encoder_slot:
                self._encode(rec.spool, fps, clip)
            size = clip.stat().st_size + thumb.stat().st_size
            labels = self._ranked_labels(rec)
            self.db.execute(
                "UPDATE events SET end_ts=?, status='done', score=?, clip_path=?, thumb_path=?, size_bytes=?, labels=?, detections=?, confirmed=? WHERE id=?",
                (
                    rec.spool.last_ts,
                    rec.peak_score,
                    str(clip),
                    str(thumb),
                    size,
                    json.dumps(labels),
                    json.dumps([d.as_dict() for d in rec.best_boxes]),
                    int(bool(labels)),
                    rec.event_id,
                ),
            )
            log.info(
                "event %s done: %.1fs, %d frames, %.0f KB, labels=%s",
                rec.event_id,
                rec.spool.duration,
                rec.spool.count,
                size / 1024,
                labels,
            )
        except Exception:
            log.exception("event %s failed", rec.event_id)
            self.db.execute("UPDATE events SET status='failed', end_ts=? WHERE id=?", (time.time(), rec.event_id))
        finally:
            rec.spool.delete()
            self.on_finish(rec.event_id)

    @staticmethod
    def _middle_frame(spool: Spool) -> bytes:
        return next(islice((jpeg for _, jpeg in spool.frames()), spool.count // 2, None))

    @staticmethod
    def _encode(spool: Spool, fps: float, out: Path) -> None:
        ffmpeg = shutil.which("ffmpeg")
        if ffmpeg is None:
            raise RuntimeError("ffmpeg is not installed")
        command = [
            ffmpeg,
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "image2pipe",
            "-framerate",
            f"{fps:.3f}",
            "-c:v",
            "mjpeg",
            "-i",
            "-",
            "-threads",
            "1",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "28",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            str(out),
        ]
        if nice := shutil.which("nice"):
            command = [nice, "-n", str(ENCODER_NICE), *command]
        proc = subprocess.Popen(command, stdin=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            for _, jpeg in spool.frames():
                proc.stdin.write(jpeg)
            proc.stdin.close()
        except BrokenPipeError:
            pass
        error = proc.stderr.read().decode(errors="replace")
        if proc.wait() != 0:
            raise RuntimeError(f"ffmpeg failed: {error.strip()}")
