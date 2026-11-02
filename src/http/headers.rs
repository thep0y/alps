//! Request and response header filtering, including security headers.

use std::collections::HashSet;

use axum::http::{
    HeaderMap, StatusCode,
    header::{self, HeaderName, HeaderValue},
};

const RESPONSE_ALLOW: &[HeaderName] = &[
    header::CONTENT_TYPE,
    header::CONTENT_LENGTH,
    header::CONTENT_RANGE,
    header::ACCEPT_RANGES,
    header::ETAG,
    header::LAST_MODIFIED,
    header::CACHE_CONTROL,
    header::EXPIRES,
    header::CONTENT_DISPOSITION,
    header::CONTENT_ENCODING,
];

#[derive(Debug, Clone)]
pub struct ForwardHeaders(HeaderMap);

impl ForwardHeaders {
    pub fn as_map(&self) -> &HeaderMap {
        &self.0
    }
}

/// Copy only file-semantics request headers and force identity encoding.
pub fn forward_request(headers: &HeaderMap, forward_accept: bool) -> ForwardHeaders {
    let mut out = HeaderMap::new();
    for name in [
        header::RANGE,
        header::IF_RANGE,
        header::IF_NONE_MATCH,
        header::IF_MODIFIED_SINCE,
    ] {
        copy_all(headers, &mut out, name);
    }
    if forward_accept {
        copy_all(headers, &mut out, header::ACCEPT);
    }
    out.insert(
        header::ACCEPT_ENCODING,
        HeaderValue::from_static("identity"),
    );
    ForwardHeaders(out)
}

fn copy_all(from: &HeaderMap, to: &mut HeaderMap, name: HeaderName) {
    for value in from.get_all(&name).iter() {
        to.append(name.clone(), value.clone());
    }
}

/// True when the inbound request carries a body that must be rejected.
pub fn has_request_body(headers: &HeaderMap) -> bool {
    if headers.contains_key(header::TRANSFER_ENCODING) {
        return true;
    }
    match headers.get(header::CONTENT_LENGTH) {
        None => false,
        Some(value) => match value
            .to_str()
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
        {
            Some(0) => false,
            Some(_) => true,
            None => true,
        },
    }
}

/// Build the downstream response headers from the upstream response headers.
pub fn filter_response(upstream: &HeaderMap, status: StatusCode) -> HeaderMap {
    let connection_tokens = connection_tokens(upstream);
    let mut out = HeaderMap::new();

    for name in RESPONSE_ALLOW {
        if connection_tokens.contains(name) {
            continue;
        }
        copy_all(upstream, &mut out, name.clone());
    }

    if status == StatusCode::NO_CONTENT || status == StatusCode::NOT_MODIFIED {
        out.remove(header::CONTENT_LENGTH);
        out.remove(header::CONTENT_RANGE);
    }

    if let Some(value) = out.get(header::CONTENT_DISPOSITION).cloned() {
        match sanitize_content_disposition(&value) {
            Some(value) => {
                out.insert(header::CONTENT_DISPOSITION, value);
            }
            None => {
                out.remove(header::CONTENT_DISPOSITION);
            }
        }
    }

    out.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    out.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    out.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox"),
    );
    if !out.contains_key(header::CONTENT_DISPOSITION) {
        out.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment"),
        );
    }
    out
}

fn connection_tokens(headers: &HeaderMap) -> HashSet<HeaderName> {
    let mut tokens = HashSet::new();
    for value in headers.get_all(header::CONNECTION).iter() {
        let Ok(value) = value.to_str() else {
            continue;
        };
        for token in value.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            if let Ok(name) = HeaderName::from_bytes(token.as_bytes()) {
                tokens.insert(name);
            }
        }
    }
    for name in [
        header::CONNECTION,
        HeaderName::from_static("keep-alive"),
        header::PROXY_AUTHENTICATE,
        header::PROXY_AUTHORIZATION,
        header::TE,
        header::TRAILER,
        header::TRANSFER_ENCODING,
        header::UPGRADE,
    ] {
        tokens.insert(name);
    }
    tokens
}

