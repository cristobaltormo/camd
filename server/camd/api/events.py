from pathlib import Path

from fastapi import APIRouter, Depends, HTTPException
from fastapi.responses import FileResponse

from ..events import event_json
from ..storage import remove_event
from .deps import Services, current_session, services
from .schemas import IdsBody, KeepBody

router = APIRouter(prefix="/api/events", dependencies=[Depends(current_session)])

CACHE_HEADER = {"Cache-Control": "private, max-age=3600"}
FINISHED = "status != 'recording'"
VISIBLE = "(analyzed=0 OR confirmed=1)"


def event_file(svc: Services, event_id: int, column: str, media_type: str, filename: str | None = None) -> FileResponse:
    row = svc.db.one("SELECT * FROM events WHERE id=?", (event_id,))
    path = Path(row[column]) if row and row[column] else None
    if path is None or not path.is_file() or svc.cfg.clips_dir.resolve() not in path.resolve().parents:
        raise HTTPException(404, "not found")
    return FileResponse(path, media_type=media_type, filename=filename, headers=CACHE_HEADER)


@router.get("")
def list_events(
    limit: int = 50,
    before: float | None = None,
    label: str | None = None,
    hide_unconfirmed: bool = False,
    svc: Services = Depends(services),
):
    clauses, params = [], []
    if before is not None:
        clauses.append("start_ts < ?")
        params.append(before)
    if label:
        clauses.append("labels LIKE ?")
        params.append(f'%"{label.replace(chr(34), "")}"%')
    if hide_unconfirmed:
        clauses.append(VISIBLE)
    where = f"WHERE {' AND '.join(clauses)}" if clauses else ""
    rows = svc.db.query(f"SELECT * FROM events {where} ORDER BY start_ts DESC LIMIT ?", (*params, max(1, min(limit, 200))))
    return {"events": [event_json(row) for row in rows]}


@router.get("/count")
def count_events(since: float = 0, svc: Services = Depends(services)):
    row = svc.db.one(f"SELECT COUNT(*) AS n FROM events WHERE start_ts > ? AND {VISIBLE}", (since,))
    return {"count": row["n"]}


@router.get("/{event_id}/clip")
def event_clip(event_id: int, download: bool = False, svc: Services = Depends(services)):
    return event_file(svc, event_id, "clip_path", "video/mp4", f"event-{event_id}.mp4" if download else None)


@router.get("/{event_id}/thumb")
def event_thumb(event_id: int, svc: Services = Depends(services)):
    return event_file(svc, event_id, "thumb_path", "image/jpeg")


@router.post("/delete")
def delete_events(body: IdsBody, svc: Services = Depends(services)):
    removed = 0
    for event_id in body.ids:
        row = svc.db.one(f"SELECT id, clip_path, thumb_path FROM events WHERE id=? AND {FINISHED}", (event_id,))
        if row is not None:
            remove_event(svc.db, row)
            removed += 1
    return {"removed": removed}


@router.delete("/{event_id}")
def delete_event(event_id: int, svc: Services = Depends(services)):
    row = svc.db.one(f"SELECT id, clip_path, thumb_path FROM events WHERE id=? AND {FINISHED}", (event_id,))
    if row is None:
        raise HTTPException(404, "not found")
    remove_event(svc.db, row)
    return {"ok": True}


@router.post("/{event_id}/keep")
def keep_event(event_id: int, body: KeepBody, svc: Services = Depends(services)):
    if svc.db.one(f"SELECT id FROM events WHERE id=? AND {FINISHED}", (event_id,)) is None:
        raise HTTPException(404, "not found")
    svc.db.execute("UPDATE events SET keep=? WHERE id=?", (int(body.keep), event_id))
    return {"ok": True, "keep": body.keep}
