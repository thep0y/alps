//! URL normalization, allowlist matching and the ValidatedTarget type.

pub mod allowlist;
pub mod ip_policy;

use std::borrow::Cow;

use thiserror::Error;
use url::{Host, Url};

use crate::config::{Config, normalize_host};
use allowlist::Allowlist;

pub use ip_policy::IpPolicy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Http,
    Https,
}

impl Scheme {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "http" => Some(Self::Http),
            "https" => Some(Self::Https),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Self::Http => 80,
            Self::Https => 443,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolicyError {
    #[error("target url is empty")]
    Empty,
    #[error("target url is too long")]
    TooLong,
    #[error("target url contains control or whitespace characters")]
    InvalidCharacters,
    #[error("target url is not a valid absolute URL")]
    InvalidUrl,
    #[error("only http and https are supported")]
    UnsupportedScheme,
    #[error("http destinations are disabled")]
    HttpDisabled,
    #[error("userinfo is not allowed in target urls")]
    UserInfo,
    #[error("fragments are not allowed in target urls")]
    Fragment,
    #[error("IP literal destinations are not allowed")]
    IpLiteral,
    #[error("destination is not allowlisted")]
    DestinationDenied,
}

/// A target that has passed scheme, host, port and IP-literal checks.
///
/// Fields are private: the only way to obtain one is through UrlPolicy.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedTarget {
    url: Url,
    host: String,
    scheme: Scheme,
    port: u16,
}