fn sanitize_content_disposition(value: &HeaderValue) -> Option<HeaderValue> {
    let text = value.to_str().ok()?;
    if text.is_empty() || text.chars().any(|c| c.is_control()) {
        return None;
    }
    if text.contains('/') || text.contains('\\') || text.contains("..") {
        return None;
    }
    if !text
        .get(..10)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("attachment"))
    {
        return None;
    }
    Some(value.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_only_allowed_request_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::RANGE, "bytes=0-10".parse().unwrap());
        headers.insert(header::COOKIE, "a=b".parse().unwrap());
        headers.insert(header::AUTHORIZATION, "Bearer x".parse().unwrap());
        headers.insert(header::HOST, "alps.local".parse().unwrap());
        headers.insert(header::ACCEPT, "text/html".parse().unwrap());

        let forwarded = forward_request(&headers, false);
        assert_eq!(forwarded.as_map()[header::RANGE], "bytes=0-10");
        assert!(!forwarded.as_map().contains_key(header::COOKIE));
        assert!(!forwarded.as_map().contains_key(header::AUTHORIZATION));
        assert!(!forwarded.as_map().contains_key(header::HOST));
        assert!(!forwarded.as_map().contains_key(header::ACCEPT));
        assert_eq!(forwarded.as_map()[header::ACCEPT_ENCODING], "identity");
    }

    #[test]
    fn can_forward_accept_when_enabled() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, "video/mp4".parse().unwrap());
        let forwarded = forward_request(&headers, true);
        assert_eq!(forwarded.as_map()[header::ACCEPT], "video/mp4");
    }

    #[test]
    fn detects_request_bodies() {
        let mut headers = HeaderMap::new();
        assert!(!has_request_body(&headers));
        headers.insert(header::CONTENT_LENGTH, "0".parse().unwrap());
        assert!(!has_request_body(&headers));
        headers.insert(header::CONTENT_LENGTH, "12".parse().unwrap());
        assert!(has_request_body(&headers));
        headers.remove(header::CONTENT_LENGTH);
        headers.insert(header::TRANSFER_ENCODING, "chunked".parse().unwrap());
        assert!(has_request_body(&headers));
    }

    #[test]
    fn strips_sensitive_and_hop_by_hop_response_headers() {
        let mut upstream = HeaderMap::new();
        upstream.insert(
            header::CONTENT_TYPE,
            "application/octet-stream".parse().unwrap(),
        );
        upstream.insert(header::SET_COOKIE, "a=b".parse().unwrap());
        upstream.insert(header::LOCATION, "https://evil.example/".parse().unwrap());
        upstream.insert(header::TRANSFER_ENCODING, "chunked".parse().unwrap());
        upstream.insert(header::CONNECTION, "keep-alive, X-Secret".parse().unwrap());
        upstream.insert("x-secret", "leak".parse().unwrap());

        let filtered = filter_response(&upstream, StatusCode::OK);
        assert_eq!(filtered[header::CONTENT_TYPE], "application/octet-stream");
        assert!(!filtered.contains_key(header::SET_COOKIE));
        assert!(!filtered.contains_key(header::LOCATION));
        assert!(!filtered.contains_key(header::TRANSFER_ENCODING));
        assert!(!filtered.contains_key("x-secret"));
        assert_eq!(filtered[header::CONTENT_DISPOSITION], "attachment");
        assert_eq!(filtered["x-content-type-options"], "nosniff");
        assert_eq!(filtered[header::CONTENT_SECURITY_POLICY], "sandbox");
    }

    #[test]
    fn connection_token_can_hide_allowlisted_header() {
        let mut upstream = HeaderMap::new();
        upstream.insert(header::CONTENT_TYPE, "text/html".parse().unwrap());
        upstream.insert(header::CONNECTION, "Content-Type".parse().unwrap());
        let filtered = filter_response(&upstream, StatusCode::OK);
        assert!(!filtered.contains_key(header::CONTENT_TYPE));
    }

    #[test]
    fn removes_content_length_from_bodyless_statuses() {
        let mut upstream = HeaderMap::new();
        upstream.insert(header::CONTENT_LENGTH, "123".parse().unwrap());
        upstream.insert(header::CONTENT_RANGE, "bytes 0-1/2".parse().unwrap());
        let filtered = filter_response(&upstream, StatusCode::NOT_MODIFIED);
        assert!(!filtered.contains_key(header::CONTENT_LENGTH));
        assert!(!filtered.contains_key(header::CONTENT_RANGE));
    }

    #[test]
    fn sanitizes_content_disposition() {
        let mut upstream = HeaderMap::new();
        upstream.insert(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"file.bin\"".parse().unwrap(),
        );
        let filtered = filter_response(&upstream, StatusCode::OK);
        assert!(
            filtered[header::CONTENT_DISPOSITION]
                .to_str()
                .unwrap()
                .contains("file.bin")
        );

        let mut inline = HeaderMap::new();
        inline.insert(header::CONTENT_DISPOSITION, "inline".parse().unwrap());
        let filtered = filter_response(&inline, StatusCode::OK);
        assert_eq!(filtered[header::CONTENT_DISPOSITION], "attachment");

        let mut evil = HeaderMap::new();
        evil.insert(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"../etc/passwd\"".parse().unwrap(),
        );
        let filtered = filter_response(&evil, StatusCode::OK);
        assert_eq!(filtered[header::CONTENT_DISPOSITION], "attachment");
    }
}
