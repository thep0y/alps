//! Proxy error type and JSON error responses.

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use serde_json::json;
use thiserror::Error;

use crate::policy::PolicyError;

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("invalid url")]
    InvalidUrl,
    #[error("invalid request")]
    InvalidRequest,
    #[error("invalid token")]
    InvalidToken,
    #[error("token expired")]
    TokenExpired,
    #[error("signed links are disabled")]
    SigningDisabled,
    #[error("unauthorized")]
    Unauthorized,
    #[error("method not allowed")]
    MethodNotAllowed,
    #[error("destination denied")]
    DestinationDenied,
    #[error("ip denied")]
    IpDenied,
    #[error("capacity exceeded")]
    CapacityExceeded,
    #[error("rate limited")]
    RateLimited,
    #[error("upstream redirect error")]
    UpstreamRedirectError,
    #[error("upstream connect error")]
    UpstreamConnectError,
    #[error("upstream dns error")]
    UpstreamDnsError,
    #[error("upstream timeout")]
    UpstreamTimeout,
    #[error("upstream protocol error")]
    UpstreamProtocolError,
    #[error("request body not allowed")]
    RequestBodyNotAllowed,
    #[error("upstream response too large")]
    UpstreamResponseTooLarge,
    #[error("internal error")]
    Internal,
}

impl ProxyError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::InvalidUrl | Self::InvalidRequest | Self::RequestBodyNotAllowed => {
                StatusCode::BAD_REQUEST
            }
            Self::Unauthorized | Self::InvalidToken => StatusCode::UNAUTHORIZED,
            Self::DestinationDenied | Self::IpDenied | Self::TokenExpired => StatusCode::FORBIDDEN,
            Self::SigningDisabled => StatusCode::NOT_FOUND,
            Self::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::CapacityExceeded => StatusCode::SERVICE_UNAVAILABLE,
            Self::UpstreamRedirectError
            | Self::UpstreamConnectError
            | Self::UpstreamDnsError
            | Self::UpstreamProtocolError
            | Self::UpstreamResponseTooLarge => StatusCode::BAD_GATEWAY,
            Self::UpstreamTimeout => StatusCode::GATEWAY_TIMEOUT,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidUrl => "invalid_url",
            Self::InvalidRequest => "invalid_request",
            Self::InvalidToken => "invalid_token",
            Self::TokenExpired => "token_expired",
            Self::SigningDisabled => "signing_disabled",
            Self::Unauthorized => "unauthorized",
            Self::MethodNotAllowed => "method_not_allowed",
            Self::DestinationDenied => "destination_denied",
            Self::IpDenied => "ip_denied",
            Self::CapacityExceeded => "capacity_exceeded",
            Self::RateLimited => "rate_limited",
            Self::UpstreamRedirectError => "upstream_redirect_error",
            Self::UpstreamConnectError => "upstream_connect_error",
            Self::UpstreamDnsError => "upstream_dns_error",
            Self::UpstreamTimeout => "upstream_timeout",
            Self::UpstreamProtocolError => "upstream_protocol_error",
            Self::RequestBodyNotAllowed => "request_body_not_allowed",
            Self::UpstreamResponseTooLarge => "upstream_response_too_large",
            Self::Internal => "internal_error",
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            Self::InvalidUrl => "the target url is missing or invalid",
            Self::InvalidRequest => "the sign request is invalid",
            Self::InvalidToken => "the signed link is invalid",
            Self::TokenExpired => "the signed link has expired",
            Self::SigningDisabled => "signed links are not enabled",
            Self::Unauthorized => "a valid bearer token is required",
            Self::MethodNotAllowed => "only GET and HEAD are supported",
            Self::DestinationDenied => "the destination is not allowed",
            Self::IpDenied => "the destination resolves to a non-public address",
            Self::CapacityExceeded => "too many active requests, retry later",
            Self::RateLimited => "request rate limit exceeded",
            Self::UpstreamRedirectError => "the upstream redirect chain is invalid",
            Self::UpstreamConnectError => "failed to connect to the upstream",
            Self::UpstreamDnsError => "failed to resolve the upstream host",
            Self::UpstreamTimeout => "the upstream timed out",
            Self::UpstreamProtocolError => "the upstream response is invalid",
            Self::RequestBodyNotAllowed => "GET and HEAD requests must not include a body",
            Self::UpstreamResponseTooLarge => "the upstream response exceeds the configured limit",
            Self::Internal => "internal error",
        }
    }
}

