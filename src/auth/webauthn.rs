use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ciborium::Value as Cbor;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::{
    Auth, DEFAULT_DEVICE_NAME, MAX_DEVICE_NAME, USER_ID, random_bytes, sha256_hex, token_urlsafe,
    truncate_chars,
};
use crate::clock;
use crate::db::CredentialRow;
use crate::error::{Error, Result};

pub const TIMEOUT_MS: u64 = 60_000;
pub const ALGORITHMS: [i64; 3] = [-8, -7, -257];
pub const CHALLENGE_BYTES: usize = 64;

const FLAG_UP: u8 = 0x01;
const FLAG_UV: u8 = 0x04;
const FLAG_AT: u8 = 0x40;
const FLAG_ED: u8 = 0x80;

pub fn b64url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn b64url_decode(text: &str) -> Option<Vec<u8>> {
    let clean: String = text
        .trim_end_matches('=')
        .chars()
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            other => other,
        })
        .collect();
    URL_SAFE_NO_PAD.decode(clean).ok()
}

fn registration_key(code: &str) -> String {
    format!("reg:{}", sha256_hex(code))
}

fn login_key(cid: &str) -> String {
    format!("login:{cid}")
}

fn descriptors(ids: &[String]) -> Vec<Value> {
    ids.iter()
        .map(|id| {
            let canonical = b64url_decode(id).map(|b| b64url(&b)).unwrap_or(id.clone());
            json!({"id": canonical, "type": "public-key"})
        })
        .collect()
}

impl Auth {
    pub fn registration_options(&self, code: &str) -> Result<Value> {
        self.check_code(code)?;
        let existing = descriptors(&self.db().credential_ids()?);
        let challenge = random_bytes::<CHALLENGE_BYTES>();
        let params: Vec<Value> = ALGORITHMS
            .iter()
            .map(|alg| json!({"type": "public-key", "alg": alg}))
            .collect();
        let options = json!({
            "rp": {"name": self.site_name, "id": self.rp_id},
            "user": {"id": b64url(USER_ID), "name": self.site_name, "displayName": self.site_name},
            "challenge": b64url(&challenge),
            "pubKeyCredParams": params,
            "timeout": TIMEOUT_MS,
            "excludeCredentials": existing,
            "authenticatorSelection": {"residentKey": "preferred", "requireResidentKey": false, "userVerification": "required"},
            "attestation": "none",
        });
        self.put_challenge(&registration_key(code), challenge.to_vec());
        Ok(options)
    }

    pub fn register(
        &self,
        code: &str,
        credential: &Value,
        name: &str,
        user_agent: &str,
    ) -> Result<String> {
        self.check_code(code)?;
        let challenge = self.pop_challenge(&registration_key(code))?;
        let verified = verify_registration(credential, &challenge, &self.rp_id, &self.origin)
            .map_err(|e| Error::invalid(format!("registration failed: {e}")))?;
        let id = b64url(&verified.credential_id);
        if self.db().credential(&id)?.is_some() {
            return Err(Error::invalid(
                "registration failed: this passkey is already registered",
            ));
        }
        let name = if name.is_empty() {
            DEFAULT_DEVICE_NAME
        } else {
            name
        };
        self.db().insert_credential(&CredentialRow {
            id,
            public_key: verified.public_key,
            sign_count: i64::from(verified.sign_count),
            name: truncate_chars(name, MAX_DEVICE_NAME),
            created: clock::now(),
        })?;
        self.consume_code(code)?;
        self.create_session(user_agent)
    }

    pub fn login_options(&self) -> Result<(String, Value)> {
        let ids = self.db().credential_ids()?;
        if ids.is_empty() {
            return Err(Error::invalid("no credentials enrolled"));
        }
        let challenge = random_bytes::<CHALLENGE_BYTES>();
        let options = json!({
            "challenge": b64url(&challenge),
            "timeout": TIMEOUT_MS,
            "rpId": self.rp_id,
            "allowCredentials": descriptors(&ids),
            "userVerification": "required",
        });
        let cid = token_urlsafe(16);
        self.put_challenge(&login_key(&cid), challenge.to_vec());
        Ok((cid, options))
    }

