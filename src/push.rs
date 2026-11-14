use std::path::{Path, PathBuf};
use std::sync::Arc;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes128Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hkdf::Hkdf;
use p256::ecdsa::SigningKey;
use p256::ecdsa::signature::Signer;
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey, LineEnding};
use p256::{PublicKey, SecretKey};
use serde_json::{Value, json};
use sha2::Sha256;

use crate::auth::random_bytes;
use crate::clock;
use crate::db::Database;
use crate::error::{Error, Result};
use crate::notify::push_agent;

pub const WEB_PUSH_TTL_S: u64 = 3600;
pub const GONE: [u16; 2] = [404, 410];
pub const VAPID_EXPIRY_S: u64 = 12 * 3600;
pub const RECORD_SIZE: u32 = 4096;

pub struct WebPush {
    pub db: Arc<Database>,
    pub key_path: PathBuf,
    pub subject: String,
    pub agent: &'static ureq::Agent,
    key: SecretKey,
}

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn b64_decode(text: &str) -> Option<Vec<u8>> {
    crate::auth::webauthn::b64url_decode(text)
}

pub fn random_secret() -> SecretKey {
    loop {
        if let Ok(key) = SecretKey::from_slice(&random_bytes::<32>()) {
            return key;
        }
    }
}

pub fn load_key(path: &Path) -> Result<SecretKey> {
    let text = std::fs::read_to_string(path)?;
    SecretKey::from_pkcs8_pem(&text)
        .or_else(|_| SecretKey::from_sec1_pem(&text))
        .map_err(|e| Error::internal(format!("cannot read {}: {e}", path.display())))
}

fn create_key(path: &Path) -> Result<SecretKey> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let key = random_secret();
    let pem = key
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|e| Error::internal(format!("cannot encode the VAPID key: {e}")))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(pem.as_bytes())?;
    Ok(key)
}

pub fn audience(endpoint: &str) -> Option<String> {
    let (scheme, rest) = endpoint.split_once("://")?;
    let netloc = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = netloc.rsplit_once('@').map_or(netloc, |(_, host)| host);
    if host.is_empty() {
        return None;
    }
    Some(format!("{}://{host}", scheme.to_ascii_lowercase()))
}

