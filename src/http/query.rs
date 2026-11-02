//! Raw query-string parsing that preserves URL percent-encoding.

use axum::http::Uri;
use percent_encoding::percent_decode_str;

use crate::http::errors::ProxyError;

/// Extract the url query parameter without form-style plus decoding.
pub fn extract_target_url(uri: &Uri, max_url_length: usize) -> Result<String, ProxyError> {
    let query = uri.query().ok_or(ProxyError::InvalidUrl)?;
    if query.len() > max_url_length.saturating_mul(3).saturating_add(64) {
        return Err(ProxyError::InvalidUrl);
    }

    let mut found: Option<String> = None;
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if key != "url" {
            continue;
        }
        if found.is_some() {
            return Err(ProxyError::InvalidUrl);
        }
        let decoded = percent_decode_str(value)
            .decode_utf8()
            .map_err(|_| ProxyError::InvalidUrl)?;
        found = Some(decoded.into_owned());
    }

    let url = found.ok_or(ProxyError::InvalidUrl)?;
    if url.is_empty() || url.len() > max_url_length {
        return Err(ProxyError::InvalidUrl);
    }
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(ProxyError::InvalidUrl);
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(query: &str) -> Uri {
        format!("/v1/proxy?{query}").parse().unwrap()
    }

    #[test]
    fn decodes_percent_encoding_without_form_rules() {
        assert_eq!(
            extract_target_url(&uri("url=https%3A%2F%2Fexample.org%2Fa%2Bb"), 8192).unwrap(),
            "https://example.org/a+b"
        );
        assert_eq!(
            extract_target_url(&uri("url=https%3A%2F%2Fexample.org%2Fa+b"), 8192).unwrap(),
            "https://example.org/a+b"
        );
        assert_eq!(
            extract_target_url(&uri("url=https%3A%2F%2Fexample.org%2Fa%2520b"), 8192).unwrap(),
            "https://example.org/a%20b"
        );
        // A literal space in the decoded URL is rejected as invalid input.
        assert!(extract_target_url(&uri("url=https%3A%2F%2Fexample.org%2Fa%20b"), 8192).is_err());
    }

    #[test]
    fn rejects_missing_duplicate_and_empty() {
        assert!(extract_target_url(&uri("other=1"), 8192).is_err());
        assert!(extract_target_url(&uri("url=a&url=b"), 8192).is_err());
        assert!(extract_target_url(&uri("url="), 8192).is_err());
        assert!(extract_target_url(&Uri::from_static("/v1/proxy"), 8192).is_err());
    }

    #[test]
    fn rejects_control_characters_and_bad_utf8() {
        assert!(extract_target_url(&uri("url=https%3A%2F%2Fexample.org%2F%00"), 8192).is_err());
        assert!(extract_target_url(&uri("url=https%3A%2F%2Fexample.org%2F%ff"), 8192).is_err());
    }

    #[test]
    fn enforces_length_limit() {
        let long = format!("url=https%3A%2F%2Fexample.org%2F{}", "a".repeat(200));
        assert!(extract_target_url(&uri(&long), 128).is_err());
    }
}