    pub fn login(&self, cid: &str, credential: &Value, user_agent: &str) -> Result<String> {
        let challenge = self.pop_challenge(&login_key(cid))?;
        let id = credential.get("id").and_then(Value::as_str).unwrap_or("");
        let Some(row) = self.db().credential(id)? else {
            return Err(Error::invalid("unknown credential"));
        };
        let sign_count = verify_assertion(
            credential,
            &challenge,
            &self.rp_id,
            &self.origin,
            &row.public_key,
            row.sign_count,
        )
        .map_err(|e| Error::invalid(format!("login failed: {e}")))?;
        self.db()
            .update_sign_count(&row.id, i64::from(sign_count))?;
        self.create_session(user_agent)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Registered {
    pub credential_id: Vec<u8>,
    pub public_key: Vec<u8>,
    pub sign_count: u32,
    pub fmt: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttestedCredential {
    pub aaguid: [u8; 16],
    pub credential_id: Vec<u8>,
    pub public_key: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuthData {
    pub rp_id_hash: [u8; 32],
    pub flags: u8,
    pub sign_count: u32,
    pub attested: Option<AttestedCredential>,
}

fn cbor_item_len(bytes: &[u8]) -> std::result::Result<(Cbor, usize), String> {
    let mut reader = bytes;
    let value: Cbor = ciborium::de::from_reader(&mut reader)
        .map_err(|e| format!("invalid CBOR in authenticator data: {e}"))?;
    Ok((value, bytes.len() - reader.len()))
}

pub fn parse_auth_data(bytes: &[u8]) -> std::result::Result<AuthData, String> {
    if bytes.len() < 37 {
        return Err("Authenticator data was too short".into());
    }
    let mut rp_id_hash = [0u8; 32];
    rp_id_hash.copy_from_slice(&bytes[..32]);
    let flags = bytes[32];
    let sign_count = u32::from_be_bytes([bytes[33], bytes[34], bytes[35], bytes[36]]);
    let mut rest = &bytes[37..];
    let mut attested = None;
    if flags & FLAG_AT != 0 {
        if rest.len() < 18 {
            return Err("Attested credential data was too short".into());
        }
        let mut aaguid = [0u8; 16];
        aaguid.copy_from_slice(&rest[..16]);
        let id_len = usize::from(u16::from_be_bytes([rest[16], rest[17]]));
        rest = &rest[18..];
        if rest.len() < id_len {
            return Err("Credential ID was longer than the authenticator data".into());
        }
        let credential_id = rest[..id_len].to_vec();
        rest = &rest[id_len..];
        let (_, used) = cbor_item_len(rest)?;
        let public_key = rest[..used].to_vec();
        rest = &rest[used..];
        attested = Some(AttestedCredential {
            aaguid,
            credential_id,
            public_key,
        });
    }
    if flags & FLAG_ED != 0 {
        let (_, used) = cbor_item_len(rest)?;
        rest = &rest[used..];
    }
    if !rest.is_empty() {
        return Err("Leftover bytes detected while parsing authenticator data".into());
    }
    Ok(AuthData {
        rp_id_hash,
        flags,
        sign_count,
        attested,
    })
}

pub enum PublicKey {
    Es256(p256::ecdsa::VerifyingKey),
    Ed25519(ed25519_dalek::VerifyingKey),
    Rs256(rsa::RsaPublicKey),
}

fn cose_get(map: &[(Cbor, Cbor)], key: i64) -> Option<&Cbor> {
    map.iter()
        .find(|(k, _)| k.as_integer().and_then(|i| i64::try_from(i).ok()) == Some(key))
        .map(|(_, v)| v)
}

fn cose_int(map: &[(Cbor, Cbor)], key: i64) -> Option<i64> {
    cose_get(map, key)
        .and_then(Cbor::as_integer)
        .and_then(|i| i64::try_from(i).ok())
}

fn cose_bytes(map: &[(Cbor, Cbor)], key: i64) -> Option<&[u8]> {
    cose_get(map, key)
        .and_then(Cbor::as_bytes)
        .map(Vec::as_slice)
}

impl PublicKey {
    pub fn from_cose(bytes: &[u8]) -> std::result::Result<(i64, PublicKey), String> {
        let value: Cbor = ciborium::de::from_reader(bytes)
            .map_err(|e| format!("Could not decode credential public key: {e}"))?;
        let map = value
            .as_map()
            .ok_or("Credential public key was not a CBOR map")?;
        let kty = cose_int(map, 1).ok_or("Credential public key missing kty")?;
        let alg = cose_int(map, 3).ok_or("Credential public key missing alg")?;
        let key = match (kty, alg) {
            (2, -7) => {
                if cose_int(map, -1) != Some(1) {
                    return Err("Unsupported EC2 curve".into());
                }
                let x = cose_bytes(map, -2).ok_or("EC2 key missing x")?;
                let y = cose_bytes(map, -3).ok_or("EC2 key missing y")?;
                if x.len() != 32 || y.len() != 32 {
                    return Err("EC2 key coordinates have the wrong length".into());
                }
                let point = p256::EncodedPoint::from_affine_coordinates(
                    p256::FieldBytes::from_slice(x),
                    p256::FieldBytes::from_slice(y),
                    false,
                );
                let key = p256::ecdsa::VerifyingKey::from_encoded_point(&point)
                    .map_err(|_| "EC2 key is not a point on P-256")?;
                PublicKey::Es256(key)
            }
            (1, -8) => {
                if cose_int(map, -1) != Some(6) {
                    return Err("Unsupported OKP curve".into());
                }
                let x: [u8; 32] = cose_bytes(map, -2)
                    .ok_or("OKP key missing x")?
                    .try_into()
                    .map_err(|_| "OKP key has the wrong length")?;
                let key = ed25519_dalek::VerifyingKey::from_bytes(&x)
                    .map_err(|_| "OKP key is not a valid Ed25519 key")?;
                PublicKey::Ed25519(key)
            }
            (3, -257) => {
                let n = cose_bytes(map, -1).ok_or("RSA key missing n")?;
                let e = cose_bytes(map, -2).ok_or("RSA key missing e")?;
                let key = rsa::RsaPublicKey::new(
                    rsa::BigUint::from_bytes_be(n),
                    rsa::BigUint::from_bytes_be(e),
                )
                .map_err(|e| format!("Invalid RSA key: {e}"))?;
                PublicKey::Rs256(key)
            }
            _ => return Err(format!("Unsupported public key algorithm {alg}")),
        };
        Ok((alg, key))
    }

    pub fn verify(&self, message: &[u8], signature: &[u8]) -> bool {
        match self {
            PublicKey::Es256(key) => {
                use p256::ecdsa::signature::Verifier;
                p256::ecdsa::Signature::from_der(signature)
                    .is_ok_and(|sig| key.verify(message, &sig).is_ok())
            }
            PublicKey::Ed25519(key) => ed25519_dalek::Signature::from_slice(signature)
                .is_ok_and(|sig| key.verify_strict(message, &sig).is_ok()),
            PublicKey::Rs256(key) => key
                .verify(
                    rsa::Pkcs1v15Sign::new::<Sha256>(),
                    &Sha256::digest(message),
                    signature,
                )
                .is_ok(),
        }
    }
}

fn field<'a>(value: &'a Value, path: &[&str]) -> std::result::Result<&'a Value, String> {
    let mut current = value;
    for key in path {
        current = current
            .get(key)
            .ok_or_else(|| format!("Missing {}", path.join(".")))?;
    }
    Ok(current)
}

fn b64_field(value: &Value, path: &[&str]) -> std::result::Result<Vec<u8>, String> {
    let text = field(value, path)?
        .as_str()
        .ok_or_else(|| format!("{} must be a string", path.join(".")))?;
    b64url_decode(text).ok_or_else(|| format!("{} is not valid base64url", path.join(".")))
}

fn check_credential_shape(credential: &Value) -> std::result::Result<(), String> {
    let id = credential.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() {
        return Err("Missing credential ID".into());
    }
    let raw_id = b64_field(credential, &["rawId"])?;
    if id != b64url(&raw_id) {
        return Err("id and raw_id were not equivalent".into());
    }
    match credential.get("type").and_then(Value::as_str) {
        Some("public-key") => Ok(()),
        other => Err(format!(
            "Unexpected credential type \"{}\", expected \"public-key\"",
            other.unwrap_or("")
        )),
    }
}

fn check_client_data(
    raw: &[u8],
    kind: &str,
    challenge: &[u8],
    origin: &str,
) -> std::result::Result<(), String> {
    let data: Value =
        serde_json::from_slice(raw).map_err(|_| "Client data was not valid JSON".to_string())?;
    let got_kind = data.get("type").and_then(Value::as_str).unwrap_or("");
    if got_kind != kind {
        return Err(format!(
            "Unexpected client data type \"{got_kind}\", expected \"{kind}\""
        ));
    }
    let got_challenge = data
        .get("challenge")
        .and_then(Value::as_str)
        .and_then(b64url_decode)
        .unwrap_or_default();
    if !bool::from(got_challenge.ct_eq(challenge)) {
        return Err("Client data challenge was not expected challenge".into());
    }
    let got_origin = data.get("origin").and_then(Value::as_str).unwrap_or("");
    if got_origin != origin {
        return Err(format!(
            "Unexpected client data origin \"{got_origin}\", expected \"{origin}\""
        ));
    }
    Ok(())
}

fn check_flags(data: &AuthData, rp_id: &str, ceremony: &str) -> std::result::Result<(), String> {
    let expected: [u8; 32] = Sha256::digest(rp_id.as_bytes()).into();
    if !bool::from(data.rp_id_hash.ct_eq(&expected)) {
        return Err("Unexpected RP ID hash".into());
    }
    if data.flags & FLAG_UP == 0 {
        return Err(format!("User was not present during {ceremony}"));
    }
    if data.flags & FLAG_UV == 0 {
        return Err(format!(
            "User verification is required but user was not verified during {ceremony}"
        ));
    }
    Ok(())
}

fn text_key<'a>(map: &'a [(Cbor, Cbor)], key: &str) -> Option<&'a Cbor> {
    map.iter()
        .find(|(k, _)| k.as_text() == Some(key))
        .map(|(_, v)| v)
}

