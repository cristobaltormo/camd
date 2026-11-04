import ipaddress
from urllib.parse import urlsplit

from fastapi import APIRouter, Depends, HTTPException

from ..messages import translate
from .deps import Services, current_session, services
from .schemas import EndpointBody, PushBody


def is_private_host(host: str) -> bool:
    if host in ("", "localhost") or host.endswith(".local"):
        return True
    try:
        address = ipaddress.ip_address(host)
    except ValueError:
        return False
    return not address.is_global


router = APIRouter(prefix="/api/push", dependencies=[Depends(current_session)])


@router.get("/key")
def public_key(svc: Services = Depends(services)):
    return {"key": svc.rt.notifier.web.public_key}


@router.post("/subscribe")
def subscribe(body: PushBody, svc: Services = Depends(services)):
    endpoint = body.subscription.get("endpoint", "")
    host = urlsplit(endpoint).hostname or ""
    if urlsplit(endpoint).scheme != "https" or is_private_host(host):
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
