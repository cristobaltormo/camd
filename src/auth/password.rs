use scrypt::Params;
use subtle::ConstantTimeEq;

pub const SCRYPT_LOG_N: u8 = 14;
pub const SCRYPT_R: u32 = 8;
pub const SCRYPT_P: u32 = 1;
pub const SCRYPT_LEN: usize = 32;
pub const SALT_LEN: usize = 16;
const DUMMY_SALT: [u8; SALT_LEN] = [0; SALT_LEN];

fn derive(password: &str, salt: &[u8]) -> Vec<u8> {
    let params =
        Params::new(SCRYPT_LOG_N, SCRYPT_R, SCRYPT_P, SCRYPT_LEN).expect("valid scrypt parameters");
    let mut out = vec![0u8; SCRYPT_LEN];
    scrypt::scrypt(password.as_bytes(), salt, &params, &mut out).expect("scrypt output length");
    out
}

pub fn hash_password(password: &str, salt: Option<[u8; SALT_LEN]>) -> String {
    let salt = salt.unwrap_or_else(|| {
        let mut fresh = [0u8; SALT_LEN];
        getrandom::fill(&mut fresh).expect("system randomness");
        fresh
    });
    format!(
        "scrypt${}${}",
        hex::encode(salt),
        hex::encode(derive(password, &salt))
    )
}

fn parse(stored: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let parts: Vec<&str> = stored.split('$').collect();
    let [_, salt_hex, digest_hex] = parts.as_slice() else {
        return None;
    };
    Some((hex::decode(salt_hex).ok()?, hex::decode(digest_hex).ok()?))
}

pub fn check_password(password: &str, stored: Option<&str>) -> bool {
    let (salt, expected) =
        parse(stored.unwrap_or("")).unwrap_or_else(|| (DUMMY_SALT.to_vec(), Vec::new()));
    let actual = derive(password, &salt);
    !expected.is_empty() && bool::from(actual.ct_eq(&expected))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_rules_and_hashing() {
        let stored = hash_password("long enough password", None);
        assert!(check_password("long enough password", Some(&stored)));
        assert!(!check_password("another one entirely", Some(&stored)));
        assert!(!check_password("anything", None));
        assert_ne!(hash_password("long enough password", None), stored);
    }

    #[test]
    fn the_stored_format_is_the_python_one() {
        let stored = hash_password("correct horse battery", Some([7; SALT_LEN]));
        let parts: Vec<&str> = stored.split('$').collect();
        assert_eq!(parts[0], "scrypt");
        assert_eq!(parts[1], "07".repeat(16));
        assert_eq!(parts[2].len(), 64);
    }

    #[test]
    fn hashes_written_by_python_verify_and_are_reproduced() {
        let ascii = "scrypt$000102030405060708090a0b0c0d0e0f$e96ec6ba8c63a23698220f37c7fc042925d430c7feadec9ceb04af03cabb9f2f";
        let salt: [u8; SALT_LEN] = std::array::from_fn(|i| i as u8);
        assert_eq!(hash_password("correct horse battery", Some(salt)), ascii);
        assert!(check_password("correct horse battery", Some(ascii)));
        let unicode = "scrypt$ffffffffffffffffffffffffffffffff$9ce4d3b1e4f09d48e7607174fb0b296d30c47052f828048a604eebbdb693999f";
        assert!(check_password(
            "contrase\u{f1}a larga \u{f1}",
            Some(unicode)
        ));
    }

    #[test]
    fn malformed_hashes_never_match() {
        for stored in ["", "scrypt$zz$00", "scrypt$00$", "a$b$c$d", "scrypt$0011"] {
            assert!(!check_password("x", Some(stored)), "{stored}");
        }
    }
}
