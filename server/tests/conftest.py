import sys
import time
from dataclasses import dataclass
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from authenticator import SoftAuthenticator  # noqa: E402
from frames import jpeg, scene  # noqa: E402

from camd.app import create_app  # noqa: E402
from camd.config import Config  # noqa: E402
from camd.db import Database  # noqa: E402

ORIGIN = "http://localhost:8180"
HEADERS = {"Origin": ORIGIN}


@dataclass
class Env:
    cfg: Config
    db: Database
    app: object
    client: TestClient

    @property
    def rt(self):
        return self.app.state.services.rt

    @property
    def auth(self):
        return self.app.state.services.auth

    def post(self, path: str, body: dict | None = None):
        return self.client.post(path, json=body if body is not None else {}, headers=HEADERS)

    def delete(self, path: str):
        return self.client.delete(path, headers=HEADERS)

    def get(self, path: str, **kwargs):
        return self.client.get(path, **kwargs)

    def frame(self, count: int = 1) -> bytes:
        data = jpeg(scene())
        for _ in range(count):
            self.rt.grabber._publish(time.time(), data)
        return data


def make_env(tmp_path, extras=None, **overrides) -> Env:
    cfg = Config(data_dir=tmp_path, rp_id="localhost", origin=ORIGIN, camera_token="t", detection="off", **overrides)
    db = Database(cfg.db_path)
    app = create_app(cfg, db, start=False, extras=extras)
    return Env(cfg, db, app, TestClient(app, base_url=ORIGIN))


def enroll(env: Env, authenticator: SoftAuthenticator) -> None:
    code = env.auth.new_enroll_code()
    options = env.post("/api/auth/register/options", {"code": code}).json()
    response = env.post("/api/auth/register/verify", {"code": code, "credential": authenticator.create(options), "name": "test"})
    assert response.status_code == 200, response.text


@pytest.fixture
def anonymous(tmp_path) -> Env:
    return make_env(tmp_path)


@pytest.fixture
def env(tmp_path) -> Env:
    environment = make_env(tmp_path)
    enroll(environment, SoftAuthenticator("localhost", ORIGIN))
    return environment