pub fn vapid_jwt(key: &SecretKey, audience: &str, subject: &str, now: f64) -> String {
    let header = b64(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = json!({"aud": audience, "exp": now as u64 + VAPID_EXPIRY_S, "sub": subject});
    let claims = b64(claims.to_string().as_bytes());
    let signing_input = format!("{header}.{claims}");
    let signature: p256::ecdsa::Signature = SigningKey::from(key).sign(signing_input.as_bytes());
    format!("{signing_input}.{}", b64(&signature.to_bytes()))
}

fn hkdf(salt: &[u8], ikm: &[u8], info: &[u8], out: &mut [u8]) {
    Hkdf::<Sha256>::new(Some(salt), ikm)
        .expand(info, out)
        .expect("hkdf output length");
}

pub fn encrypt_with(
    sender: &SecretKey,
    salt: [u8; 16],
    receiver: &[u8],
    auth_secret: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    let receiver_key =
        PublicKey::from_sec1_bytes(receiver).map_err(|_| Error::invalid("invalid p256dh key"))?;
    let receiver_raw = receiver_key.to_sec1_bytes();
    let sender_raw = sender.public_key().to_sec1_bytes();
    let shared = p256::ecdh::diffie_hellman(sender.to_nonzero_scalar(), receiver_key.as_affine());
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(&receiver_raw);
    key_info.extend_from_slice(&sender_raw);
    let mut ikm = [0u8; 32];
    hkdf(auth_secret, shared.raw_secret_bytes(), &key_info, &mut ikm);
    let mut cek = [0u8; 16];
    let mut nonce = [0u8; 12];
    hkdf(&salt, &ikm, b"Content-Encoding: aes128gcm\0", &mut cek);
    hkdf(&salt, &ikm, b"Content-Encoding: nonce\0", &mut nonce);
    if plaintext.len() + 17 > RECORD_SIZE as usize {
        return Err(Error::invalid("push payload too large"));
    }
    let mut record = plaintext.to_vec();
    record.push(2);
    let cipher = Aes128Gcm::new_from_slice(&cek).expect("aes key length");
    let sealed = cipher
        .encrypt(&Nonce::from(nonce), record.as_slice())
        .map_err(|_| Error::internal("encryption failed"))?;
    let mut out = salt.to_vec();
    out.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    out.push(sender_raw.len() as u8);
    out.extend_from_slice(&sender_raw);
    out.extend_from_slice(&sealed);
    Ok(out)
}

pub fn encrypt(receiver: &[u8], auth_secret: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
    encrypt_with(
        &random_secret(),
        random_bytes::<16>(),
        receiver,
        auth_secret,
        plaintext,
    )
}

pub struct PushRequest {
    pub endpoint: String,
    pub authorization: String,
    pub body: Vec<u8>,
}

pub enum Outcome {
    Sent,
    Gone,
    Failed(String),
}

impl WebPush {
    pub fn open(db: Arc<Database>, key_path: &Path, subject: &str) -> Result<WebPush> {
        let key = if key_path.exists() {
            load_key(key_path)?
        } else {
            create_key(key_path)?
        };
        Ok(WebPush {
            db,
            key_path: key_path.to_path_buf(),
            subject: subject.to_string(),
            agent: push_agent(),
            key,
        })
    }

    pub fn public_key(&self) -> String {
        b64(&self.key.public_key().to_sec1_bytes())
    }

    pub fn count(&self) -> i64 {
        self.db.push_count().unwrap_or(0)
    }

    pub fn subscribe(&self, subscription: &Value) -> Result<()> {
        let endpoint = subscription
            .get("endpoint")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::invalid("invalid subscription"))?;
        self.db
            .upsert_push_sub(endpoint, &serde_json::to_string(subscription)?)
    }

    pub fn unsubscribe(&self, endpoint: &str) -> Result<()> {
        self.db.delete_push_sub(endpoint)
    }

    pub fn request(&self, subscription: &Value, payload: &[u8]) -> Result<PushRequest> {
        let endpoint = subscription
            .get("endpoint")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::invalid("subscription without endpoint"))?;
        let keys = subscription.get("keys");
        let field = |name: &str| {
            keys.and_then(|k| k.get(name))
                .and_then(Value::as_str)
                .and_then(b64_decode)
                .ok_or_else(|| Error::invalid(format!("subscription without a valid {name} key")))
        };
        let receiver = field("p256dh")?;
        let auth_secret = field("auth")?;
        let audience = audience(endpoint).ok_or_else(|| Error::invalid("invalid endpoint"))?;
        let jwt = vapid_jwt(&self.key, &audience, &self.subject, clock::now());
        Ok(PushRequest {
            endpoint: endpoint.to_string(),
            authorization: format!("vapid t={jwt},k={}", self.public_key()),
            body: encrypt(&receiver, &auth_secret, payload)?,
        })
    }

    pub fn deliver(agent: &ureq::Agent, request: &PushRequest) -> Outcome {
        let answer = agent
            .post(&request.endpoint)
            .header("Authorization", &request.authorization)
            .header("TTL", WEB_PUSH_TTL_S.to_string())
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            .send(request.body.as_slice());
        match answer {
            Ok(response) => {
                let status = response.status().as_u16();
                if (200..300).contains(&status) {
                    Outcome::Sent
                } else if GONE.contains(&status) {
                    Outcome::Gone
                } else {
                    let reason = response.status().canonical_reason().unwrap_or("");
                    Outcome::Failed(format!("Push failed: {status} {reason}"))
                }
            }
            Err(e) => Outcome::Failed(e.to_string()),
        }
    }

    pub fn send_with(
        &self,
        title: &str,
        body: &str,
        url: &str,
        tag: &str,
        deliver: &dyn Fn(&PushRequest) -> Outcome,
    ) -> usize {
        let payload = json!({"title": title, "body": body, "url": url, "tag": tag}).to_string();
        let subscriptions = match self.db.push_subs() {
            Ok(subs) => subs,
            Err(e) => {
                crate::error!("camd.push", "web push error: {}", e);
                return 0;
            }
        };
        let mut sent = 0;
        for (endpoint, data) in subscriptions {
            let request = serde_json::from_str::<Value>(&data)
                .map_err(Error::from)
                .and_then(|sub| self.request(&sub, payload.as_bytes()));
            let request = match request {
                Ok(request) => request,
                Err(e) => {
                    crate::error!("camd.push", "web push error: {}", e);
                    continue;
                }
            };
            match deliver(&request) {
                Outcome::Sent => sent += 1,
                Outcome::Gone => {
                    if let Err(e) = self.unsubscribe(&endpoint) {
                        crate::error!("camd.push", "web push error: {}", e);
                    }
                }
                Outcome::Failed(reason) => {
                    crate::warning!("camd.push", "web push failed: {}", reason)
                }
            }
        }
        sent
    }

    pub fn send_all(&self, title: &str, body: &str, url: &str, tag: &str) -> usize {
        self.send_with(title, body, url, tag, &|request| {
            WebPush::deliver(self.agent, request)
        })
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::Mutex;

    fn decode(text: &str) -> Vec<u8> {
        b64_decode(text).unwrap()
    }

    pub fn decrypt(receiver: &SecretKey, auth_secret: &[u8], body: &[u8]) -> Vec<u8> {
        let salt = &body[..16];
        let rs = u32::from_be_bytes(body[16..20].try_into().unwrap());
        assert_eq!(rs, RECORD_SIZE);
        let id_len = body[20] as usize;
        let sender = PublicKey::from_sec1_bytes(&body[21..21 + id_len]).unwrap();
        let shared = p256::ecdh::diffie_hellman(receiver.to_nonzero_scalar(), sender.as_affine());
        let mut info = b"WebPush: info\0".to_vec();
        info.extend_from_slice(&receiver.public_key().to_sec1_bytes());
        info.extend_from_slice(&sender.to_sec1_bytes());
        let mut ikm = [0u8; 32];
        hkdf(auth_secret, shared.raw_secret_bytes(), &info, &mut ikm);
        let mut cek = [0u8; 16];
        let mut nonce = [0u8; 12];
        hkdf(salt, &ikm, b"Content-Encoding: aes128gcm\0", &mut cek);
        hkdf(salt, &ikm, b"Content-Encoding: nonce\0", &mut nonce);
        let mut plain = Aes128Gcm::new_from_slice(&cek)
            .unwrap()
            .decrypt(&Nonce::from(nonce), &body[21 + id_len..])
            .unwrap();
        assert_eq!(plain.pop(), Some(2));
        plain
    }

    #[test]
    fn rfc8291_example_is_reproduced() {
        let sender =
            SecretKey::from_slice(&decode("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")).unwrap();
        let receiver = decode(
            "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
        );
        let salt: [u8; 16] = decode("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let body = encrypt_with(
            &sender,
            salt,
            &receiver,
            &decode("BTBZMqHH6r4Tts7J_aSIgg"),
            b"When I grow up, I want to be a watermelon",
        )
        .unwrap();
        assert_eq!(
            b64(&body),
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
        );
    }

    fn web(dir: &Path) -> WebPush {
        let db = Arc::new(Database::open(&dir.join("camd.sqlite3")).unwrap());
        WebPush::open(db, &dir.join("vapid_private.pem"), "mailto:me@example.org").unwrap()
    }

    #[test]
    fn the_key_is_created_once_with_mode_600_and_reused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let first = web(dir.path()).public_key();
        let path = dir.path().join("vapid_private.pem");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .starts_with("-----BEGIN PRIVATE KEY-----")
        );
        assert_eq!(web(dir.path()).public_key(), first);
        assert_eq!(decode(&first).len(), 65);
    }

    #[test]
    fn a_py_vapid_key_is_reused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("vapid_private.pem"),
            "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgc/w7lkAoJTO6Y7da\njEfxPKH78bDUUa79x9+GKQ6H63mhRANCAARJ375g+ZFTjGFHB9E4VNmGjb92wmqQ\nc5iDCVuZn73BDnJICezk/Tw/tZCNNULbctMCydID01L8R9U0TH3z56au\n-----END PRIVATE KEY-----\n",
        )
        .unwrap();
        let push = web(dir.path());
        assert_eq!(
            push.public_key(),
            "BEnfvmD5kVOMYUcH0ThU2YaNv3bCapBzmIMJW5mfvcEOckgJ7OT9PD-1kI01Qtty0wLJ0gPTUvxH1TRMffPnpq4"
        );
    }

    #[test]
    fn delivery_encrypts_signs_and_drops_gone_subscriptions() {
        let dir = tempfile::tempdir().unwrap();
        let push = web(dir.path());
        let browser = random_secret();
        let auth_secret = random_bytes::<16>();
        for (endpoint, _) in [
            ("https://push.example.org/ok", 0),
            ("https://push.example.org/gone", 1),
            ("https://push.example.org/err", 2),
        ] {
            push.subscribe(&json!({"endpoint": endpoint, "keys": {"p256dh": b64(&browser.public_key().to_sec1_bytes()), "auth": b64(&auth_secret)}}))
                .unwrap();
        }
        push.subscribe(&json!({"endpoint": "https://push.example.org/broken", "keys": {}}))
            .unwrap();
        let seen = Mutex::new(Vec::new());
        let sent = push.send_with("T", "B", "/#/events/1", "motion", &|request| {
            let plain = decrypt(&browser, &auth_secret, &request.body);
            seen.lock().unwrap().push((
                request.endpoint.clone(),
                plain,
                request.authorization.clone(),
            ));
            match request.endpoint.rsplit('/').next() {
                Some("ok") => Outcome::Sent,
                Some("gone") => Outcome::Gone,
                _ => Outcome::Failed("Push failed: 500".into()),
            }
        });
        assert_eq!(sent, 1);
        assert_eq!(push.count(), 3);
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen.len(), 3);
        let payload: Value = serde_json::from_slice(&seen[0].1).unwrap();
        assert_eq!(
            payload,
            json!({"title": "T", "body": "B", "url": "/#/events/1", "tag": "motion"})
        );
        let auth = &seen[0].2;
        assert!(
            auth.starts_with("vapid t=") && auth.ends_with(&format!(",k={}", push.public_key()))
        );
        let jwt = auth
            .trim_start_matches("vapid t=")
            .split(',')
            .next()
            .unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        let claims: Value = serde_json::from_slice(&decode(parts[1])).unwrap();
        assert_eq!(claims["aud"], "https://push.example.org");
        assert_eq!(claims["sub"], "mailto:me@example.org");
        let expiry = claims["exp"].as_f64().unwrap() - clock::now();
        assert!((VAPID_EXPIRY_S as f64 - 5.0..=VAPID_EXPIRY_S as f64 + 1.0).contains(&expiry));
        use p256::ecdsa::signature::Verifier;
        let signature = p256::ecdsa::Signature::from_slice(&decode(parts[2])).unwrap();
        p256::ecdsa::VerifyingKey::from(&push.key.public_key())
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
            .unwrap();
    }

    #[test]
    fn audience_is_scheme_and_netloc() {
        assert_eq!(
            audience("https://fcm.googleapis.com/fcm/send/x").as_deref(),
            Some("https://fcm.googleapis.com")
        );
        assert_eq!(
            audience("https://h:8443?x").as_deref(),
            Some("https://h:8443")
        );
        assert_eq!(audience("nope"), None);
        assert_eq!(
            audience("HTTPS://user:secret@web.push.apple.com:443/x").as_deref(),
            Some("https://web.push.apple.com:443")
        );
        assert_eq!(audience("https://user@/x"), None);
    }
}