fn certificate_public_key(der: &[u8]) -> Option<p256::ecdsa::VerifyingKey> {
    use p256::pkcs8::DecodePublicKey;
    fn element(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
        let tag = *input.first()?;
        let first = *input.get(1)?;
        let (len, header) = if first < 0x80 {
            (usize::from(first), 2)
        } else {
            let count = usize::from(first & 0x7f);
            if count == 0 || count > 4 {
                return None;
            }
            let mut len = 0usize;
            for i in 0..count {
                len = (len << 8) | usize::from(*input.get(2 + i)?);
            }
            (len, 2 + count)
        };
        let end = header.checked_add(len)?;
        let body = input.get(header..end)?;
        Some((tag, &input[..end], body))
    }
    let (_, _, certificate) = element(der)?;
    let (_, _, mut tbs) = element(certificate)?;
    let mut fields = Vec::new();
    while !tbs.is_empty() {
        let (tag, whole, _) = element(tbs)?;
        fields.push((tag, whole));
        tbs = &tbs[whole.len()..];
    }
    let skip = usize::from(fields.first()?.0 == 0xa0);
    let (_, spki) = fields.get(skip + 5)?;
    p256::ecdsa::VerifyingKey::from_public_key_der(spki).ok()
}

fn check_attestation(
    fmt: &str,
    statement: &[(Cbor, Cbor)],
    auth_data: &[u8],
    client_hash: &[u8],
    alg: i64,
    key: &PublicKey,
) -> std::result::Result<(), String> {
    match fmt {
        "none" => {
            if statement.is_empty() {
                Ok(())
            } else {
                Err("None attestation had unexpected attestation statement".into())
            }
        }
        "packed" => {
            let stated_alg = text_key(statement, "alg")
                .and_then(Cbor::as_integer)
                .and_then(|i| i64::try_from(i).ok())
                .ok_or("Packed attestation missing alg")?;
            let signature = text_key(statement, "sig")
                .and_then(Cbor::as_bytes)
                .ok_or("Packed attestation missing sig")?;
            let mut signed = auth_data.to_vec();
            signed.extend_from_slice(client_hash);
            let valid = match text_key(statement, "x5c").and_then(Cbor::as_array) {
                Some(chain) => {
                    if stated_alg != -7 {
                        return Err(format!(
                            "Unsupported packed attestation certificate algorithm {stated_alg}"
                        ));
                    }
                    let leaf = chain
                        .first()
                        .and_then(Cbor::as_bytes)
                        .and_then(|der| certificate_public_key(der))
                        .ok_or("Could not read the attestation certificate")?;
                    PublicKey::Es256(leaf).verify(&signed, signature)
                }
                None => {
                    if stated_alg != alg {
                        return Err("Attestation statement alg did not match public key alg".into());
                    }
                    key.verify(&signed, signature)
                }
            };
            if valid {
                Ok(())
            } else {
                Err("Could not verify attestation statement signature".into())
            }
        }
        other => Err(format!("Unsupported attestation format \"{other}\"")),
    }
}