impl ValidatedTarget {
    fn new(url: Url, host: String, scheme: Scheme, port: u16) -> Self {
        Self {
            url,
            host,
            scheme,
            port,
        }
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn scheme(&self) -> Scheme {
        self.scheme
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

#[derive(Debug, Clone)]
pub struct UrlPolicy {
    allowlist: Allowlist,
    allow_http: bool,
    max_url_length: usize,
}

impl UrlPolicy {
    pub fn from_config(config: &Config) -> Self {
        Self {
            allowlist: Allowlist::compile(&config.allowlist),
            allow_http: config.proxy.allow_http,
            max_url_length: config.proxy.max_url_length,
        }
    }

    pub fn allowlist_len(&self) -> usize {
        self.allowlist.len()
    }

    /// Validate a raw URL string coming from a request or a Location header.
    pub fn validate(&self, raw: &str) -> Result<ValidatedTarget, PolicyError> {
        if raw.is_empty() {
            return Err(PolicyError::Empty);
        }
        if raw.len() > self.max_url_length {
            return Err(PolicyError::TooLong);
        }
        if raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(PolicyError::InvalidCharacters);
        }
        let normalized = with_default_scheme(raw);
        if normalized.len() > self.max_url_length {
            return Err(PolicyError::TooLong);
        }
        let url = Url::parse(normalized.as_ref()).map_err(|_| PolicyError::InvalidUrl)?;
        self.validate_parsed(url)
    }

    /// Validate a URL that has already been parsed (used for redirects).
    pub fn validate_url(&self, url: Url) -> Result<ValidatedTarget, PolicyError> {
        if url.as_str().len() > self.max_url_length {
            return Err(PolicyError::TooLong);
        }
        self.validate_parsed(url)
    }

    fn validate_parsed(&self, mut url: Url) -> Result<ValidatedTarget, PolicyError> {
        let scheme = match url.scheme() {
            "https" => Scheme::Https,
            "http" => Scheme::Http,
            _ => return Err(PolicyError::UnsupportedScheme),
        };
        if scheme == Scheme::Http && !self.allow_http {
            return Err(PolicyError::HttpDisabled);
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(PolicyError::UserInfo);
        }
        if url.fragment().is_some() {
            return Err(PolicyError::Fragment);
        }

        let raw_host = match url.host() {
            Some(Host::Domain(domain)) => domain.to_string(),
            Some(Host::Ipv4(_)) | Some(Host::Ipv6(_)) => return Err(PolicyError::IpLiteral),
            None => return Err(PolicyError::InvalidUrl),
        };
        let host = normalize_host(&raw_host).ok_or(PolicyError::InvalidUrl)?;

        let port = url.port_or_known_default().ok_or(PolicyError::InvalidUrl)?;

        if !self.allowlist.permits(&host, scheme, port, url.path()) {
            return Err(PolicyError::DestinationDenied);
        }

        if raw_host != host {
            url.set_host(Some(&host))
                .map_err(|_| PolicyError::InvalidUrl)?;
        }

        Ok(ValidatedTarget::new(url, host, scheme, port))
    }
}

/// Add the default HTTPS scheme when callers pass a bare host or a
/// protocol-relative URL. Explicit schemes are preserved and validated later.
fn with_default_scheme(raw: &str) -> Cow<'_, str> {
    if raw.starts_with("//") {
        Cow::Owned(format!("https:{raw}"))
    } else if has_explicit_scheme(raw) {
        Cow::Borrowed(raw)
    } else {
        Cow::Owned(format!("https://{raw}"))
    }
}

fn has_explicit_scheme(raw: &str) -> bool {
    let Some((scheme, _)) = raw.split_once("://") else {
        return false;
    };
    let mut chars = scheme.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AllowlistRule;

    fn config_with(rules: Vec<AllowlistRule>, allow_http: bool) -> Config {
        let mut config: Config = toml::from_str("").unwrap();
        config.allowlist = rules;
        config.proxy.allow_http = allow_http;
        config
    }

    fn rule(host: &str, subdomains: bool) -> AllowlistRule {
        AllowlistRule {
            host: host.to_string(),
            include_subdomains: subdomains,
            schemes: vec!["https".into()],
            ports: vec![443],
            path_prefixes: Vec::new(),
        }
    }

    fn policy() -> UrlPolicy {
        UrlPolicy::from_config(&config_with(
            vec![
                rule("files.example.org", false),
                rule("cdn.example.net", true),
            ],
            false,
        ))
    }

    #[test]
    fn accepts_allowlisted_host() {
        let target = policy()
            .validate("https://files.example.org/releases/app.tar.gz")
            .unwrap();
        assert_eq!(target.host(), "files.example.org");
        assert_eq!(target.port(), 443);
        assert_eq!(target.scheme(), Scheme::Https);
    }

    #[test]
    fn defaults_bare_urls_to_https() {
        let target = policy()
            .validate("files.example.org/releases/app.tar.gz")
            .unwrap();
        assert_eq!(target.scheme(), Scheme::Https);
        assert_eq!(target.port(), 443);
        assert_eq!(
            target.url().as_str(),
            "https://files.example.org/releases/app.tar.gz"
        );

        let target = policy().validate("//files.example.org/x").unwrap();
        assert_eq!(target.scheme(), Scheme::Https);
        assert_eq!(target.url().as_str(), "https://files.example.org/x");
    }

    #[test]
    fn default_scheme_ignores_double_slashes_in_the_query() {
        let target = policy()
            .validate("files.example.org/file?next=https://evil.example/x")
            .unwrap();
        assert_eq!(target.host(), "files.example.org");
        assert_eq!(target.url().query(), Some("next=https://evil.example/x"));
    }

    #[test]
    fn bare_host_port_uses_https_port_rule() {
        let mut custom = rule("files.example.org", false);
        custom.ports = vec![8443];
        let policy = UrlPolicy::from_config(&config_with(vec![custom], false));
        let target = policy.validate("files.example.org:8443/x").unwrap();
        assert_eq!(target.scheme(), Scheme::Https);
        assert_eq!(target.port(), 8443);
    }

    #[test]
    fn explicit_http_scheme_is_preserved() {
        let mut custom = rule("files.example.org", false);
        custom.schemes = vec!["http".into()];
        custom.ports = vec![80];
        let policy = UrlPolicy::from_config(&config_with(vec![custom], true));
        let target = policy.validate("http://files.example.org/x").unwrap();
        assert_eq!(target.scheme(), Scheme::Http);
    }

    #[test]
    fn accepts_subdomain_rule() {
        assert!(policy().validate("https://a.cdn.example.net/x").is_ok());
    }

    #[test]
    fn rejects_suffix_lookalike() {
        assert_eq!(
            policy().validate("https://files.example.org.evil.example/x"),
            Err(PolicyError::DestinationDenied)
        );
        assert_eq!(
            policy().validate("https://notfiles.example.org/x"),
            Err(PolicyError::DestinationDenied)
        );
    }

    #[test]
    fn rejects_userinfo_fragment_and_ip_literals() {
        let policy = policy();
        assert_eq!(
            policy.validate("https://user:pass@files.example.org/x"),
            Err(PolicyError::UserInfo)
        );
        assert_eq!(
            policy.validate("https://files.example.org/x#frag"),
            Err(PolicyError::Fragment)
        );
        assert_eq!(
            policy.validate("https://93.184.216.34/x"),
            Err(PolicyError::IpLiteral)
        );
        assert_eq!(
            policy.validate("https://[2606:4700::1]/x"),
            Err(PolicyError::IpLiteral)
        );
    }

    #[test]
    fn rejects_http_by_default() {
        assert_eq!(
            policy().validate("http://files.example.org/x"),
            Err(PolicyError::HttpDisabled)
        );
    }

    #[test]
    fn rejects_non_default_port_without_rule() {
        assert_eq!(
            policy().validate("https://files.example.org:8443/x"),
            Err(PolicyError::DestinationDenied)
        );
    }

    #[test]
    fn rejects_invalid_and_control_input() {
        let policy = policy();
        assert_eq!(policy.validate(""), Err(PolicyError::Empty));
        assert_eq!(
            policy.validate("not a url"),
            Err(PolicyError::InvalidCharacters)
        );
        assert_eq!(
            policy.validate("https://files.example.org/a b"),
            Err(PolicyError::InvalidCharacters)
        );
        assert_eq!(
            policy.validate("ftp://files.example.org/x"),
            Err(PolicyError::UnsupportedScheme)
        );
    }

    #[test]
    fn rejects_too_long_url() {
        let mut config = config_with(vec![rule("files.example.org", false)], false);
        config.proxy.max_url_length = 128;
        let policy = UrlPolicy::from_config(&config);
        let url = format!("https://files.example.org/{}", "a".repeat(200));
        assert_eq!(policy.validate(&url), Err(PolicyError::TooLong));

        let mut short = config_with(vec![rule("files.example.org", false)], false);
        short.proxy.max_url_length = 20;
        let short = UrlPolicy::from_config(&short);
        assert_eq!(
            short.validate("files.example.org/x"),
            Err(PolicyError::TooLong)
        );
    }

    #[test]
    fn normalizes_trailing_dot_and_case() {
        let mut config = config_with(vec![rule("files.example.org", false)], false);
        config.proxy.max_url_length = 8192;
        let policy = UrlPolicy::from_config(&config);
        let target = policy.validate("https://FILES.example.ORG./x").unwrap();
        assert_eq!(target.host(), "files.example.org");
        assert_eq!(target.url().host_str(), Some("files.example.org"));
    }

    #[test]
    fn matches_idna_rule_and_target() {
        let mut config = config_with(vec![rule("bücher.example", false)], false);
        config.proxy.max_url_length = 8192;
        let policy = UrlPolicy::from_config(&config);
        let target = policy.validate("https://bücher.example/file").unwrap();
        assert!(target.host().starts_with("xn--"));
    }

    #[test]
    fn empty_allowlist_denies_everything() {
        let policy = UrlPolicy::from_config(&config_with(vec![], false));
        assert_eq!(
            policy.validate("https://files.example.org/x"),
            Err(PolicyError::DestinationDenied)
        );
    }

    #[test]
    fn redirect_url_is_revalidated() {
        let policy = policy();
        let url = Url::parse("https://evil.example/x").unwrap();
        assert_eq!(
            policy.validate_url(url),
            Err(PolicyError::DestinationDenied)
        );
    }

    #[test]
    fn path_prefixes_are_enforced_after_url_normalization() {
        let mut release = rule("files.example.org", false);
        release.path_prefixes = vec!["/releases/".into()];
        let policy = UrlPolicy::from_config(&config_with(vec![release], false));

        assert!(
            policy
                .validate("https://files.example.org/releases/app.tar")
                .is_ok()
        );
        assert_eq!(
            policy.validate("https://files.example.org/other"),
            Err(PolicyError::DestinationDenied)
        );
        assert_eq!(
            policy.validate("https://files.example.org/releases"),
            Err(PolicyError::DestinationDenied)
        );
        assert_eq!(
            policy.validate("https://files.example.org/releases/../secret"),
            Err(PolicyError::DestinationDenied)
        );
        assert_eq!(
            policy.validate("https://files.example.org/releases%2Fsecret"),
            Err(PolicyError::DestinationDenied)
        );
    }
}