impl From<PolicyError> for ProxyError {
    fn from(error: PolicyError) -> Self {
        match error {
            PolicyError::Empty
            | PolicyError::InvalidUrl
            | PolicyError::TooLong
            | PolicyError::InvalidCharacters => Self::InvalidUrl,
            PolicyError::UnsupportedScheme
            | PolicyError::HttpDisabled
            | PolicyError::UserInfo
            | PolicyError::Fragment
            | PolicyError::IpLiteral
            | PolicyError::DestinationDenied => Self::DestinationDenied,
        }
    }
}

pub fn error_response(error: &ProxyError, request_id: &str, is_head: bool) -> Response {
    let body = json!({
        "error": {
            "code": error.code(),
            "message": error.message(),
            "request_id": request_id,
        }
    });
    let bytes = serde_json::to_vec(&body).unwrap_or_else(|_| b"{\"error\":{}}".to_vec());

    let mut builder = Response::builder()
        .status(error.status())
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store");

    if let StatusCode::UNAUTHORIZED = error.status() {
        builder = builder.header(header::WWW_AUTHENTICATE, "Bearer realm=\"alps\"");
    }
    if let StatusCode::METHOD_NOT_ALLOWED = error.status() {
        builder = builder.header(header::ALLOW, "GET, HEAD");
    }
    if let StatusCode::TOO_MANY_REQUESTS = error.status() {
        builder = builder.header(header::RETRY_AFTER, "1");
    }

    if is_head {
        builder
            .header(header::CONTENT_LENGTH, HeaderValue::from_static("0"))
            .body(Body::empty())
            .expect("valid response")
    } else {
        builder.body(Body::from(bytes)).expect("valid response")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_policy_errors_to_status_codes() {
        assert_eq!(
            ProxyError::from(PolicyError::DestinationDenied).status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            ProxyError::from(PolicyError::IpLiteral).status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            ProxyError::from(PolicyError::InvalidUrl).status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn error_body_has_code_and_request_id() {
        let response = error_response(&ProxyError::DestinationDenied, "req-1", false);
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }

    #[test]
    fn head_error_has_no_body() {
        let response = error_response(&ProxyError::InvalidUrl, "req-1", true);
        assert_eq!(response.headers()[header::CONTENT_LENGTH], "0");
    }

    #[test]
    fn method_not_allowed_advertises_allow() {
        let response = error_response(&ProxyError::MethodNotAllowed, "req-1", false);
        assert_eq!(response.headers()[header::ALLOW], "GET, HEAD");
    }

    #[test]
    fn all_variants_have_status_code_and_message() {
        use ProxyError::*;
        let errors = [
            InvalidUrl,
            InvalidRequest,
            InvalidToken,
            TokenExpired,
            SigningDisabled,
            Unauthorized,
            MethodNotAllowed,
            DestinationDenied,
            IpDenied,
            CapacityExceeded,
            RateLimited,
            UpstreamRedirectError,
            UpstreamConnectError,
            UpstreamDnsError,
            UpstreamTimeout,
            UpstreamProtocolError,
            RequestBodyNotAllowed,
            UpstreamResponseTooLarge,
            Internal,
        ];
        for error in &errors {
            assert!(!error.code().is_empty());
            assert!(!error.message().is_empty());
            let _ = error.status();
        }
        assert_eq!(CapacityExceeded.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(UpstreamTimeout.status(), StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(Internal.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn auth_and_rate_limit_responses_add_headers() {
        let unauthorized = error_response(&ProxyError::Unauthorized, "req-1", false);
        assert_eq!(
            unauthorized.headers()[header::WWW_AUTHENTICATE],
            "Bearer realm=\"alps\""
        );
        let limited = error_response(&ProxyError::RateLimited, "req-1", false);
        assert_eq!(limited.headers()[header::RETRY_AFTER], "1");
    }
}