pub fn verify_registration(
    credential: &Value,
    challenge: &[u8],
    rp_id: &str,
    origin: &str,
) -> std::result::Result<Registered, String> {
    check_credential_shape(credential)?;
    let client_data = b64_field(credential, &["response", "clientDataJSON"])?;
    check_client_data(&client_data, "webauthn.create", challenge, origin)?;
    let attestation = b64_field(credential, &["response", "attestationObject"])?;
    let object: Cbor = ciborium::de::from_reader(attestation.as_slice())
        .map_err(|_| "Could not decode attestation object".to_string())?;
    let object = object
        .as_map()
        .ok_or("Attestation object was not a CBOR map")?;
    let fmt = text_key(object, "fmt")
        .and_then(Cbor::as_text)
        .ok_or("Attestation object missing fmt")?;
    let statement = text_key(object, "attStmt")
        .and_then(Cbor::as_map)
        .ok_or("Attestation object missing attStmt")?;
    let raw_auth_data = text_key(object, "authData")
        .and_then(Cbor::as_bytes)
        .ok_or("Attestation object missing authData")?;
    let auth_data = parse_auth_data(raw_auth_data)?;
    check_flags(&auth_data, rp_id, "attestation")?;
    let attested = auth_data
        .attested
        .as_ref()
        .ok_or("Authenticator did not provide attested credential data")?;
    let (alg, key) = PublicKey::from_cose(&attested.public_key)?;
    let client_hash = Sha256::digest(&client_data);
    check_attestation(fmt, statement, raw_auth_data, &client_hash, alg, &key)?;
    Ok(Registered {
        credential_id: attested.credential_id.clone(),
        public_key: attested.public_key.clone(),
        sign_count: auth_data.sign_count,
        fmt: fmt.to_string(),
    })
}

