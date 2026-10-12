import hashlib
import secrets
import threading
import time
from collections import defaultdict, deque

from webauthn import (
    base64url_to_bytes,
    generate_authentication_options,
    generate_registration_options,
    options_to_json,
    verify_authentication_response,
    verify_registration_response,
)
from webauthn.helpers import bytes_to_base64url
from webauthn.helpers.structs import (
    AuthenticatorSelectionCriteria,
    PublicKeyCredentialDescriptor,
    ResidentKeyRequirement,
    UserVerificationRequirement,
)

from .config import Config
from .db import Database

SESSION_TTL = 30 * 86400
ENROLL_TTL = 600
CHALLENGE_TTL = 300
USER_ID = b"owner"
USER_NAME = "cristobal"


class AuthError(Exception):
    pass


class RateLimiter:
    def __init__(self, limit: int, window: float):
        self.limit = limit
        self.window = window
        self._hits: dict[str, deque[float]] = defaultdict(deque)
        self._lock = threading.Lock()

    def allow(self, key: str) -> bool:
        now = time.time()
        with self._lock:
            hits = self._hits[key]
            while hits and now - hits[0] > self.window:
                hits.popleft()
            if len(hits) >= self.limit:
                return False
            hits.append(now)
            return True


def _hash(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


class Auth:
    """Passkey-only login for a single owner, with server-side revocable sessions."""

    def __init__(self, cfg: Config, db: Database):
        self.cfg = cfg
        self.db = db
        self._challenges: dict[str, tuple[bytes, float]] = {}
        self._lock = threading.Lock()
        self.limiter = RateLimiter(limit=10, window=60)

    def _put_challenge(self, key: str, challenge: bytes) -> None:
        with self._lock:
            now = time.time()
            self._challenges = {k: v for k, v in self._challenges.items() if v[1] > now}
            self._challenges[key] = (challenge, now + CHALLENGE_TTL)

    def _pop_challenge(self, key: str) -> bytes:
        with self._lock:
            entry = self._challenges.pop(key, None)
        if entry is None or entry[1] < time.time():
            raise AuthError("challenge expired")
        return entry[0]

    def has_credentials(self) -> bool:
        return self.db.one("SELECT 1 FROM credentials LIMIT 1") is not None

    def new_enroll_code(self) -> str:
        code = secrets.token_hex(4)
        self.db.execute("DELETE FROM enroll_codes WHERE expires < ?", (time.time(),))
        self.db.execute(
            "INSERT INTO enroll_codes(code_hash, expires) VALUES(?, ?)", (_hash(code), time.time() + ENROLL_TTL)
        )
        return code

    def _check_code(self, code: str) -> None:
        row = self.db.one("SELECT expires FROM enroll_codes WHERE code_hash=?", (_hash(code),))
        if row is None or row["expires"] < time.time():
            raise AuthError("invalid or expired code")

    def registration_options(self, code: str) -> str:
        self._check_code(code)
        existing = [
            PublicKeyCredentialDescriptor(id=base64url_to_bytes(r["id"]))
            for r in self.db.query("SELECT id FROM credentials")
        ]
        options = generate_registration_options(
            rp_id=self.cfg.rp_id,
            rp_name="Habitacion",
            user_id=USER_ID,
            user_name=USER_NAME,
            exclude_credentials=existing,
            authenticator_selection=AuthenticatorSelectionCriteria(
                resident_key=ResidentKeyRequirement.PREFERRED,
                user_verification=UserVerificationRequirement.REQUIRED,
            ),
        )
        self._put_challenge("reg:" + _hash(code), options.challenge)
        return options_to_json(options)

    def register(self, code: str, credential: dict, name: str, user_agent: str) -> str:
        self._check_code(code)
        challenge = self._pop_challenge("reg:" + _hash(code))
        try:
            verified = verify_registration_response(
                credential=credential,
                expected_challenge=challenge,
                expected_rp_id=self.cfg.rp_id,
                expected_origin=self.cfg.origin,
                require_user_verification=True,
            )
        except Exception as exc:
            raise AuthError(f"registration failed: {exc}") from exc
        self.db.execute(
            "INSERT INTO credentials(id, public_key, sign_count, name, created) VALUES(?, ?, ?, ?, ?)",
            (
                bytes_to_base64url(verified.credential_id),
                verified.credential_public_key,
                verified.sign_count,
                (name or "dispositivo")[:40],
                time.time(),
            ),
        )
        self.db.execute("DELETE FROM enroll_codes WHERE code_hash=?", (_hash(code),))
        return self.create_session(user_agent)

    def login_options(self) -> tuple[str, str]:
        creds = [
            PublicKeyCredentialDescriptor(id=base64url_to_bytes(r["id"]))
            for r in self.db.query("SELECT id FROM credentials")
        ]
        if not creds:
            raise AuthError("no credentials enrolled")
        options = generate_authentication_options(
            rp_id=self.cfg.rp_id,
            allow_credentials=creds,
            user_verification=UserVerificationRequirement.REQUIRED,
        )
        cid = secrets.token_urlsafe(16)
        self._put_challenge("login:" + cid, options.challenge)
        return cid, options_to_json(options)

    def login(self, cid: str, credential: dict, user_agent: str) -> str:
        challenge = self._pop_challenge("login:" + cid)
        cred_id = credential.get("id", "")
        row = self.db.one("SELECT * FROM credentials WHERE id=?", (cred_id,))
        if row is None:
            raise AuthError("unknown credential")
        try:
            verified = verify_authentication_response(
                credential=credential,
                expected_challenge=challenge,
                expected_rp_id=self.cfg.rp_id,
                expected_origin=self.cfg.origin,
                credential_public_key=bytes(row["public_key"]),
                credential_current_sign_count=row["sign_count"],
                require_user_verification=True,
            )
        except Exception as exc:
            raise AuthError(f"login failed: {exc}") from exc
        self.db.execute("UPDATE credentials SET sign_count=? WHERE id=?", (verified.new_sign_count, cred_id))
        return self.create_session(user_agent)

    def create_session(self, user_agent: str) -> str:
        token = secrets.token_urlsafe(32)
        now = time.time()
        self.db.execute(
            "INSERT INTO sessions(token_hash, created, expires, last_seen, user_agent) VALUES(?, ?, ?, ?, ?)",
            (_hash(token), now, now + SESSION_TTL, now, user_agent[:200]),
        )
        return token

    def check_session(self, token: str | None) -> int | None:
        if not token:
            return None
        row = self.db.one("SELECT id, expires FROM sessions WHERE token_hash=?", (_hash(token),))
        if row is None or row["expires"] < time.time():
            return None
        self.db.execute("UPDATE sessions SET last_seen=? WHERE id=?", (time.time(), row["id"]))
        return row["id"]

    def revoke(self, session_id: int) -> None:
        self.db.execute("DELETE FROM sessions WHERE id=?", (session_id,))

    def revoke_all(self) -> None:
        self.db.execute("DELETE FROM sessions")

    def sessions(self) -> list[dict]:
        return [
            {"id": r["id"], "created": r["created"], "last_seen": r["last_seen"], "user_agent": r["user_agent"]}
            for r in self.db.query("SELECT * FROM sessions WHERE expires > ? ORDER BY last_seen DESC", (time.time(),))
        ]

    def credentials(self) -> list[dict]:
        return [
            {"id": r["id"], "name": r["name"], "created": r["created"]}
            for r in self.db.query("SELECT id, name, created FROM credentials ORDER BY created")
        ]

    def delete_credential(self, cred_id: str) -> None:
        self.db.execute("DELETE FROM credentials WHERE id=?", (cred_id,))
