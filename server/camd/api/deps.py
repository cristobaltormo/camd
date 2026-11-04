from dataclasses import dataclass

from fastapi import HTTPException, Request

from ..auth import Auth
from ..config import Config
from ..db import Database
from ..runtime import Runtime
from ..viewers import ViewerPool

COOKIE = "cam_session"


@dataclass
class Services:
    cfg: Config
    db: Database
    rt: Runtime
    auth: Auth
    viewers: ViewerPool
    build: str = "dev"


def services(request: Request) -> Services:
    return request.app.state.services


def client_ip(request: Request, trust_proxy: bool) -> str:
    if trust_proxy:
        forwarded = request.headers.get("cf-connecting-ip") or request.headers.get("x-forwarded-for", "").split(",")[0].strip()
        if forwarded:
            return forwarded
    return request.client.host if request.client else "unknown"


def throttle(request: Request) -> None:
    svc = services(request)
    if not svc.auth.limiter.allow(client_ip(request, svc.cfg.trust_proxy)):
        raise HTTPException(429, "too many attempts")


def current_session(request: Request) -> int:
    sid = services(request).auth.check_session(request.cookies.get(COOKIE))
    if sid is None:
        raise HTTPException(401, "login required")
    return sid