pub fn verify_assertion(
    credential: &Value,
    challenge: &[u8],
    rp_id: &str,
    origin: &str,
    public_key: &[u8],
    current_sign_count: i64,
) -> std::result::Result<u32, String> {
    check_credential_shape(credential)?;
    let client_data = b64_field(credential, &["response", "clientDataJSON"])?;
    check_client_data(&client_data, "webauthn.get", challenge, origin)?;
    let raw_auth_data = b64_field(credential, &["response", "authenticatorData"])?;
    let signature = b64_field(credential, &["response", "signature"])?;
    let auth_data = parse_auth_data(&raw_auth_data)?;
    check_flags(&auth_data, rp_id, "authentication")?;
    let new_count = i64::from(auth_data.sign_count);
    if (new_count > 0 || current_sign_count > 0) && new_count <= current_sign_count {
        return Err(format!(
            "Response sign count of {new_count} was not greater than current count of {current_sign_count}"
        ));
    }
    let (_, key) = PublicKey::from_cose(public_key)?;
    let mut signed = raw_auth_data.clone();
    signed.extend_from_slice(&Sha256::digest(&client_data));
    if !key.verify(&signed, &signature) {
        return Err("Could not verify authentication signature".into());
    }
    Ok(auth_data.sign_count)
}

#[cfg(test)]
pub mod soft {
    use super::*;
    use p256::ecdsa::SigningKey;
    use p256::ecdsa::signature::Signer;

    pub struct SoftAuthenticator {
        pub rp_hash: [u8; 32],
        pub origin: String,
        pub key: SigningKey,
        pub cred_id: Vec<u8>,
        pub counter: u32,
    }

    fn cbor(value: &Cbor) -> Vec<u8> {
        let mut out = Vec::new();
        ciborium::ser::into_writer(value, &mut out).unwrap();
        out
    }

    pub fn cose_es256(key: &p256::ecdsa::VerifyingKey) -> Vec<u8> {
        let point = key.to_encoded_point(false);
        cbor(&Cbor::Map(vec![
            (Cbor::from(1), Cbor::from(2)),
            (Cbor::from(3), Cbor::from(-7)),
            (Cbor::from(-1), Cbor::from(1)),
            (Cbor::from(-2), Cbor::Bytes(point.x().unwrap().to_vec())),
            (Cbor::from(-3), Cbor::Bytes(point.y().unwrap().to_vec())),
        ]))
    }

    impl SoftAuthenticator {
        pub fn new(rp_id: &str, origin: &str) -> SoftAuthenticator {
            Self::with_key(
                rp_id,
                origin,
                SigningKey::from_slice(&random_bytes::<32>()).unwrap(),
            )
        }

        pub fn with_key(rp_id: &str, origin: &str, key: SigningKey) -> SoftAuthenticator {
            SoftAuthenticator {
                rp_hash: Sha256::digest(rp_id.as_bytes()).into(),
                origin: origin.to_string(),
                key,
                cred_id: random_bytes::<16>().to_vec(),
                counter: 0,
            }
        }

        pub fn client_data(&self, kind: &str, challenge: &str) -> Vec<u8> {
            serde_json::to_vec(
                &json!({"type": kind, "challenge": challenge, "origin": self.origin}),
            )
            .unwrap()
        }

