from authenticator import SoftAuthenticator
from conftest import HEADERS, ORIGIN, enroll
from fastapi.testclient import TestClient

from camd.auth import check_password, hash_password


def test_everything_requires_a_session(anonymous):
    for path in ("/api/status", "/api/events", "/api/live", "/api/snapshot", "/api/settings", "/api/push/key", "/api/photos"):
        assert anonymous.get(path).status_code == 401
    assert anonymous.get("/healthz").status_code == 200
    assert anonymous.get("/").status_code == 200


def test_register_login_logout(anonymous):
    authenticator = SoftAuthenticator("localhost", ORIGIN)
    enroll(anonymous, authenticator)
    assert anonymous.get("/api/status").status_code == 200

    other = TestClient(anonymous.app, base_url=ORIGIN)
    options = other.post("/api/auth/login/options", headers=HEADERS).json()
    login = other.post("/api/auth/login/verify", json={"cid": options["cid"], "credential": authenticator.get(options["options"])}, headers=HEADERS)
    assert login.status_code == 200 and other.get("/api/status").status_code == 200

    other.post("/api/auth/logout", headers=HEADERS)
    assert other.get("/api/status").status_code == 401
    assert anonymous.get("/api/status").status_code == 200


def test_bad_signature_and_replay_are_rejected(anonymous):
    authenticator = SoftAuthenticator("localhost", ORIGIN)
    enroll(anonymous, authenticator)
    fresh = TestClient(anonymous.app, base_url=ORIGIN)

    options = fresh.post("/api/auth/login/options", headers=HEADERS).json()
    corrupt = authenticator.get(options["options"], corrupt=True)
    assert fresh.post("/api/auth/login/verify", json={"cid": options["cid"], "credential": corrupt}, headers=HEADERS).status_code == 401

    options = fresh.post("/api/auth/login/options", headers=HEADERS).json()
    credential = authenticator.get(options["options"])
    assert fresh.post("/api/auth/login/verify", json={"cid": options["cid"], "credential": credential}, headers=HEADERS).status_code == 200
    replay = TestClient(anonymous.app, base_url=ORIGIN)
    assert replay.post("/api/auth/login/verify", json={"cid": options["cid"], "credential": credential}, headers=HEADERS).status_code == 401


def test_enroll_code_is_single_use(anonymous):
    assert anonymous.post("/api/auth/register/options", {"code": "deadbeef"}).status_code == 400
    authenticator = SoftAuthenticator("localhost", ORIGIN)
    code = anonymous.auth.new_enroll_code()
    options = anonymous.post("/api/auth/register/options", {"code": code}).json()
    body = {"code": code, "credential": authenticator.create(options)}
    assert anonymous.post("/api/auth/register/verify", body).status_code == 200
    assert anonymous.post("/api/auth/register/verify", body).status_code == 400


def test_wrong_origin_is_blocked_on_writes(env):
    assert env.client.post("/api/mode", json={"mode": "armed"}, headers={"Origin": "https://evil.example"}).status_code == 403
    assert env.client.post("/api/mode", json={"mode": "armed"}).status_code == 403
    assert env.post("/api/mode", {"mode": "armed"}).status_code == 200


def test_attempts_are_rate_limited(anonymous):
    codes = [anonymous.post("/api/auth/register/options", {"code": f"abcd{i:04d}"}).status_code for i in range(14)]
    assert 429 in codes


def test_the_last_way_in_cannot_be_removed(env):
    credential = env.get("/api/auth/devices").json()["credentials"][0]["id"]
    assert env.delete(f"/api/auth/credentials/{credential}").status_code == 400
    env.auth.set_password("a long enough password")
    assert env.delete(f"/api/auth/credentials/{credential}").status_code == 200


def test_password_login(anonymous):
    assert anonymous.get("/api/auth/state").json()["password"] is False
    anonymous.auth.set_password("correct horse battery")
    assert anonymous.get("/api/auth/state").json()["password"] is True
    assert anonymous.post("/api/auth/login/password", {"password": "wrong"}).status_code == 401
    assert anonymous.get("/api/status").status_code == 401
    assert anonymous.post("/api/auth/login/password", {"password": "correct horse battery"}).status_code == 200
    assert anonymous.get("/api/status").status_code == 200


def test_password_rules_and_hashing():
    stored = hash_password("long enough password")
    assert check_password("long enough password", stored)
    assert not check_password("another one entirely", stored)
    assert not check_password("anything", None)
    assert hash_password("long enough password") != stored


def test_short_password_is_refused(anonymous):
    import pytest

    from camd.auth import AuthError

    with pytest.raises(AuthError):
        anonymous.auth.set_password("short")


def test_a_request_from_the_same_host_is_accepted_even_if_it_is_not_the_configured_origin(env):
    other = {"Origin": "http://192.168.1.50:8180", "Host": "192.168.1.50:8180"}
    assert env.client.post("/api/mode", json={"mode": "armed"}, headers=other).status_code == 200
    forged = {"Origin": "http://evil.example", "Host": "192.168.1.50:8180"}
    assert env.client.post("/api/mode", json={"mode": "armed"}, headers=forged).status_code == 403


def test_the_client_address_comes_from_the_proxy_not_from_the_client(env):
    from camd.api.deps import client_ip

    class Req:
        client = type("C", (), {"host": "10.0.0.2"})()

        def __init__(self, headers):
            self.headers = headers

    forged = Req({"x-forwarded-for": "1.2.3.4, 203.0.113.9", "cf-connecting-ip": "6.6.6.6"})
    assert client_ip(forged, True) == "203.0.113.9"
    assert client_ip(forged, False) == "10.0.0.2"
    assert client_ip(Req({"cf-connecting-ip": "6.6.6.6"}), True) == "6.6.6.6"
