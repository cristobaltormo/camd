import json
import logging

from fastapi import APIRouter, Depends, HTTPException, Request, Response

from ..auth import SESSION_TTL, AuthError
from .deps import COOKIE, Services, current_session, services, throttle
from .schemas import CodeBody, LoginBody, PasswordBody, RegisterBody

log = logging.getLogger("camd.api.auth")
router = APIRouter(prefix="/api/auth")


def set_cookie(response: Response, token: str, svc: Services) -> None:
    response.set_cookie(COOKIE, token, max_age=SESSION_TTL, httponly=True, samesite="strict", secure=svc.cfg.secure_cookies, path="/")


@router.get("/state")
def state(request: Request, svc: Services = Depends(services)):
    return {
        "passkeys": svc.auth.has_credentials(),
        "password": svc.auth.has_password(),
        "logged_in": svc.auth.check_session(request.cookies.get(COOKIE)) is not None,
        "site_name": svc.cfg.site_name,
        "language": svc.cfg.language,
    }


@router.post("/register/options", dependencies=[Depends(throttle)])
def register_options(body: CodeBody, svc: Services = Depends(services)):
    try:
        return Response(svc.auth.registration_options(body.code), media_type="application/json")
    except AuthError as exc:
        raise HTTPException(400, str(exc))


@router.post("/register/verify", dependencies=[Depends(throttle)])
def register_verify(body: RegisterBody, request: Request, response: Response, svc: Services = Depends(services)):
    try:
        token = svc.auth.register(body.code, body.credential, body.name, request.headers.get("user-agent", ""))
    except AuthError as exc:
        raise HTTPException(400, str(exc))
    set_cookie(response, token, svc)
    return {"ok": True}


@router.post("/login/options", dependencies=[Depends(throttle)])
def login_options(svc: Services = Depends(services)):
    try:
        cid, options = svc.auth.login_options()
    except AuthError as exc:
        raise HTTPException(400, str(exc))
    return {"cid": cid, "options": json.loads(options)}


@router.post("/login/verify", dependencies=[Depends(throttle)])
def login_verify(body: LoginBody, request: Request, response: Response, svc: Services = Depends(services)):
    try:
        token = svc.auth.login(body.cid, body.credential, request.headers.get("user-agent", ""))
    except AuthError as exc:
        log.warning("passkey login rejected: %s", exc)
        raise HTTPException(401, "login failed")
    set_cookie(response, token, svc)
    return {"ok": True}


@router.post("/login/password", dependencies=[Depends(throttle)])
def login_password(body: PasswordBody, request: Request, response: Response, svc: Services = Depends(services)):
    try:
        token = svc.auth.login_password(body.password, request.headers.get("user-agent", ""))
    except AuthError:
        raise HTTPException(401, "login failed")
    set_cookie(response, token, svc)
    return {"ok": True}


@router.post("/logout")
def logout(response: Response, sid: int = Depends(current_session), svc: Services = Depends(services)):
    svc.auth.revoke(sid)
    response.delete_cookie(COOKIE, path="/")
    return {"ok": True}


@router.post("/enroll-code")
def enroll_code(_: int = Depends(current_session), svc: Services = Depends(services)):
    return {"code": svc.auth.new_enroll_code(), "expires_in": 600}


@router.get("/devices")
def devices(sid: int = Depends(current_session), svc: Services = Depends(services)):
    return {"credentials": svc.auth.credentials(), "sessions": svc.auth.sessions(), "current_session": sid, "password": svc.auth.has_password()}


@router.delete("/sessions/{session_id}")
def delete_session(session_id: int, _: int = Depends(current_session), svc: Services = Depends(services)):
    svc.auth.revoke(session_id)
    return {"ok": True}


@router.delete("/credentials/{credential_id}")
def delete_credential(credential_id: str, _: int = Depends(current_session), svc: Services = Depends(services)):
    if len(svc.auth.credentials()) <= 1 and not svc.auth.has_password():
        raise HTTPException(400, "cannot remove the last way to sign in")
    svc.auth.delete_credential(credential_id)
    return {"ok": True}
