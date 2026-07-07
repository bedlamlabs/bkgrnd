//! PWA authentication: argon2 password hashing, a persistent JSON user store,
//! and HMAC-signed *stateless* session cookies. Because sessions are signed
//! rather than stored, they survive server restarts as long as the signing
//! secret is stable (it comes from WOPR_SESSION_SECRET, falling back to the
//! WOPR bearer token).
//!
//! The iOS app and the Mac menubar keep authenticating with the raw WOPR
//! bearer token and are completely unaffected by anything in this module.

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// 30-day browser session.
pub const SESSION_TTL_SECS: u64 = 60 * 60 * 24 * 30;
/// Short-lived signed token for Cast stream URLs (the receiver device can't
/// send our session cookie, so the URL itself must carry proof of auth).
pub const CAST_TTL_SECS: u64 = 60 * 60 * 6;
pub const SESSION_COOKIE: &str = "bkgrnd_session";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct User {
    pub username: String,
    #[serde(rename = "passwordHash")]
    pub password_hash: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

pub fn users_path(data_dir: &Path) -> PathBuf {
    data_dir.join("users.json")
}

pub fn load_users(data_dir: &Path) -> Vec<User> {
    match std::fs::read(users_path(data_dir)) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub fn save_users(data_dir: &Path, users: &[User]) -> std::io::Result<()> {
    let path = users_path(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(users).unwrap_or_default();
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &path)
}

/// Usernames are case-insensitive, 1-32 chars of [a-z0-9_.-]. Because a username
/// becomes a directory name for per-user storage, we also require at least one
/// alphanumeric char — this rejects ".", ".." and other path-traversal shapes.
pub fn normalize_username(raw: &str) -> Option<String> {
    let name = raw.trim().to_lowercase();
    if name.is_empty() || name.len() > 32 {
        return None;
    }
    let charset_ok = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    let has_alnum = name.chars().any(|c| c.is_ascii_alphanumeric());
    if charset_ok && has_alnum {
        Some(name)
    } else {
        None
    }
}

pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sign(secret: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(msg);
    mac.finalize().into_bytes().to_vec()
}

/// Session cookie value: `base64url(payload).base64url(hmac(payload))`, where
/// payload = `username:exp_unix`.
pub fn mint_session(secret: &[u8], username: &str, ttl_secs: u64) -> String {
    let exp = now_unix() + ttl_secs;
    let payload = format!("{username}:{exp}");
    let sig = sign(secret, payload.as_bytes());
    format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(payload.as_bytes()),
        URL_SAFE_NO_PAD.encode(sig)
    )
}

/// Returns the username if the token's signature is valid and it hasn't expired.
pub fn verify_session(secret: &[u8], token: &str) -> Option<String> {
    let (payload_b64, sig_b64) = token.split_once('.')?;
    let payload = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let sig = URL_SAFE_NO_PAD.decode(sig_b64).ok()?;
    let expected = sign(secret, &payload);
    if expected.ct_eq(&sig).unwrap_u8() != 1 {
        return None;
    }
    let payload = String::from_utf8(payload).ok()?;
    let (username, exp) = payload.rsplit_once(':')?;
    let exp: u64 = exp.parse().ok()?;
    if now_unix() > exp {
        return None;
    }
    Some(username.to_string())
}

/// Short-lived signed token embedded in Cast stream URLs. Not tied to a
/// specific track — just proves the request was authorized recently.
pub fn mint_cast_sig(secret: &[u8], ttl_secs: u64) -> String {
    let exp = now_unix() + ttl_secs;
    let sig = sign(secret, format!("cast:{exp}").as_bytes());
    format!("{exp}.{}", URL_SAFE_NO_PAD.encode(sig))
}

pub fn verify_cast_sig(secret: &[u8], token: &str) -> bool {
    let Some((exp_str, sig_b64)) = token.split_once('.') else {
        return false;
    };
    let Ok(exp) = exp_str.parse::<u64>() else {
        return false;
    };
    if now_unix() > exp {
        return false;
    }
    let Ok(sig) = URL_SAFE_NO_PAD.decode(sig_b64) else {
        return false;
    };
    let expected = sign(secret, format!("cast:{exp}").as_bytes());
    expected.ct_eq(&sig).unwrap_u8() == 1
}

/// Pull the session token out of a `Cookie:` header value.
pub fn session_from_cookie_header(cookie_header: &str) -> Option<String> {
    let prefix = format!("{SESSION_COOKIE}=");
    cookie_header
        .split(';')
        .filter_map(|pair| pair.trim().strip_prefix(prefix.as_str()))
        .next()
        .map(|s| s.to_string())
}

/// `Set-Cookie` value for a freshly minted session (HttpOnly + Secure so JS
/// can never read it and it only travels over HTTPS).
pub fn session_set_cookie(token: &str) -> String {
    format!(
        "{SESSION_COOKIE}={token}; Path=/; Max-Age={SESSION_TTL_SECS}; HttpOnly; Secure; SameSite=Lax"
    )
}

/// `Set-Cookie` value that clears the session (logout).
pub fn session_clear_cookie() -> String {
    format!("{SESSION_COOKIE}=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Lax")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_roundtrip() {
        let secret = b"unit-test-secret";
        let tok = mint_session(secret, "ethan", 60);
        assert_eq!(verify_session(secret, &tok).as_deref(), Some("ethan"));
        // Wrong secret fails.
        assert!(verify_session(b"other", &tok).is_none());
        // Tampered payload fails.
        assert!(verify_session(secret, "bogus.sig").is_none());
    }

    #[test]
    fn expired_session_rejected() {
        let secret = b"unit-test-secret";
        // TTL 0 → exp == now; a token minted "now" is not > now yet, so mint
        // with a payload already in the past by signing manually.
        let past = now_unix() - 10;
        let payload = format!("ethan:{past}");
        let sig = sign(secret, payload.as_bytes());
        let tok = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(payload.as_bytes()),
            URL_SAFE_NO_PAD.encode(sig)
        );
        assert!(verify_session(secret, &tok).is_none());
    }

    #[test]
    fn password_hash_roundtrip() {
        let hash = hash_password("3L!0n$").unwrap();
        assert!(verify_password("3L!0n$", &hash));
        assert!(!verify_password("wrong", &hash));
    }

    #[test]
    fn cast_sig_roundtrip() {
        let secret = b"unit-test-secret";
        let sig = mint_cast_sig(secret, 60);
        assert!(verify_cast_sig(secret, &sig));
        assert!(!verify_cast_sig(b"other", &sig));
    }

    #[test]
    fn username_rules() {
        assert_eq!(normalize_username(" Ethan ").as_deref(), Some("ethan"));
        assert!(normalize_username("bad name").is_none());
        assert!(normalize_username("").is_none());
        // Path-traversal shapes must be rejected (usernames become dir names).
        assert!(normalize_username("..").is_none());
        assert!(normalize_username(".").is_none());
        assert!(normalize_username("../etc").is_none());
    }

    #[test]
    fn cookie_parsing() {
        assert_eq!(
            session_from_cookie_header("foo=1; bkgrnd_session=abc.def; bar=2").as_deref(),
            Some("abc.def")
        );
        assert!(session_from_cookie_header("foo=1").is_none());
    }
}