        pub fn create_with(&self, options: &Value, flags: u8, fmt: &str, statement: Cbor) -> Value {
            let mut auth_data = self.rp_hash.to_vec();
            auth_data.push(flags);
            auth_data.extend_from_slice(&self.counter.to_be_bytes());
            auth_data.extend_from_slice(&[0u8; 16]);
            auth_data.extend_from_slice(&(self.cred_id.len() as u16).to_be_bytes());
            auth_data.extend_from_slice(&self.cred_id);
            auth_data.extend_from_slice(&cose_es256(self.key.verifying_key()));
            let client =
                self.client_data("webauthn.create", options["challenge"].as_str().unwrap());
            let statement = match statement {
                Cbor::Null => {
                    let mut signed = auth_data.clone();
                    signed.extend_from_slice(&Sha256::digest(&client));
                    let sig: p256::ecdsa::Signature = self.key.sign(&signed);
                    Cbor::Map(vec![
                        (Cbor::from("alg"), Cbor::from(-7)),
                        (
                            Cbor::from("sig"),
                            Cbor::Bytes(sig.to_der().as_bytes().to_vec()),
                        ),
                    ])
                }
                other => other,
            };
            let attestation = cbor(&Cbor::Map(vec![
                (Cbor::from("fmt"), Cbor::from(fmt)),
                (Cbor::from("attStmt"), statement),
                (Cbor::from("authData"), Cbor::Bytes(auth_data)),
            ]));
            json!({
                "id": b64url(&self.cred_id),
                "rawId": b64url(&self.cred_id),
                "type": "public-key",
                "response": {"clientDataJSON": b64url(&client), "attestationObject": b64url(&attestation)},
                "clientExtensionResults": {},
            })
        }

        pub fn create(&self, options: &Value) -> Value {
            self.create_with(options, 0x45, "none", Cbor::Map(vec![]))
        }

        pub fn get_with(&mut self, options: &Value, flags: u8, corrupt: bool) -> Value {
            self.counter += 1;
            let client = self.client_data("webauthn.get", options["challenge"].as_str().unwrap());
            let mut auth_data = self.rp_hash.to_vec();
            auth_data.push(flags);
            auth_data.extend_from_slice(&self.counter.to_be_bytes());
            let mut signed = auth_data.clone();
            signed.extend_from_slice(&Sha256::digest(&client));
            let sig: p256::ecdsa::Signature = self.key.sign(&signed);
            let mut signature = sig.to_der().as_bytes().to_vec();
            if corrupt {
                let last = signature.len() - 1;
                signature[last] ^= 1;
            }
            json!({
                "id": b64url(&self.cred_id),
                "rawId": b64url(&self.cred_id),
                "type": "public-key",
                "response": {
                    "clientDataJSON": b64url(&client),
                    "authenticatorData": b64url(&auth_data),
                    "signature": b64url(&signature),
                    "userHandle": b64url(USER_ID),
                },
                "clientExtensionResults": {},
            })
        }

        pub fn get(&mut self, options: &Value) -> Value {
            self.get_with(options, 0x05, false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::soft::SoftAuthenticator;
    use super::*;
    use crate::auth::tests::auth;

    const ORIGIN: &str = "http://localhost:8180";

    fn enrolled(auth: &Auth) -> SoftAuthenticator {
        let device = SoftAuthenticator::new("localhost", ORIGIN);
        let code = auth.new_enroll_code().unwrap();
        let options = auth.registration_options(&code).unwrap();
        auth.register(&code, &device.create(&options), "phone", "ua")
            .unwrap();
        device
    }

    #[test]
    fn registration_options_match_py_webauthn() {
        let (_d, auth) = auth();
        let code = auth.new_enroll_code().unwrap();
        let options = auth.registration_options(&code).unwrap();
        let keys: Vec<&str> = options
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "rp",
                "user",
                "challenge",
                "pubKeyCredParams",
                "timeout",
                "excludeCredentials",
                "authenticatorSelection",
                "attestation"
            ]
        );
        assert_eq!(options["user"]["id"], "b3duZXI");
        assert_eq!(
            b64url_decode(options["challenge"].as_str().unwrap())
                .unwrap()
                .len(),
            64
        );
        assert_eq!(options["excludeCredentials"], json!([]));
        assert_eq!(
            auth.registration_options("deadbeef").unwrap_err().message(),
            "invalid or expired code"
        );
    }

