import hashlib
import json
import os
import struct

import cbor2
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import ec
from webauthn.helpers import bytes_to_base64url as b64u


class SoftAuthenticator:
    def __init__(self, rp_id: str, origin: str):
        self.rp_hash = hashlib.sha256(rp_id.encode()).digest()
        self.origin = origin
        self.key = ec.generate_private_key(ec.SECP256R1())
        self.cred_id = os.urandom(16)
        self.counter = 0

    def _client_data(self, kind: str, challenge: str) -> bytes:
        return json.dumps({"type": kind, "challenge": challenge, "origin": self.origin}).encode()

    def create(self, options: dict) -> dict:
        nums = self.key.public_key().public_numbers()
        cose = {1: 2, 3: -7, -1: 1, -2: nums.x.to_bytes(32, "big"), -3: nums.y.to_bytes(32, "big")}
        attested = bytes(16) + struct.pack(">H", len(self.cred_id)) + self.cred_id + cbor2.dumps(cose)
        auth_data = self.rp_hash + bytes([0x45]) + struct.pack(">I", self.counter) + attested
        attestation = cbor2.dumps({"fmt": "none", "attStmt": {}, "authData": auth_data})
        return {
            "id": b64u(self.cred_id),
            "rawId": b64u(self.cred_id),
            "type": "public-key",
            "response": {
                "clientDataJSON": b64u(self._client_data("webauthn.create", options["challenge"])),
                "attestationObject": b64u(attestation),
            },
            "clientExtensionResults": {},
        }

    def get(self, options: dict, corrupt: bool = False) -> dict:
        self.counter += 1
        client = self._client_data("webauthn.get", options["challenge"])
        auth_data = self.rp_hash + bytes([0x05]) + struct.pack(">I", self.counter)
        signed = auth_data + hashlib.sha256(client).digest()
        signature = self.key.sign(signed, ec.ECDSA(hashes.SHA256()))
        if corrupt:
            signature = signature[:-1] + bytes([signature[-1] ^ 1])
        return {
            "id": b64u(self.cred_id),
            "rawId": b64u(self.cred_id),
            "type": "public-key",
            "response": {
                "clientDataJSON": b64u(client),
                "authenticatorData": b64u(auth_data),
                "signature": b64u(signature),
                "userHandle": b64u(b"owner"),
            },
            "clientExtensionResults": {},
        }
