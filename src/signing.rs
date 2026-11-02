//! HMAC signed, expiring download links.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;

use crate::config::Config;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Serialize, Deserialize)]
struct Payload {
    v: u8,
    u: String,
    e: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    m: Option<u64>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SigningError {
    #[error("signed links are disabled")]
    Disabled,
    #[error("token is malformed")]
    Malformed,
    #[error("token signature is invalid")]
    InvalidSignature,
    #[error("token has expired")]
    Expired,
    #[error("token payload is invalid")]
    InvalidPayload,
    #[error("requested ttl is invalid")]
    InvalidTtl,
    #[error("token is too long")]
    TooLong,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedLink {
    pub url: String,
    pub expires_at: u64,
    pub max_bytes: Option<u64>,
}

pub struct Signer {
    current: Vec<u8>,
    previous: Option<Vec<u8>>,
    enabled: bool,
    default_ttl: u64,
    max_ttl: u64,
    max_token_bytes: usize,
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signer")
            .field("enabled", &self.enabled)
            .field("default_ttl", &self.default_ttl)
            .field("max_ttl", &self.max_ttl)
            .field("has_previous_secret", &self.previous.is_some())
            .finish()
    }
}

impl Signer {
    pub fn new(config: &Config, secrets: &[Vec<u8>]) -> Self {
        let mut iter = secrets.iter();
        let current = iter.next().cloned().unwrap_or_default();
        let previous = iter.next().cloned();
        Self {
            current,
            previous,
            enabled: config.signing.enabled && !secrets.is_empty(),
            default_ttl: config.signing.default_ttl_secs,
            max_ttl: config.signing.max_ttl_secs,
            max_token_bytes: config.signing.max_token_bytes,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn default_ttl(&self) -> u64 {
        self.default_ttl
    }

    pub fn max_ttl(&self) -> u64 {
        self.max_ttl
    }

    pub fn sign(
        &self,
        url: &str,
        ttl: u64,
        max_bytes: Option<u64>,
    ) -> Result<String, SigningError> {
        self.sign_at(url, ttl, max_bytes, unix_now())
    }

    pub fn sign_at(
        &self,
        url: &str,
        ttl: u64,
        max_bytes: Option<u64>,
        now: u64,
    ) -> Result<String, SigningError> {
        if !self.enabled {
            return Err(SigningError::Disabled);
        }
        if ttl == 0 || ttl > self.max_ttl {
            return Err(SigningError::InvalidTtl);
        }
        let payload = Payload {
            v: 1,
            u: url.to_string(),
            e: now.saturating_add(ttl),
            m: max_bytes,
        };
        let bytes = serde_json::to_vec(&payload).map_err(|_| SigningError::InvalidPayload)?;
        let signature = sign_bytes(&self.current, &bytes);
        let token = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(&bytes),
            URL_SAFE_NO_PAD.encode(signature)
        );
        if token.len() > self.max_token_bytes {
            return Err(SigningError::TooLong);
        }
        Ok(token)
    }

    pub fn verify(&self, token: &str) -> Result<VerifiedLink, SigningError> {
        self.verify_at(token, unix_now())
    }

    pub fn verify_at(&self, token: &str, now: u64) -> Result<VerifiedLink, SigningError> {
        if !self.enabled {
            return Err(SigningError::Disabled);
        }
        if token.len() > self.max_token_bytes {
            return Err(SigningError::TooLong);
        }
        let (payload_part, signature_part) =
            token.split_once('.').ok_or(SigningError::Malformed)?;
        if payload_part.is_empty() || signature_part.is_empty() || signature_part.contains('.') {
            return Err(SigningError::Malformed);
        }
        let payload_bytes = URL_SAFE_NO_PAD
            .decode(payload_part)
            .map_err(|_| SigningError::Malformed)?;
        let signature = URL_SAFE_NO_PAD
            .decode(signature_part)
            .map_err(|_| SigningError::Malformed)?;
        if signature.len() != 32 {
            return Err(SigningError::Malformed);
        }
        let mut valid = verify_with(&self.current, &payload_bytes, &signature);
        if !valid && let Some(previous) = &self.previous {
            valid = verify_with(previous, &payload_bytes, &signature);
        }
        if !valid {
            return Err(SigningError::InvalidSignature);
        }

        let payload: Payload =
            serde_json::from_slice(&payload_bytes).map_err(|_| SigningError::InvalidPayload)?;
        if payload.v != 1 || payload.u.is_empty() {
            return Err(SigningError::InvalidPayload);
        }
        if now >= payload.e {
            return Err(SigningError::Expired);
        }
        Ok(VerifiedLink {
            url: payload.u,
            expires_at: payload.e,
            max_bytes: payload.m,
        })
    }
}

fn sign_bytes(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts keys of any length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn verify_with(key: &[u8], data: &[u8], signature: &[u8]) -> bool {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts keys of any length");
    mac.update(data);
    mac.verify_slice(signature).is_ok()
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn signer(enabled: bool, secrets: Vec<Vec<u8>>) -> Signer {
        let mut config: Config = toml::from_str("").unwrap();
        config.signing.enabled = enabled;
        config.signing.default_ttl_secs = 60;
        config.signing.max_ttl_secs = 3600;
        Signer::new(&config, &secrets)
    }

    #[test]
    fn round_trip_and_expiry() {
        let signer = signer(true, vec![vec![7u8; 32]]);
        let token = signer
            .sign_at("https://files.example.org/a.bin", 60, None, 1_000)
            .unwrap();
        let verified = signer.verify_at(&token, 1_001).unwrap();
        assert_eq!(verified.url, "https://files.example.org/a.bin");
        assert_eq!(verified.expires_at, 1_060);
        assert_eq!(verified.max_bytes, None);
        assert_eq!(signer.verify_at(&token, 1_060), Err(SigningError::Expired));
    }

    #[test]
    fn carries_optional_max_bytes() {
        let signer = signer(true, vec![vec![9u8; 32]]);
        let token = signer
            .sign_at("https://files.example.org/a.bin", 30, Some(1024), 100)
            .unwrap();
        assert_eq!(signer.verify_at(&token, 101).unwrap().max_bytes, Some(1024));
    }

    #[test]
    fn rejects_tampering() {
        let signer = signer(true, vec![vec![3u8; 32]]);
        let token = signer
            .sign_at("https://files.example.org/a.bin", 60, None, 10)
            .unwrap();
        let mut chars: Vec<char> = token.chars().collect();
        let index = chars.len() - 1;
        chars[index] = if chars[index] == 'A' { 'B' } else { 'A' };
        let tampered: String = chars.into_iter().collect();
        assert!(matches!(
            signer.verify_at(&tampered, 11),
            Err(SigningError::InvalidSignature) | Err(SigningError::Malformed)
        ));
    }

    #[test]
    fn rejects_garbage_and_long_tokens() {
        let signer = signer(true, vec![vec![1u8; 32]]);
        assert_eq!(signer.verify_at("", 1), Err(SigningError::Malformed));
        assert_eq!(signer.verify_at("a.b.c", 1), Err(SigningError::Malformed));
        let long = "a".repeat(5000);
        assert_eq!(signer.verify_at(&long, 1), Err(SigningError::TooLong));
    }

    #[test]
    fn rotation_accepts_previous_secret() {
        let old_secret = vec![5u8; 32];
        let new_secret = vec![6u8; 32];
        let old_signer = signer(true, vec![old_secret.clone()]);
        let token = old_signer
            .sign_at("https://files.example.org/a.bin", 60, None, 100)
            .unwrap();
        let rotating = signer(true, vec![new_secret, old_secret]);
        assert!(rotating.verify_at(&token, 101).is_ok());
        assert_eq!(
            rotating.verify_at("garbage", 101),
            Err(SigningError::Malformed)
        );
    }

    #[test]
    fn disabled_signer_rejects_everything() {
        let signer = signer(false, vec![vec![2u8; 32]]);
        assert!(matches!(
            signer.sign_at("https://x", 10, None, 1),
            Err(SigningError::Disabled)
        ));
        assert!(matches!(
            signer.verify_at("a.b", 1),
            Err(SigningError::Disabled)
        ));
    }

    #[test]
    fn rejects_bad_ttl() {
        let signer = signer(true, vec![vec![4u8; 32]]);
        assert_eq!(
            signer.sign_at("https://x", 0, None, 1),
            Err(SigningError::InvalidTtl)
        );
        assert_eq!(
            signer.sign_at("https://x", 86_401, None, 1),
            Err(SigningError::InvalidTtl)
        );
    }
}
