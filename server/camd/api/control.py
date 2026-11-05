from fastapi import APIRouter, Depends, HTTPException

from .. import __version__, prefs
from ..arming import MODES
from ..detect import COCO_LABELS
from .deps import Services, current_session, services
from .schemas import ModeBody, PrivacyBody, SettingsBody, SnoozeBody

router = APIRouter(prefix="/api", dependencies=[Depends(current_session)])


@router.get("/status")
def status(svc: Services = Depends(services)):
    return svc.rt.status(__version__, svc.build)


@router.post("/mode")
def set_mode(body: ModeBody, svc: Services = Depends(services)):
    if body.mode not in MODES:
        raise HTTPException(400, "invalid mode")
    svc.rt.set_mode(body.mode)
    return {"ok": True}


@router.post("/snooze")
def snooze(body: SnoozeBody, svc: Services = Depends(services)):
    svc.rt.arming.snooze(body.minutes)
    return {"ok": True}


@router.post("/privacy")
def privacy(body: PrivacyBody, svc: Services = Depends(services)):
    if not svc.cfg.camera_enabled:
        raise HTTPException(409, "camera not available")
    if not svc.rt.set_privacy(body.on):
        raise HTTPException(502, "camera did not answer")
    return {"ok": True}


@router.get("/settings")
def get_settings(svc: Services = Depends(services)):
    motion = svc.rt.load_motion_settings()
    return {
        "sensitivity": motion.sensitivity,
        "ignore_rects": motion.ignore_rects,
        "alert_rects": motion.alert_rects,
        "retention_days": svc.cfg.retention_days,
        "available_labels": list(COCO_LABELS),
        **prefs.snapshot(svc.cfg),
    }


@router.post("/settings")
def post_settings(body: SettingsBody, svc: Services = Depends(services)):
    data = body.model_dump(exclude_none=True)
    motion = svc.rt.load_motion_settings()
    sensitivity = data.pop("sensitivity", motion.sensitivity)
    rects = data.pop("ignore_rects", motion.ignore_rects)
    alert_rects = data.pop("alert_rects", motion.alert_rects)
    retention = data.pop("retention_days", svc.cfg.retention_days)
    if any(len(rect) != 4 or any(not 0 <= v <= 1 for v in rect) for rect in rects + alert_rects):
        raise HTTPException(400, "invalid rect")
    try:
        changed = prefs.update(svc.cfg, svc.db, data)
    except ValueError as exc:
        raise HTTPException(400, str(exc))
    svc.rt.save_motion_settings(sensitivity, rects, alert_rects)
    svc.cfg.retention_days = retention
    svc.db.set_setting("retention_days", retention)
    svc.rt.apply_prefs(changed)
    return {"ok": True}
