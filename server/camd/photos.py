import logging
import time
from pathlib import Path

import cv2
import numpy as np

from .config import Config
from .db import Database
from .storage import unlink

log = logging.getLogger("camd.photos")

MAX_PHOTOS = 1000
THUMB_WIDTH = 360
THUMB_QUALITY = 80


class PhotoLimit(Exception):
    pass


class Photos:
    def __init__(self, cfg: Config, db: Database):
        self.cfg = cfg
        self.db = db

    def count(self) -> int:
        return self.db.row("SELECT COUNT(*) AS n FROM photos")["n"]

    def usage(self) -> tuple[int, int]:
        row = self.db.row("SELECT COUNT(*) AS n, COALESCE(SUM(size_bytes), 0) AS bytes FROM photos")
        return row["n"], row["bytes"]

    def save(self, jpeg: bytes, ts: float | None = None) -> dict:
        if self.count() >= MAX_PHOTOS:
            raise PhotoLimit(f"photo limit of {MAX_PHOTOS} reached")
        ts = ts if ts is not None else time.time()
        thumbnail = self._thumbnail(jpeg)
        photo_id = self.db.execute("INSERT INTO photos(ts, path, thumb_path) VALUES(?, '', '')", (ts,)).lastrowid
        try:
            folder = self.cfg.photos_dir / time.strftime("%Y/%m", time.localtime(ts))
            folder.mkdir(parents=True, exist_ok=True)
            path, thumb = folder / f"{photo_id}.jpg", folder / f"{photo_id}_t.jpg"
            path.write_bytes(jpeg)
            thumb.write_bytes(thumbnail)
            size = path.stat().st_size + thumb.stat().st_size
            self.db.execute("UPDATE photos SET path=?, thumb_path=?, size_bytes=? WHERE id=?", (str(path), str(thumb), size, photo_id))
        except Exception:
            self.db.execute("DELETE FROM photos WHERE id=?", (photo_id,))
            raise
        return {"id": photo_id, "ts": ts, "size": size}

    @staticmethod
    def _thumbnail(jpeg: bytes) -> bytes:
        image = cv2.imdecode(np.frombuffer(jpeg, np.uint8), cv2.IMREAD_COLOR)
        if image is None:
            raise ValueError("not a decodable jpeg")
        height = max(1, int(image.shape[0] * THUMB_WIDTH / image.shape[1]))
        small = cv2.resize(image, (THUMB_WIDTH, height), interpolation=cv2.INTER_AREA)
        ok, buffer = cv2.imencode(".jpg", small, [cv2.IMWRITE_JPEG_QUALITY, THUMB_QUALITY])
        if not ok:
            raise ValueError("thumbnail encode failed")
        return buffer.tobytes()

    def list(self, limit: int, before: float | None) -> list[dict]:
        if before is None:
            rows = self.db.query("SELECT * FROM photos ORDER BY ts DESC LIMIT ?", (limit,))
        else:
            rows = self.db.query("SELECT * FROM photos WHERE ts < ? ORDER BY ts DESC LIMIT ?", (before, limit))
        return [{"id": r["id"], "ts": r["ts"], "size": r["size_bytes"]} for r in rows]

    def get(self, photo_id: int):
        return self.db.one("SELECT * FROM photos WHERE id=?", (photo_id,))

    def file(self, photo_id: int, thumb: bool) -> Path | None:
        row = self.get(photo_id)
        if row is None:
            return None
        path = Path(row["thumb_path" if thumb else "path"])
        if not path.is_file() or self.cfg.photos_dir.resolve() not in path.resolve().parents:
            return None
        return path

    def remove(self, photo_id: int) -> bool:
        row = self.get(photo_id)
        if row is None:
            return False
        unlink(row["path"])
        unlink(row["thumb_path"])
        self.db.execute("DELETE FROM photos WHERE id=?", (photo_id,))
        return True
