import base64
import json
import logging
from pathlib import Path

from cryptography.hazmat.primitives import serialization
from py_vapid import Vapid
import requests
from pywebpush import WebPushException, webpush

from .db import Database

log = logging.getLogger("camd.push")


class Push:
    def __init__(self, db: Database, key_path: Path, subject: str, notifich_url: str = "", notifich_key: str = ""):
        self.db = db
        self.notifich_url = notifich_url.rstrip("/")
        self.notifich_key = notifich_key
        self.subject = subject
        self.key_path = key_path
        if not key_path.exists():
            vapid = Vapid()
            vapid.generate_keys()
            key_path.parent.mkdir(parents=True, exist_ok=True)
            vapid.save_key(str(key_path))
            key_path.chmod(0o600)
        self._vapid = Vapid.from_file(str(key_path))

    @property
    def public_key(self) -> str:
        raw = self._vapid.public_key.public_bytes(
            serialization.Encoding.X962, serialization.PublicFormat.UncompressedPoint
        )
        return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()

    def subscribe(self, subscription: dict) -> None:
        endpoint = subscription["endpoint"]
        self.db.execute(
            "INSERT INTO push_subs(endpoint, data) VALUES(?, ?) "
            "ON CONFLICT(endpoint) DO UPDATE SET data=excluded.data",
            (endpoint, json.dumps(subscription)),
        )

    def unsubscribe(self, endpoint: str) -> None:
        self.db.execute("DELETE FROM push_subs WHERE endpoint=?", (endpoint,))

    @property
    def notifich_enabled(self) -> bool:
        return bool(self.notifich_url and self.notifich_key)

    def _send_notifich(self, title: str, body: str) -> bool:
        try:
            res = requests.post(
                f"{self.notifich_url}/api/notify",
                json={"title": title, "body": body},
                headers={"X-API-Key": self.notifich_key},
                timeout=10,
            )
            if res.ok:
                return True
            log.warning("notifich answered %s", res.status_code)
        except requests.RequestException as exc:
            log.warning("notifich failed: %s", exc)
        return False

    def count(self) -> int:
        return self.db.one("SELECT COUNT(*) AS n FROM push_subs")["n"]

    def send(self, title: str, body: str, url: str = "/", tag: str = "camd") -> int:
        payload = json.dumps({"title": title, "body": body, "url": url, "tag": tag})
        sent = 1 if self.notifich_enabled and self._send_notifich(title, body) else 0
        for row in self.db.query("SELECT endpoint, data FROM push_subs"):
            try:
                webpush(
                    subscription_info=json.loads(row["data"]),
                    data=payload,
                    vapid_private_key=str(self.key_path),
                    vapid_claims={"sub": self.subject},
                    ttl=3600,
                )
                sent += 1
            except WebPushException as exc:
                status = getattr(exc.response, "status_code", None)
                if status in (404, 410):
                    self.unsubscribe(row["endpoint"])
                else:
                    log.warning("push failed: %s", exc)
            except Exception:
                log.exception("push error")
        return sent