    #[test]
    fn register_and_login_with_a_soft_authenticator() {
        let (_d, auth) = auth();
        let mut device = enrolled(&auth);
        let stored = auth.credentials().unwrap();
        assert_eq!(stored[0].name, "phone");
        assert_eq!(stored[0].id, b64url(&device.cred_id));
        let (cid, options) = auth.login_options().unwrap();
        assert_eq!(options["allowCredentials"][0]["id"], stored[0].id);
        let assertion = device.get(&options);
        let token = auth.login(&cid, &assertion, "ua").unwrap();
        assert!(auth.check_session(Some(&token)).unwrap().is_some());
        assert_eq!(
            auth.login(&cid, &assertion, "ua").unwrap_err().message(),
            "challenge expired"
        );
        assert_eq!(
            auth.db()
                .credential(&stored[0].id)
                .unwrap()
                .unwrap()
                .sign_count,
            1
        );
    }

    #[test]
    fn bad_assertions_are_rejected() {
        let (_d, auth) = auth();
        let mut device = enrolled(&auth);
        type Make = Box<dyn Fn(&mut SoftAuthenticator, &Value) -> Value>;
        let cases: Vec<(Make, &str)> = vec![
            (Box::new(|d, o| d.get_with(o, 0x05, true)), "signature"),
            (Box::new(|d, o| d.get_with(o, 0x01, false)), "verification"),
            (Box::new(|d, o| d.get_with(o, 0x04, false)), "not present"),
            (
                Box::new(|d, o| {
                    let mut c = d.get(o);
                    c["response"]["clientDataJSON"] =
                        json!(b64url(&d.client_data("webauthn.get", "AAAA")));
                    c
                }),
                "challenge",
            ),
            (
                Box::new(|d, o| {
                    let mut c = d.get(o);
                    c["rawId"] = json!("AAAA");
                    c
                }),
                "equivalent",
            ),
            (
                Box::new(|d, o| {
                    d.counter = 0;
                    d.get(o)
                }),
                "sign count",
            ),
        ];
        let (cid, options) = auth.login_options().unwrap();
        auth.login(&cid, &device.get(&options), "ua").unwrap();
        for (make, expected) in cases {
            let (cid, options) = auth.login_options().unwrap();
            let credential = make(&mut device, &options);
            let err = auth.login(&cid, &credential, "ua").unwrap_err();
            assert!(
                err.message().contains(expected),
                "{} vs {expected}",
                err.message()
            );
        }
        let (cid, options) = auth.login_options().unwrap();
        let mut stranger = SoftAuthenticator::new("localhost", ORIGIN);
        assert_eq!(
            auth.login(&cid, &stranger.get(&options), "ua")
                .unwrap_err()
                .message(),
            "unknown credential"
        );
    }

    #[test]
    fn wrong_origin_or_rp_and_garbage_are_registration_errors() {
        let (_d, auth) = auth();
        for device in [
            SoftAuthenticator::new("localhost", "https://evil.example"),
            SoftAuthenticator::new("evil.example", ORIGIN),
        ] {
            let code = auth.new_enroll_code().unwrap();
            let options = auth.registration_options(&code).unwrap();
            let err = auth
                .register(&code, &device.create(&options), "", "ua")
                .unwrap_err();
            assert!(err.message().starts_with("registration failed: "), "{err}");
        }
        for garbage in [
            json!(null),
            json!({}),
            json!({"id": "x", "rawId": "x"}),
            json!([1, 2]),
            json!({"id": "AAAA", "rawId": "AAAA", "type": "public-key", "response": {"clientDataJSON": "!!", "attestationObject": 5}}),
        ] {
            let code = auth.new_enroll_code().unwrap();
            auth.registration_options(&code).unwrap();
            assert!(
                auth.register(&code, &garbage, "", "ua")
                    .unwrap_err()
                    .is_invalid()
            );
        }
        assert!(!auth.has_credentials().unwrap());
    }

    #[test]
    fn the_same_passkey_twice_is_a_clean_error() {
        let (_d, auth) = auth();
        let device = enrolled(&auth);
        let code = auth.new_enroll_code().unwrap();
        let options = auth.registration_options(&code).unwrap();
        assert_eq!(
            options["excludeCredentials"][0]["id"],
            b64url(&device.cred_id)
        );
        let err = auth
            .register(&code, &device.create(&options), "", "ua")
            .unwrap_err();
        assert!(err.is_invalid() && err.message().contains("already registered"));
        auth.check_code(&code).unwrap();
    }

