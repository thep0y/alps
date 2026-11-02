//! Manual redirect handling with per-hop revalidation.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use axum::http::{HeaderMap, Method, StatusCode, header::LOCATION};

use crate::config::Config;
use crate::http::errors::ProxyError;
use crate::http::headers::ForwardHeaders;
use crate::limits::{HostPermit, Limits};
use crate::policy::{PolicyError, UrlPolicy, ValidatedTarget};
use crate::upstream::resolver::{ResolveError, SecureResolver};

pub struct RedirectController {
    client: reqwest::Client,
    policy: Arc<UrlPolicy>,
    resolver: Arc<SecureResolver>,
    limits: Arc<Limits>,
    max_redirects: usize,
    header_timeout: Duration,
}

impl RedirectController {
    pub fn new(
        client: reqwest::Client,
        policy: Arc<UrlPolicy>,
        resolver: Arc<SecureResolver>,
        limits: Arc<Limits>,
        config: &Config,
    ) -> Self {
        Self {
            client,
            policy,
            resolver,
            limits,
            max_redirects: config.proxy.max_redirects,
            header_timeout: config.proxy.response_header_timeout(),
        }
    }

    /// Fetch the target, following and validating each allowed redirect.
    pub async fn fetch(
        &self,
        target: ValidatedTarget,
        method: Method,
        headers: ForwardHeaders,
    ) -> Result<UpstreamResponse, ProxyError> {
        let mut current = target;
        let mut host_permit = self
            .limits
            .acquire_host(current.host())
            .map_err(|_| ProxyError::CapacityExceeded)?;
        let mut visited = HashSet::new();
        visited.insert(current.url().as_str().to_string());
        let mut hops: u8 = 0;

        loop {
            // Preflight resolution gives a stable ip_denied signal; the
            // resolver used by reqwest revalidates on the actual connection.
            self.resolver
                .resolve_allowed(current.host())
                .await
                .map_err(map_resolve_error)?;

            let request = self
                .client
                .request(method.clone(), current.url().clone())
                .headers(headers.as_map().clone());
            let response = tokio::time::timeout(self.header_timeout, request.send())
                .await
                .map_err(|_| ProxyError::UpstreamTimeout)?
                .map_err(map_reqwest_error)?;

            let status = response.status();
            let follow = matches!(
                status,
                StatusCode::MOVED_PERMANENTLY
                    | StatusCode::FOUND
                    | StatusCode::SEE_OTHER
                    | StatusCode::TEMPORARY_REDIRECT
                    | StatusCode::PERMANENT_REDIRECT
            );
            // 304 is a bodyless final response, not a redirect to follow.
            if !follow {
                if status != StatusCode::NOT_MODIFIED && (300..400).contains(&status.as_u16()) {
                    return Err(ProxyError::UpstreamRedirectError);
                }
                let host = current.host().to_string();
                let response_headers = response.headers().clone();
                return Ok(UpstreamResponse {
                    status,
                    headers: response_headers,
                    body: response,
                    hops,
                    host,
                    host_permit,
                });
            }
            if hops as usize >= self.max_redirects {
                tracing::debug!(hops, "redirect limit reached");
                return Err(ProxyError::UpstreamRedirectError);
            }

            let location = response
                .headers()
                .get(LOCATION)
                .ok_or(ProxyError::UpstreamRedirectError)?
                .to_str()
                .map_err(|_| ProxyError::UpstreamRedirectError)?;
            let next_url = current
                .url()
                .join(location)
                .map_err(|_| ProxyError::UpstreamRedirectError)?;
            let next = match self.policy.validate_url(next_url) {
                Ok(next) => next,
                Err(PolicyError::InvalidUrl)
                | Err(PolicyError::TooLong)
                | Err(PolicyError::Empty)
                | Err(PolicyError::InvalidCharacters) => {
                    return Err(ProxyError::UpstreamRedirectError);
                }
                Err(_) => return Err(ProxyError::DestinationDenied),
            };
            if !visited.insert(next.url().as_str().to_string()) {
                tracing::debug!("redirect loop detected");
                return Err(ProxyError::UpstreamRedirectError);
            }

            // Acquire the next hop permit before dropping the old one so the
            // host limit never has a gap, then drop the redirect response.
            let next_permit = self
                .limits
                .acquire_host(next.host())
                .map_err(|_| ProxyError::CapacityExceeded)?;
            drop(response);
            host_permit = next_permit;
            current = next;
            hops += 1;
            crate::metrics::global().record_redirect();
        }
    }
}

pub struct UpstreamResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: reqwest::Response,
    pub hops: u8,
    pub host: String,
    pub host_permit: HostPermit,
}

fn map_resolve_error(error: ResolveError) -> ProxyError {
    match error {
        ResolveError::IpDenied => ProxyError::IpDenied,
        ResolveError::Timeout => ProxyError::UpstreamTimeout,
        ResolveError::Io(error) => {
            tracing::debug!(error = %error, "dns resolution failed");
            ProxyError::UpstreamDnsError
        }
        ResolveError::NotFound => ProxyError::UpstreamDnsError,
    }
}

fn map_reqwest_error(error: reqwest::Error) -> ProxyError {
    let error = error.without_url();
    let kind = if error.is_timeout() {
        "timeout"
    } else if error.is_dns() {
        "dns"
    } else if error.is_connect() {
        "connect"
    } else if error.is_body() {
        "body"
    } else if error.is_redirect() {
        "redirect"
    } else {
        "other"
    };
    tracing::debug!(error_kind = kind, error = %error, "upstream request failed");
    match kind {
        "timeout" => ProxyError::UpstreamTimeout,
        "dns" => ProxyError::UpstreamDnsError,
        "body" => ProxyError::UpstreamProtocolError,
        "redirect" => ProxyError::UpstreamRedirectError,
        _ => ProxyError::UpstreamConnectError,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_resolve_errors_to_proxy_errors() {
        assert_eq!(
            map_resolve_error(ResolveError::IpDenied).code(),
            "ip_denied"
        );
        assert_eq!(
            map_resolve_error(ResolveError::Timeout).code(),
            "upstream_timeout"
        );
        assert_eq!(
            map_resolve_error(ResolveError::NotFound).code(),
            "upstream_dns_error"
        );
        assert_eq!(
            map_resolve_error(ResolveError::Io(std::io::Error::other("dns"))).code(),
            "upstream_dns_error"
        );
    }
}
