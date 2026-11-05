import logging
import time
from pathlib import Path

from .config import Config
from .db import Database

log = logging.getLogger("camd.storage")

PURGE_INTERVAL_S = 3600


def unlink(path: str | None) -> None:
    if not path:
        return
    try:
        Path(path).unlink(missing_ok=True)
    except OSError:
        log.warning("could not delete %s", path)


def remove_event(db: Database, row) -> None:
    unlink(row["clip_path"])
    unlink(row["thumb_path"])
    db.execute("DELETE FROM events WHERE id=?", (row["id"],))


def purge(cfg: Config, db: Database) -> int:
    now = time.time()
    rows = db.query(
        "SELECT id, clip_path, thumb_path FROM events WHERE start_ts < ? AND status != 'recording' AND keep=0",
        (now - cfg.retention_days * 86400,),
    )
    if cfg.detection != "off":
        rows += db.query(
            "SELECT id, clip_path, thumb_path FROM events "
            "WHERE analyzed=1 AND confirmed=0 AND kind='motion' AND status != 'recording' AND keep=0 AND start_ts < ?",
            (now - cfg.unconfirmed_keep_h * 3600,),
        )
    removed = 0
    for row in {row["id"]: row for row in rows}.values():
        remove_event(db, row)
        removed += 1

    cap = int(cfg.max_disk_gb * 1024**3)
    while db.row("SELECT COALESCE(SUM(size_bytes), 0) AS total FROM events")["total"] > cap:
        oldest = db.one("SELECT id, clip_path, thumb_path FROM events WHERE status != 'recording' AND keep=0 ORDER BY start_ts LIMIT 1")
        if oldest is None:
            break
        remove_event(db, oldest)
        removed += 1
    return removed
