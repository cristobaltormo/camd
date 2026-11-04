from fastapi import APIRouter, Depends, HTTPException
from fastapi.responses import FileResponse

from ..photos import PhotoLimit
from .deps import Services, current_session, services
from .live import fresh_frame
from .schemas import IdsBody

router = APIRouter(prefix="/api/photos", dependencies=[Depends(current_session)])


def photo_file(svc: Services, photo_id: int, thumb: bool, download: bool = False) -> FileResponse:
    path = svc.rt.photos.file(photo_id, thumb)
    if path is None:
        raise HTTPException(404, "not found")
    filename = f"photo-{int(svc.rt.photos.get(photo_id)['ts'])}.jpg" if download else None
    return FileResponse(path, media_type="image/jpeg", filename=filename, headers={"Cache-Control": "private, max-age=3600"})


@router.post("")
def take(svc: Services = Depends(services)):
    ts, jpeg = fresh_frame(svc)
    try:
        return svc.rt.photos.save(jpeg, ts)
    except PhotoLimit as exc:
        raise HTTPException(409, str(exc))
    except ValueError as exc:
        raise HTTPException(422, str(exc))


@router.get("")
def list_photos(limit: int = 60, before: float | None = None, svc: Services = Depends(services)):
    return {"photos": svc.rt.photos.list(max(1, min(limit, 200)), before)}


@router.get("/{photo_id}/img")
def image(photo_id: int, download: bool = False, svc: Services = Depends(services)):
    return photo_file(svc, photo_id, False, download)


@router.get("/{photo_id}/thumb")
def thumbnail(photo_id: int, svc: Services = Depends(services)):
    return photo_file(svc, photo_id, True)


@router.delete("/{photo_id}")
def delete(photo_id: int, svc: Services = Depends(services)):
    if not svc.rt.photos.remove(photo_id):
        raise HTTPException(404, "not found")
    return {"ok": True}


@router.post("/delete")
def delete_many(body: IdsBody, svc: Services = Depends(services)):
    return {"removed": sum(svc.rt.photos.remove(photo_id) for photo_id in body.ids)}
