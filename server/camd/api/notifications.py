from fastapi import APIRouter, Depends, HTTPException

from ..messages import translate
from .deps import Services, current_session, services
from .schemas import EndpointBody, PushBody

router = APIRouter(prefix="/api/push", dependencies=[Depends(current_session)])


@router.get("/key")
def public_key(svc: Services = Depends(services)):
    return {"key": svc.rt.notifier.web.public_key}


@router.post("/subscribe")
def subscribe(body: PushBody, svc: Services = Depends(services)):
    if "endpoint" not in body.subscription:
        raise HTTPException(400, "invalid subscription")
    svc.rt.notifier.web.subscribe(body.subscription)
    return {"ok": True}


@router.post("/unsubscribe")
def unsubscribe(body: EndpointBody, svc: Services = Depends(services)):
    svc.rt.notifier.web.unsubscribe(body.endpoint)
    return {"ok": True}


@router.post("/test")
def test(svc: Services = Depends(services)):
    language = svc.cfg.language
    sent = svc.rt.notifier.send(translate(language, "test_title"), translate(language, "test_body", site=svc.cfg.site_name), "/", "test")
    return {"sent": sent}
