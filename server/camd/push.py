import base64
import json
import logging
from pathlib import Path
from typing import Protocol

import requests
from cryptography.hazmat.primitives import serialization
from py_vapid import Vapid
from pywebpush import WebPushException, webpush

from .config import Config
from .db import Database

log = logging.getLogger("camd.push")

REQUEST_TIMEOUT_S = 10
WEB_PUSH_TTL_S = 3600
GONE = (404, 410)


class Channel(Protocol):
    name: str

    def send(self, title: str, body: str, url: str) -> bool: ...


class WebPush:
    name = "webpush"

    def __init__(self, db: Database, key_path: Path, subject: str):
        self.db = db
        self.key_path = key_path
        self.subject = subject
        if not key_path.exists():
            vapid = Vapid()
            vapid.generate_keys()
            key_path.parent.mkdir(parents=True, exist_ok=True)
            vapid.save_key(str(key_path))
            key_path.chmod(0o600)
        self._vapid = Vapid.from_file(str(key_path))

    @property
    def public_key(self) -> str:
        raw = self._vapid.public_key.public_bytes(serialization.Encoding.X962, serialization.PublicFormat.UncompressedPoint)
        return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()

    def count(self) -> int:
        return self.db.one("SELECT COUNT(*) AS n FROM push_subs")["n"]

    def subscribe(self, subscription: dict) -> None:
        self.db.execute(
            "INSERT INTO push_subs(endpoint, data) VALUES(?, ?) ON CONFLICT(endpoint) DO UPDATE SET data=excluded.data",
            (subscription["endpoint"], json.dumps(subscription)),
        )

    def unsubscribe(self, endpoint: str) -> None:
        self.db.execute("DELETE FROM push_subs WHERE endpoint=?", (endpoint,))

    def send_all(self, title: str, body: str, url: str, tag: str) -> int:
        payload = json.dumps({"title": title, "body": body, "url": url, "tag": tag})
        sent = 0
        for row in self.db.query("SELECT endpoint, data FROM push_subs"):
            try:
                webpush(
                    subscription_info=json.loads(row["data"]),
                    data=payload,
                    vapid_private_key=str(self.key_path),
                    vapid_claims={"sub": self.subject},
                    ttl=WEB_PUSH_TTL_S,
                )
                sent += 1
            except WebPushException as exc:
                if getattr(exc.response, "status_code", None) in GONE:
                    self.unsubscribe(row["endpoint"])
                else:
                    log.warning("web push failed: %s", exc)
            except Exception:
                log.exception("web push error")
        return sent


class Notifich:
    name = "notifich"

    def __init__(self, base_url: str, api_key: str):
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key

    def send(self, title: str, body: str, url: str) -> bool:
        try:
            response = requests.post(
                f"{self.base_url}/api/notify",
                json={"title": title, "body": body},
                headers={"X-API-Key": self.api_key},
                timeout=REQUEST_TIMEOUT_S,
            )
        except requests.RequestException as exc:
            log.warning("notifich failed: %s", exc)
            return False
        if not response.ok:
            log.warning("notifich answered %s", response.status_code)
        return response.ok


class Ntfy:
    name = "ntfy"

    def __init__(self, topic_url: str, token: str, origin: str):
        self.server, _, self.topic = topic_url.rstrip("/").rpartition("/")
        self.token = token
        self.origin = origin.rstrip("/")

    def send(self, title: str, body: str, url: str) -> bool:
        headers = {"Authorization": f"Bearer {self.token}"} if self.token else {}
        message = {"topic": self.topic, "title": title, "message": body, "click": self.origin + url}
        try:
            response = requests.post(self.server, json=message, headers=headers, timeout=REQUEST_TIMEOUT_S)
        except requests.RequestException as exc:
            log.warning("ntfy failed: %s", exc)
            return False
        if not response.ok:
            log.warning("ntfy answered %s", response.status_code)
        return response.ok


class Notifier:
    def __init__(self, cfg: Config, db: Database):
        self.web = WebPush(db, cfg.vapid_path, cfg.push_subject)
        self.channels: list[Channel] = []
        if cfg.notifich_url and cfg.notifich_key:
            self.channels.append(Notifich(cfg.notifich_url, cfg.notifich_key))
        if cfg.ntfy_url:
            self.channels.append(Ntfy(cfg.ntfy_url, cfg.ntfy_token, cfg.origin))

    def send(self, title: str, body: str, url: str = "/", tag: str = "camd") -> int:
        delivered = sum(channel.send(title, body, url) for channel in self.channels)
        return delivered + self.web.send_all(title, body, url, tag)
