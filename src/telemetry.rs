//! Tracing initialization and request id handling.

use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::HeaderMap;
use tracing_subscriber::EnvFilter;

use crate::config::{Config, LogFormat};

pub const REQUEST_ID_HEADER: &str = "x-request-id";

pub fn init(config: &Config) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let filter = EnvFilter::try_new(&config.logging.level)?;
    match config.logging.format {
        LogFormat::Json => {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .json()
                .try_init()?;
        }
        LogFormat::Text => {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .try_init()?;
        }
    }
    Ok(())
}

/// Use a client supplied request id when it is well formed, otherwise generate one.
pub fn request_id(headers: &HeaderMap) -> String {
    match headers
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
    {
        Some(value) if is_valid_request_id(value) => return value.to_string(),
        _ => {}
    }
    generate_request_id()
}

fn is_valid_request_id(value: &str) -> bool {
    (8..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn generate_request_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or_default();
    let entropy = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    format!("{entropy:016x}{nanos:08x}{counter:04x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header::HeaderValue;

    #[test]
    fn accepts_valid_incoming_request_id() {
        let mut headers = HeaderMap::new();
        headers.insert(REQUEST_ID_HEADER, HeaderValue::from_static("abcd-1234_xyz"));
        assert_eq!(request_id(&headers), "abcd-1234_xyz");
    }

    #[test]
    fn replaces_invalid_request_id() {
        let mut headers = HeaderMap::new();
        headers.insert(REQUEST_ID_HEADER, HeaderValue::from_static("bad id!"));
        let generated = request_id(&headers);
        assert_ne!(generated, "bad id!");
        assert!(is_valid_request_id(&generated));
    }

    #[test]
    fn generated_ids_are_unique() {
        assert_ne!(generate_request_id(), generate_request_id());
    }

    #[test]
    fn init_supports_text_and_json_and_rejects_bad_filter() {
        let mut text: Config = toml::from_str("").unwrap();
        text.logging.level = "info".into();
        text.logging.format = LogFormat::Text;
        init(&text).expect("first subscriber init");

        let mut json: Config = toml::from_str("").unwrap();
        json.logging.format = LogFormat::Json;
        assert!(init(&json).is_err(), "second global init must fail");

        let mut bad: Config = toml::from_str("").unwrap();
        bad.logging.level = "[".into();
        assert!(init(&bad).is_err());
    }
}