    #[test]
    fn packed_self_attestation_is_verified() {
        let (_d, auth) = auth();
        let device = SoftAuthenticator::new("localhost", ORIGIN);
        let code = auth.new_enroll_code().unwrap();
        let options = auth.registration_options(&code).unwrap();
        let bad = device.create_with(
            &options,
            0x45,
            "packed",
            Cbor::Map(vec![
                (Cbor::from("alg"), Cbor::from(-7)),
                (Cbor::from("sig"), Cbor::Bytes(vec![0x30, 0x00])),
            ]),
        );
        assert!(
            auth.register(&code, &bad, "", "ua")
                .unwrap_err()
                .message()
                .contains("signature")
        );
        let options = auth.registration_options(&code).unwrap();
        let good = device.create_with(&options, 0x45, "packed", Cbor::Null);
        auth.register(&code, &good, "", "ua").unwrap();
        assert_eq!(auth.credentials().unwrap()[0].name, DEFAULT_DEVICE_NAME);
        let code = auth.new_enroll_code().unwrap();
        let options = auth.registration_options(&code).unwrap();
        let other = SoftAuthenticator::new("localhost", ORIGIN);
        let tpm = other.create_with(&options, 0x45, "tpm", Cbor::Map(vec![]));
        assert!(
            auth.register(&code, &tpm, "", "ua")
                .unwrap_err()
                .message()
                .contains("Unsupported")
        );
    }

    #[test]
    fn ed25519_and_rsa_keys_verify() {
        use ed25519_dalek::Signer;
        let secret = ed25519_dalek::SigningKey::from_bytes(&random_bytes::<32>());
        let mut out = Vec::new();
        ciborium::ser::into_writer(
            &Cbor::Map(vec![
                (Cbor::from(1), Cbor::from(1)),
                (Cbor::from(3), Cbor::from(-8)),
                (Cbor::from(-1), Cbor::from(6)),
                (
                    Cbor::from(-2),
                    Cbor::Bytes(secret.verifying_key().to_bytes().to_vec()),
                ),
            ]),
            &mut out,
        )
        .unwrap();
        let (alg, key) = PublicKey::from_cose(&out).unwrap();
        assert_eq!(alg, -8);
        let sig = secret.sign(b"hello");
        assert!(key.verify(b"hello", &sig.to_bytes()));
        assert!(!key.verify(b"hellp", &sig.to_bytes()));

        use rsa::traits::PublicKeyParts;
        let private = rsa::RsaPrivateKey::new(&mut RandomRng, 1024).unwrap();
        let public = private.to_public_key();
        let mut out = Vec::new();
        ciborium::ser::into_writer(
            &Cbor::Map(vec![
                (Cbor::from(1), Cbor::from(3)),
                (Cbor::from(3), Cbor::from(-257)),
                (Cbor::from(-1), Cbor::Bytes(public.n().to_bytes_be())),
                (Cbor::from(-2), Cbor::Bytes(public.e().to_bytes_be())),
            ]),
            &mut out,
        )
        .unwrap();
        let (alg, key) = PublicKey::from_cose(&out).unwrap();
        assert_eq!(alg, -257);
        let sig = private
            .sign(
                rsa::Pkcs1v15Sign::new::<Sha256>(),
                &Sha256::digest(b"hello"),
            )
            .unwrap();
        assert!(key.verify(b"hello", &sig));
        assert!(!key.verify(b"other", &sig));
        assert!(PublicKey::from_cose(b"\xa1\x01\x04").is_err());
        assert!(PublicKey::from_cose(b"").is_err());
    }

    struct RandomRng;

    impl rsa::rand_core::RngCore for RandomRng {
        fn next_u32(&mut self) -> u32 {
            u32::from_le_bytes(random_bytes::<4>())
        }
        fn next_u64(&mut self) -> u64 {
            u64::from_le_bytes(random_bytes::<8>())
        }
        fn fill_bytes(&mut self, dest: &mut [u8]) {
            getrandom::fill(dest).unwrap();
        }
        fn try_fill_bytes(
            &mut self,
            dest: &mut [u8],
        ) -> std::result::Result<(), rsa::rand_core::Error> {
            self.fill_bytes(dest);
            Ok(())
        }
    }

    impl rsa::rand_core::CryptoRng for RandomRng {}

    #[test]
    fn auth_data_parser_survives_garbage() {
        for len in 0..80 {
            let mut bytes = vec![0xffu8; len];
            if len > 32 {
                bytes[32] = 0xc5;
            }
            let _ = parse_auth_data(&bytes);
        }
        assert!(parse_auth_data(&[0u8; 37]).is_ok());
        assert!(parse_auth_data(&[0u8; 38]).is_err());
    }
}
