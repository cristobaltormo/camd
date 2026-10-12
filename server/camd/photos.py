import logging
import time
from pathlib import Path

import cv2
import numpy as np

from .config import Config
from .db import Database

log = logging.getLogger("camd.photos")

MAX_PHOTOS = 1000
THUMB_WIDTH = 360


class PhotoLimit(Exception):
    pass


class Photos:
    def __init__(self, cfg: Config, db: Database):
        self.cfg = cfg
        self.db = db

    @property
    def folder(self) -> Path:
        return self.cfg.data_dir / "photos"

    def count(self) -> int:
        return self.db.one("SELECT COUNT(*) AS n FROM photos")["n"]

    def save(self, jpeg: bytes, ts: float | None = None) -> dict:
        if self.count() >= MAX_PHOTOS:
            raise PhotoLimit(f"photo limit of {MAX_PHOTOS} reached")
        ts = ts if ts is not None else time.time()
        cur = self.db.execute("INSERT INTO photos(ts, path, thumb_path) VALUES(?, '', '')", (ts,))
        photo_id = cur.lastrowid
        try:
            folder = self.folder / time.strftime("%Y/%m", time.localtime(ts))
            folder.mkdir(parents=True, exist_ok=True)
            path, thumb = folder / f"{photo_id}.jpg", folder / f"{photo_id}_t.jpg"
            path.write_bytes(jpeg)
            thumb.write_bytes(self._thumbnail(jpeg))
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
        scale = THUMB_WIDTH / image.shape[1]
        small = cv2.resize(image, (THUMB_WIDTH, max(1, int(image.shape[0] * scale))), interpolation=cv2.INTER_AREA)
        ok, buf = cv2.imencode(".jpg", small, [cv2.IMWRITE_JPEG_QUALITY, 80])
        if not ok:
            raise ValueError("thumbnail encode failed")
        return buf.tobytes()

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
        if not path.is_file() or self.folder.resolve() not in path.resolve().parents:
            return None
        return path

    def remove(self, photo_id: int) -> bool:
        row = self.get(photo_id)
        if row is None:
            return False
        for key in ("path", "thumb_path"):
            if row[key]:
                try:
                    Path(row[key]).unlink(missing_ok=True)
                except OSError:
                    log.warning("could not delete %s", row[key])
        self.db.execute("DELETE FROM photos WHERE id=?", (photo_id,))
        return True

    def usage(self) -> tuple[int, int]:
        row = self.db.one("SELECT COUNT(*) AS n, COALESCE(SUM(size_bytes), 0) AS bytes FROM photos")
        return row["n"], row["bytes"]
