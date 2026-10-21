from fastapi import APIRouter, Depends, HTTPException, Response
from fastapi.responses import StreamingResponse

from .deps import Services, current_session, services
from .schemas import ClipBody

router = APIRouter(prefix="/api", dependencies=[Depends(current_session)])

BOUNDARY = "frame"


def fresh_frame(svc: Services) -> tuple[float, bytes]:
    grabber = svc.rt.grabber
    latest = grabber.latest if svc.cfg.camera_enabled else None
    if latest is None or not grabber.online:
        raise HTTPException(503, "no frame")
    return latest


@router.get("/snapshot")
def snapshot(download: bool = False, svc: Services = Depends(services)):
    ts, jpeg = fresh_frame(svc)
    headers = {"Cache-Control": "no-store"}
    if download:
        headers["Content-Disposition"] = f'attachment; filename="snapshot-{int(ts)}.jpg"'
    return Response(jpeg, media_type="image/jpeg", headers=headers)


@router.get("/live")
def live(svc: Services = Depends(services)):
    grabber = svc.rt.grabber
    if grabber.standby or not svc.cfg.camera_enabled:
        raise HTTPException(503, "camera in standby")
    cancel = svc.viewers.join()

    def generate():
        grabber.viewers += 1
        try:
            for jpeg in grabber.frames():
                if cancel.is_set():
                    return
                yield b"--%s\r\nContent-Type: image/jpeg\r\nContent-Length: %d\r\n\r\n" % (BOUNDARY.encode(), len(jpeg))
                yield jpeg
                yield b"\r\n"
        finally:
            grabber.viewers -= 1
            svc.viewers.leave(cancel)

    return StreamingResponse(generate(), media_type=f"multipart/x-mixed-replace; boundary={BOUNDARY}", headers={"Cache-Control": "no-store"})


@router.post("/clips/start")
def clip_start(body: ClipBody, svc: Services = Depends(services)):
    grabber = svc.rt.grabber
    if not svc.cfg.camera_enabled or grabber.standby or not grabber.online:
        raise HTTPException(409, "camera not available")
    svc.rt.recorder.start_manual(body.seconds)
    return svc.rt.recorder.manual_state()


@router.post("/clips/stop")
def clip_stop(svc: Services = Depends(services)):
    if svc.rt.recorder.stop_manual() is None:
        raise HTTPException(404, "no clip in progress")
    return {"ok": True}
