//! Configuration loading, environment overlays and startup validation.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
};
use serde::Deserialize;
use thiserror::Error;
use tracing_subscriber::EnvFilter;
use url::Host;

/// Top-level configuration file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub proxy: ProxyConfig,
    #[serde(default)]
    pub limits: LimitsConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub signing: SigningConfig,
    #[serde(default)]
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub cors: CorsConfig,
    #[serde(default)]
    pub web: WebConfig,
    #[serde(default)]
    pub allowlist: Vec<AllowlistRule>,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(default = "default_shutdown_grace")]
    pub shutdown_grace_secs: u64,
    #[serde(default = "default_header_read_timeout")]
    pub header_read_timeout_secs: u64,
    #[serde(default = "default_max_header_bytes")]
    pub max_header_bytes: usize,
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
    #[serde(default)]
    pub http2: bool,
    #[serde(default = "default_http2_max_streams")]
    pub http2_max_concurrent_streams: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            shutdown_grace_secs: default_shutdown_grace(),
            header_read_timeout_secs: default_header_read_timeout(),
            max_header_bytes: default_max_header_bytes(),
            max_connections: default_max_connections(),
            http2: false,
            http2_max_concurrent_streams: default_http2_max_streams(),
        }
    }
}

fn default_http2_max_streams() -> u32 {
    100
}

fn default_listen() -> String {
    "0.0.0.0:8080".to_string()
}
fn default_shutdown_grace() -> u64 {
    20
}
fn default_header_read_timeout() -> u64 {
    10
}
fn default_max_header_bytes() -> usize {
    16 * 1024
}
fn default_max_connections() -> usize {
    512
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyConfig {
    #[serde(default)]
    pub allow_http: bool,
    #[serde(default = "default_max_redirects")]
    pub max_redirects: usize,
    #[serde(default = "default_max_url_length")]
    pub max_url_length: usize,
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: u64,
    #[serde(default = "default_header_timeout")]
    pub response_header_timeout_secs: u64,
    #[serde(default = "default_read_timeout")]
    pub read_idle_timeout_secs: u64,
    #[serde(default)]
    pub max_response_bytes: u64,
    #[serde(default)]
    pub forward_accept: bool,
    #[serde(default = "default_user_agent")]
    pub user_agent: String,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            allow_http: false,
            max_redirects: default_max_redirects(),
            max_url_length: default_max_url_length(),
            connect_timeout_secs: default_connect_timeout(),
            response_header_timeout_secs: default_header_timeout(),
            read_idle_timeout_secs: default_read_timeout(),
            max_response_bytes: 0,
            forward_accept: false,
            user_agent: default_user_agent(),
        }
    }
}

fn default_max_redirects() -> usize {
    5
}
fn default_max_url_length() -> usize {
    8192
}
fn default_connect_timeout() -> u64 {
    5
}
fn default_header_timeout() -> u64 {
    15
}
fn default_read_timeout() -> u64 {
    30
}
fn default_user_agent() -> String {
    format!("alps/{}", env!("CARGO_PKG_VERSION"))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsConfig {
    #[serde(default = "default_max_active_streams")]
    pub max_active_streams: usize,
    #[serde(default = "default_max_active_streams_per_host")]
    pub max_active_streams_per_host: usize,
    #[serde(default = "default_max_idle_per_host")]
    pub max_idle_connections_per_host: usize,
    #[serde(default = "default_idle_connection_timeout")]
    pub idle_connection_timeout_secs: u64,
    #[serde(default)]
    pub max_requests_per_second: u32,
    #[serde(default)]
    pub max_requests_per_second_per_ip: u32,
    /// Total egress bytes per second across all response streams. 0 disables.
    #[serde(default)]
    pub max_egress_bytes_per_second: u64,
    #[serde(default)]
    pub trusted_proxies: Vec<String>,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_active_streams: default_max_active_streams(),
            max_active_streams_per_host: default_max_active_streams_per_host(),
            max_idle_connections_per_host: default_max_idle_per_host(),
            idle_connection_timeout_secs: default_idle_connection_timeout(),
            max_requests_per_second: 0,
            max_requests_per_second_per_ip: 0,
            max_egress_bytes_per_second: 0,
            trusted_proxies: Vec::new(),
        }
    }
}

fn default_max_active_streams() -> usize {
    100
}
fn default_max_active_streams_per_host() -> usize {
    20
}
fn default_max_idle_per_host() -> usize {
    3
}
fn default_idle_connection_timeout() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityConfig {
    #[serde(default = "default_true")]
    pub block_non_global_ips: bool,
    #[serde(default = "default_true")]
    pub block_ip_literal_hosts: bool,
    #[serde(default = "default_true")]
    pub no_system_proxy: bool,
    #[serde(default = "default_content_disposition")]
    pub default_content_disposition: String,
    #[serde(default)]
    pub trusted_network_only: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            block_non_global_ips: true,
            block_ip_literal_hosts: true,
            no_system_proxy: true,
            default_content_disposition: default_content_disposition(),
            trusted_network_only: false,
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_content_disposition() -> String {
    "attachment".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    #[default]
    Bearer,
    None,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    #[serde(default)]
    pub mode: AuthMode,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowlistRule {
    pub host: String,
    #[serde(default)]
    pub include_subdomains: bool,
    #[serde(default = "default_schemes")]
    pub schemes: Vec<String>,
    #[serde(default = "default_ports")]
    pub ports: Vec<u16>,
    /// Optional literal path prefixes. Empty means every path on the host.
    #[serde(default)]
    pub path_prefixes: Vec<String>,
}

fn default_schemes() -> Vec<String> {
    vec!["https".to_string()]
}
fn default_ports() -> Vec<u16> {
    vec![443]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    #[default]
    Json,
    Text,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default)]
    pub format: LogFormat,
    #[serde(default = "default_true")]
    pub redact_url_query: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: LogFormat::Json,
            redact_url_query: true,
        }
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SigningConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_signing_ttl")]
    pub default_ttl_secs: u64,
    #[serde(default = "default_signing_max_ttl")]
    pub max_ttl_secs: u64,
    #[serde(default = "default_signing_max_token")]
    pub max_token_bytes: usize,
}

impl Default for SigningConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_ttl_secs: default_signing_ttl(),
            max_ttl_secs: default_signing_max_ttl(),
            max_token_bytes: default_signing_max_token(),
        }
    }
}

fn default_signing_ttl() -> u64 {
    300
}
fn default_signing_max_ttl() -> u64 {
    86_400
}
fn default_signing_max_token() -> usize {
    4096
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_metrics_listen")]
    pub listen: String,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: default_metrics_listen(),
        }
    }
}

fn default_metrics_listen() -> String {
    "127.0.0.1:9090".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorsConfig {
    /// Exact origins, e.g. "https://app.example.org". Empty disables CORS.
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    #[serde(default = "default_cors_max_age")]
    pub max_age_secs: u64,
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            allowed_origins: Vec::new(),
            max_age_secs: default_cors_max_age(),
        }
    }
}

fn default_cors_max_age() -> u64 {
    600
}

impl CorsConfig {
    pub fn allows(&self, origin: &str) -> bool {
        self.allowed_origins.iter().any(|allowed| allowed == origin)
    }
}

/// Optional public web UI served directly by ALPS.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct WebConfig {
    /// Directory containing the built UI (index.html plus its assets).
    ///
    /// When set, GET/HEAD /?url=... becomes a public, unauthenticated proxy
    /// endpoint and the JSON API routes are hidden unless expose_api is true.
    #[serde(default)]
    pub dir: Option<PathBuf>,
    /// Mount /v1/proxy, /v1/sign and /p/{token} on the public listener.
    #[serde(default)]
    pub expose_api: bool,
}

impl WebConfig {
    pub fn enabled(&self) -> bool {
        self.dir.is_some()
    }

    /// True when the UI is public and the JSON API is not mounted.
    pub fn hides_api(&self) -> bool {
        self.enabled() && !self.expose_api
    }
}

/// Configuration plus secrets sourced from the environment.
#[derive(Clone)]
pub struct LoadedConfig {
    pub config: Config,
    pub api_token: Option<String>,
    /// Current secret first, previous (rotation) secret second.
    pub signing_secrets: Vec<Vec<u8>>,
}

impl std::fmt::Debug for LoadedConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedConfig")
            .field("config", &self.config)
            .field("api_token", &self.api_token.as_ref().map(|_| "<redacted>"))
            .field("signing_secrets", &self.signing_secrets.len())
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read configuration {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid TOML configuration: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("invalid configuration field '{field}': {message}")]
    Invalid { field: String, message: String },
}

fn invalid(field: &str, message: impl Into<String>) -> ConfigError {
    ConfigError::Invalid {
        field: field.to_string(),
        message: message.into(),
    }
}

impl Config {
    /// Load and validate configuration from the given path, applying environment overlays.
    pub fn load(path: &Path) -> Result<LoadedConfig, ConfigError> {
        Self::load_with(path, None)
    }

    /// Load with an optional static UI override (highest precedence).
    ///
    /// The precedence is CLI `--static-dir`, then the `ALPS_STATIC_DIR`
    /// environment variable, then `web.dir` in the TOML file.
    pub fn load_with(
        path: &Path,
        static_dir: Option<PathBuf>,
    ) -> Result<LoadedConfig, ConfigError> {
        let raw = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.display().to_string(),
            source,
        })?;
        let mut config: Config = toml::from_str(&raw)?;
        if let Some(dir) = static_dir.or_else(|| {
            std::env::var_os("ALPS_STATIC_DIR")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        }) {
            config.web.dir = Some(dir);
        }
        let api_token = std::env::var("ALPS_API_TOKEN")
            .ok()
            .filter(|value| !value.is_empty());
        let signing_secrets = if config.signing.enabled {
            read_signing_secrets()?
        } else {
            Vec::new()
        };
        config.validate(api_token.as_deref())?;
        config.validate_signing(&signing_secrets)?;
        Ok(LoadedConfig {
            config,
            api_token,
            signing_secrets,
        })
    }

    /// True when the bearer token must be configured and checked.
    ///
    /// A public static UI hides the JSON API by default, so no protected route
    /// is mounted and the token is not required.
    pub fn requires_bearer_token(&self) -> bool {
        self.auth.mode == AuthMode::Bearer && !self.web.hides_api()
    }

    /// Validate and normalize the configuration in place.
    pub fn validate(&mut self, api_token: Option<&str>) -> Result<(), ConfigError> {
        let listen: SocketAddr = self.server.listen.parse().map_err(|_| {
            invalid(
                "server.listen",
                "expected a socket address like 0.0.0.0:8080",
            )
        })?;
        let metrics_listen: SocketAddr = self.metrics.listen.parse().map_err(|_| {
            invalid(
                "metrics.listen",
                "expected a socket address like 127.0.0.1:9090",
            )
        })?;
        if self.metrics.enabled && metrics_listen == listen {
            return Err(invalid("metrics.listen", "must differ from server.listen"));
        }
        check_range(
            "server.shutdown_grace_secs",
            self.server.shutdown_grace_secs,
            1,
            300,
        )?;
        check_range(
            "server.header_read_timeout_secs",
            self.server.header_read_timeout_secs,
            1,
            60,
        )?;
        check_range(
            "server.max_header_bytes",
            self.server.max_header_bytes as u64,
            8192,
            65536,
        )?;
        check_range(
            "server.max_connections",
            self.server.max_connections as u64,
            1,
            65535,
        )?;
        check_range(
            "server.http2_max_concurrent_streams",
            self.server.http2_max_concurrent_streams as u64,
            1,
            1000,
        )?;

        check_range(
            "proxy.max_redirects",
            self.proxy.max_redirects as u64,
            0,
            10,
        )?;
        check_range(
            "proxy.max_url_length",
            self.proxy.max_url_length as u64,
            128,
            65536,
        )?;
        check_range(
            "proxy.connect_timeout_secs",
            self.proxy.connect_timeout_secs,
            1,
            300,
        )?;
        check_range(
            "proxy.response_header_timeout_secs",
            self.proxy.response_header_timeout_secs,
            1,
            300,
        )?;
        check_range(
            "proxy.read_idle_timeout_secs",
            self.proxy.read_idle_timeout_secs,
            1,
            300,
        )?;
        if self.proxy.max_response_bytes > 0 && self.proxy.max_response_bytes < 1024 {
            return Err(invalid(
                "proxy.max_response_bytes",
                "must be 0 (unlimited) or at least 1024",
            ));
        }
        validate_token("proxy.user_agent", &self.proxy.user_agent, 256)?;

        check_range(
            "limits.max_active_streams",
            self.limits.max_active_streams as u64,
            1,
            1_000_000,
        )?;
        check_range(
            "limits.max_active_streams_per_host",
            self.limits.max_active_streams_per_host as u64,
            1,
            1_000_000,
        )?;
        if self.limits.max_active_streams_per_host > self.limits.max_active_streams {
            return Err(invalid(
                "limits.max_active_streams_per_host",
                "must not exceed limits.max_active_streams",
            ));
        }
        check_range(
            "limits.max_idle_connections_per_host",
            self.limits.max_idle_connections_per_host as u64,
            1,
            64,
        )?;
        check_range(
            "limits.idle_connection_timeout_secs",
            self.limits.idle_connection_timeout_secs,
            1,
            600,
        )?;
        check_range(
            "limits.max_requests_per_second",
            self.limits.max_requests_per_second as u64,
            0,
            1_000_000,
        )?;
        check_range(
            "limits.max_requests_per_second_per_ip",
            self.limits.max_requests_per_second_per_ip as u64,
            0,
            1_000_000,
        )?;
        check_range(
            "limits.max_egress_bytes_per_second",
            self.limits.max_egress_bytes_per_second,
            0,
            10_000_000_000,
        )?;
        for (index, cidr) in self.limits.trusted_proxies.iter().enumerate() {
            if cidr.parse::<ipnet::IpNet>().is_err() {
                return Err(invalid(
                    &format!("limits.trusted_proxies[{index}]"),
                    "expected an IP or CIDR such as 127.0.0.1/32",
                ));
            }
        }

        check_range(
            "signing.default_ttl_secs",
            self.signing.default_ttl_secs,
            1,
            31_536_000,
        )?;
        check_range(
            "signing.max_ttl_secs",
            self.signing.max_ttl_secs,
            1,
            31_536_000,
        )?;
        if self.signing.default_ttl_secs > self.signing.max_ttl_secs {
            return Err(invalid(
                "signing.default_ttl_secs",
                "must not exceed signing.max_ttl_secs",
            ));
        }
        check_range(
            "signing.max_token_bytes",
            self.signing.max_token_bytes as u64,
            512,
            16384,
        )?;

        if !self.security.block_non_global_ips {
            return Err(invalid(
                "security.block_non_global_ips",
                "this safety switch is fixed to true in this release",
            ));
        }
        if !self.security.block_ip_literal_hosts {
            return Err(invalid(
                "security.block_ip_literal_hosts",
                "this safety switch is fixed to true in this release",
            ));
        }
        if !self.security.no_system_proxy {
            return Err(invalid(
                "security.no_system_proxy",
                "this safety switch is fixed to true in this release",
            ));
        }
        if self.security.default_content_disposition != "attachment" {
            return Err(invalid(
                "security.default_content_disposition",
                "only 'attachment' is supported in this release",
            ));
        }
        if !self.logging.redact_url_query {
            return Err(invalid(
                "logging.redact_url_query",
                "URL query redaction cannot be disabled",
            ));
        }
        if EnvFilter::try_new(&self.logging.level).is_err() {
            return Err(invalid("logging.level", "not a valid tracing filter"));
        }

        if self.web.expose_api && !self.web.enabled() {
            return Err(invalid("web.expose_api", "web.expose_api requires web.dir"));
        }
        if let Some(dir) = self.web.dir.as_mut() {
            if dir.as_os_str().is_empty() {
                return Err(invalid("web.dir", "must not be empty"));
            }
            let metadata = std::fs::metadata(&*dir).map_err(|source| {
                invalid(
                    "web.dir",
                    format!("cannot access {}: {source}", dir.display()),
                )
            })?;
            if !metadata.is_dir() {
                return Err(invalid(
                    "web.dir",
                    format!("{} is not a directory", dir.display()),
                ));
            }
            let index = dir.join("index.html");
            if !index.is_file() {
                return Err(invalid(
                    "web.dir",
                    format!("{} does not contain index.html", dir.display()),
                ));
            }
            *dir = dir.canonicalize().map_err(|source| {
                invalid(
                    "web.dir",
                    format!("cannot canonicalize {}: {source}", dir.display()),
                )
            })?;
        }

        match self.auth.mode {
            AuthMode::Bearer => {
                // With the static UI enabled and the JSON API hidden, no
                // bearer-protected route is mounted. The public /?url= route
                // is bounded by the allowlist and does not use this token.
                if !self.web.hides_api() {
                    let token = api_token.ok_or_else(|| {
                        invalid(
                            "auth.mode",
                            "bearer mode requires the ALPS_API_TOKEN environment variable",
                        )
                    })?;
                    if token.len() < 16 {
                        return Err(invalid(
                            "auth.mode",
                            "ALPS_API_TOKEN must be at least 16 bytes long",
                        ));
                    }
                }
            }
            AuthMode::None => {
                if !listen.ip().is_loopback()
                    && !self.security.trusted_network_only
                    && !self.web.hides_api()
                {
                    return Err(invalid(
                        "auth.mode",
                        "auth.mode = 'none' on a non-loopback listener requires security.trusted_network_only = true (or web.dir with web.expose_api = false)",
                    ));
                }
            }
        }

        if self.cors.allowed_origins.len() > 64 {
            return Err(invalid(
                "cors.allowed_origins",
                "at most 64 origins are supported",
            ));
        }
        for (index, origin) in self.cors.allowed_origins.iter_mut().enumerate() {
            let field = format!("cors.allowed_origins[{index}]");
            let parsed = url::Url::parse(origin)
                .map_err(|_| invalid(&field, "expected an origin like https://app.example.org"))?;
            let valid = matches!(parsed.scheme(), "http" | "https")
                && parsed.host().is_some()
                && parsed.username().is_empty()
                && parsed.password().is_none()
                && parsed.path() == "/"
                && parsed.query().is_none()
                && parsed.fragment().is_none();
            if !valid {
                return Err(invalid(
                    &field,
                    "origin must contain only scheme, host and optional port",
                ));
            }
            *origin = parsed.origin().ascii_serialization();
        }
        self.cors.allowed_origins.sort();
        self.cors.allowed_origins.dedup();
        check_range("cors.max_age_secs", self.cors.max_age_secs, 0, 86_400)?;

        if self.allowlist.is_empty() {
            tracing::warn!("allowlist is empty: every proxy request will be denied");
        }
        for (index, rule) in self.allowlist.iter_mut().enumerate() {
            let field = format!("allowlist[{index}]");
            rule.host = normalize_host(&rule.host)
                .ok_or_else(|| invalid(&format!("{field}.host"), "expected a DNS domain name"))?;

            if rule.schemes.is_empty() {
                return Err(invalid(&format!("{field}.schemes"), "must not be empty"));
            }
            rule.schemes.sort();
            rule.schemes.dedup();
            for scheme in &rule.schemes {
                match scheme.as_str() {
                    "https" => {}
                    "http" => {
                        if !self.proxy.allow_http {
                            return Err(invalid(
                                &format!("{field}.schemes"),
                                "http requires proxy.allow_http = true",
                            ));
                        }
                    }
                    other => {
                        return Err(invalid(
                            &format!("{field}.schemes"),
                            format!("unsupported scheme '{other}'"),
                        ));
                    }
                }
            }

            if rule.ports.is_empty() {
                return Err(invalid(&format!("{field}.ports"), "must not be empty"));
            }
            rule.ports.sort_unstable();
            rule.ports.dedup();
            if rule.ports.contains(&0) {
                return Err(invalid(&format!("{field}.ports"), "port 0 is not usable"));
            }
            if rule.path_prefixes.len() > 16 {
                return Err(invalid(
                    &format!("{field}.path_prefixes"),
                    "at most 16 path prefixes are supported",
                ));
            }
            for (path_index, prefix) in rule.path_prefixes.iter_mut().enumerate() {
                if !valid_path_prefix(prefix) {
                    return Err(invalid(
                        &format!("{field}.path_prefixes[{path_index}]"),
                        "must start with '/', contain no control/space, '?' or '#', no '.'/'..' segments, and be at most 1024 bytes",
                    ));
                }
            }
            rule.path_prefixes.sort();
            rule.path_prefixes.dedup();
        }

        Ok(())
    }

    /// Validate secrets for signed links (called by load after env decoding).
    pub fn validate_signing(&self, secrets: &[Vec<u8>]) -> Result<(), ConfigError> {
        if self.signing.enabled && secrets.is_empty() {
            return Err(invalid(
                "signing.enabled",
                "requires ALPS_SIGNING_SECRET (base64 or at least 32 raw bytes)",
            ));
        }
        for secret in secrets {
            if secret.len() < 32 {
                return Err(invalid(
                    "ALPS_SIGNING_SECRET",
                    "must decode to at least 32 bytes",
                ));
            }
        }
        if !self.signing.enabled && !secrets.is_empty() {
            tracing::warn!("signing secrets are set but signing.enabled = false");
        }
        Ok(())
    }

    pub fn listen_addr(&self) -> SocketAddr {
        self.server
            .listen
            .parse()
            .expect("configuration was validated before use")
    }

    /// Short, secret-free summary used by --check-config and startup logs.
    pub fn summary(&self) -> String {
        format!(
            "listen={} allowlist_rules={} auth_mode={:?} signing={} metrics={} http2={} cors_origins={} web_static={} web_api={} max_active_streams={} max_redirects={}",
            self.server.listen,
            self.allowlist.len(),
            self.auth.mode,
            self.signing.enabled,
            self.metrics.enabled,
            self.server.http2,
            self.cors.allowed_origins.len(),
            self.web.enabled(),
            self.web.expose_api,
            self.limits.max_active_streams,
            self.proxy.max_redirects,
        )
    }
}

impl MetricsConfig {
    pub fn listen_addr(&self) -> SocketAddr {
        self.listen
            .parse()
            .expect("configuration was validated before use")
    }
}

impl ProxyConfig {
    pub fn connect_timeout(&self) -> Duration {
        Duration::from_secs(self.connect_timeout_secs)
    }
    pub fn response_header_timeout(&self) -> Duration {
        Duration::from_secs(self.response_header_timeout_secs)
    }
    pub fn read_idle_timeout(&self) -> Duration {
        Duration::from_secs(self.read_idle_timeout_secs)
    }
}

impl ServerConfig {
    pub fn shutdown_grace(&self) -> Duration {
        Duration::from_secs(self.shutdown_grace_secs)
    }
    pub fn header_read_timeout(&self) -> Duration {
        Duration::from_secs(self.header_read_timeout_secs)
    }
}

impl LimitsConfig {
    pub fn idle_connection_timeout(&self) -> Duration {
        Duration::from_secs(self.idle_connection_timeout_secs)
    }
}

fn check_range<T>(field: &str, value: T, min: T, max: T) -> Result<(), ConfigError>
where
    T: PartialOrd + Copy + std::fmt::Display,
{
    if value < min || value > max {
        return Err(invalid(
            field,
            format!("must be between {min} and {max}, got {value}"),
        ));
    }
    Ok(())
}

fn read_signing_secrets() -> Result<Vec<Vec<u8>>, ConfigError> {
    let current = read_env_secret("ALPS_SIGNING_SECRET")?;
    let previous = read_env_secret("ALPS_SIGNING_SECRET_PREVIOUS")?;
    match (current, previous) {
        (Some(current), previous) => {
            let mut secrets = vec![current];
            if let Some(previous) = previous {
                secrets.push(previous);
            }
            Ok(secrets)
        }
        (None, Some(_)) => Err(invalid(
            "ALPS_SIGNING_SECRET",
            "ALPS_SIGNING_SECRET_PREVIOUS is set without a current secret",
        )),
        (None, None) => Ok(Vec::new()),
    }
}

fn read_env_secret(name: &str) -> Result<Option<Vec<u8>>, ConfigError> {
    let Ok(value) = std::env::var(name) else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    decode_secret(&value)
        .map(Some)
        .ok_or_else(|| invalid(name, "must be base64 or at least 32 raw bytes"))
}

fn decode_secret(value: &str) -> Option<Vec<u8>> {
    for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD] {
        if let Ok(bytes) = engine.decode(value)
            && bytes.len() >= 32
        {
            return Some(bytes);
        }
    }
    let raw = value.as_bytes();
    (raw.len() >= 32).then(|| raw.to_vec())
}

fn valid_path_prefix(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && value.starts_with('/')
        && !value.contains('?')
        && !value.contains('#')
        && !value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        && !value
            .split('/')
            .any(|segment| segment == "." || segment == "..")
}

fn validate_token(field: &str, value: &str, max_len: usize) -> Result<(), ConfigError> {
    if value.is_empty() || value.len() > max_len {
        return Err(invalid(field, format!("length must be 1..={max_len}")));
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(invalid(field, "must not contain control characters"));
    }
    Ok(())
}

/// Normalize a DNS name used in an allowlist rule.
pub fn normalize_host(host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('.');
    if host.is_empty() || host.contains('*') || host.contains('/') || host.contains('@') {
        return None;
    }
    match Host::parse(host).ok()? {
        Host::Domain(domain) => {
            let domain = domain.trim_end_matches('.').to_ascii_lowercase();
            if domain.is_empty()
                || domain.len() > 253
                || domain
                    .split('.')
                    .any(|label| label.is_empty() || label.len() > 63)
            {
                None
            } else {
                Some(domain)
            }
        }
        Host::Ipv4(_) | Host::Ipv6(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Config {
        let mut cfg: Config = toml::from_str(
            r#"
[[allowlist]]
host = "Files.Example.ORG."
schemes = ["https"]
ports = [443]
"#,
        )
        .unwrap();
        cfg.validate(Some("0123456789abcdef")).unwrap();
        cfg
    }

    struct WebDir(PathBuf);

    impl WebDir {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let index = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("alps-web-test-{}-{index}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("index.html"), "<!doctype html>").unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for WebDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn normalizes_rule_hosts() {
        let cfg = base();
        assert_eq!(cfg.allowlist[0].host, "files.example.org");
    }

    #[test]
    fn rejects_unknown_fields() {
        let err = toml::from_str::<Config>("[server]\nlisten = \"127.0.0.1:8080\"\nbogus = 1\n")
            .unwrap_err();
        assert!(err.to_string().contains("bogus"));
    }

    #[test]
    fn rejects_disabled_security_switches() {
        let mut cfg = base();
        cfg.security.block_non_global_ips = false;
        assert!(cfg.validate(Some("0123456789abcdef")).is_err());
    }

    #[test]
    fn rejects_http_rule_without_allow_http() {
        let mut cfg = base();
        cfg.allowlist[0].schemes = vec!["http".into()];
        assert!(cfg.validate(Some("0123456789abcdef")).is_err());
    }

    #[test]
    fn bearer_requires_token() {
        let mut cfg = base();
        assert!(cfg.validate(None).is_err());
    }

    #[test]
    fn none_auth_requires_trusted_network_for_public_bind() {
        let mut cfg = base();
        cfg.auth.mode = AuthMode::None;
        cfg.server.listen = "0.0.0.0:8080".into();
        assert!(cfg.validate(None).is_err());
        cfg.security.trusted_network_only = true;
        assert!(cfg.validate(None).is_ok());
    }

    #[test]
    fn web_public_mode_allows_none_auth_and_skips_the_bearer_token() {
        let dir = WebDir::new();
        let mut cfg = base();
        cfg.server.listen = "0.0.0.0:8080".into();
        cfg.web.dir = Some(dir.path().to_path_buf());

        cfg.auth.mode = AuthMode::None;
        cfg.validate(None).unwrap();

        cfg.auth.mode = AuthMode::Bearer;
        cfg.validate(None).unwrap();
        assert!(!cfg.requires_bearer_token());
        assert!(cfg.summary().contains("web_static=true"));
    }

    #[test]
    fn web_expose_api_keeps_the_normal_auth_rules() {
        let dir = WebDir::new();
        let mut cfg = base();
        cfg.server.listen = "0.0.0.0:8080".into();
        cfg.web.dir = Some(dir.path().to_path_buf());
        cfg.web.expose_api = true;

        cfg.auth.mode = AuthMode::None;
        assert!(cfg.validate(None).is_err());
        cfg.security.trusted_network_only = true;
        assert!(cfg.validate(None).is_ok());

        cfg.auth.mode = AuthMode::Bearer;
        assert!(cfg.validate(None).is_err());
        assert!(cfg.validate(Some("0123456789abcdef")).is_ok());
        assert!(cfg.requires_bearer_token());
    }

    #[test]
    fn web_expose_api_requires_a_directory() {
        let mut cfg = base();
        cfg.web.expose_api = true;
        assert!(cfg.validate(Some("0123456789abcdef")).is_err());
    }

    #[test]
    fn web_directory_requires_index_html() {
        let dir = WebDir::new();
        std::fs::remove_file(dir.path().join("index.html")).unwrap();
        let mut cfg = base();
        cfg.web.dir = Some(dir.path().to_path_buf());
        assert!(cfg.validate(Some("0123456789abcdef")).is_err());
    }

    #[test]
    fn web_directory_is_canonicalized() {
        let dir = WebDir::new();
        let mut cfg = base();
        cfg.web.dir = Some(dir.path().to_path_buf());
        cfg.validate(Some("0123456789abcdef")).unwrap();
        assert!(cfg.web.dir.as_ref().is_some_and(|path| path.is_absolute()));
    }

    #[test]
    fn rejects_ip_literal_rules() {
        let mut cfg = base();
        cfg.allowlist[0].host = "127.0.0.1".into();
        assert!(cfg.validate(Some("0123456789abcdef")).is_err());
    }

    #[test]
    fn rejects_redaction_disabled() {
        let mut cfg = base();
        cfg.logging.redact_url_query = false;
        assert!(cfg.validate(Some("0123456789abcdef")).is_err());
    }

    #[test]
    fn signing_requires_a_secret() {
        let mut cfg = base();
        cfg.signing.enabled = true;
        assert!(cfg.validate_signing(&[]).is_err());
        assert!(cfg.validate_signing(&[vec![7u8; 32]]).is_ok());
    }

    #[test]
    fn rejects_invalid_trusted_proxy() {
        let mut cfg = base();
        cfg.limits.trusted_proxies = vec!["not-a-cidr".to_string()];
        assert!(cfg.validate(Some("0123456789abcdef")).is_err());
    }

    #[test]
    fn metrics_listener_must_differ_from_main_listener() {
        let mut cfg = base();
        cfg.server.listen = "127.0.0.1:9090".to_string();
        cfg.metrics.enabled = true;
        cfg.metrics.listen = "127.0.0.1:9090".to_string();
        assert!(cfg.validate(Some("0123456789abcdef")).is_err());
    }

    #[test]
    fn path_prefixes_are_validated() {
        let mut cfg = base();
        cfg.allowlist[0].path_prefixes = vec!["releases".to_string()];
        assert!(cfg.validate(Some("0123456789abcdef")).is_err());

        cfg.allowlist[0].path_prefixes = vec!["/releases/".to_string()];
        assert!(cfg.validate(Some("0123456789abcdef")).is_ok());
    }

    #[test]
    fn cors_origins_are_normalized() {
        let mut cfg = base();
        cfg.cors.allowed_origins = vec!["HTTPS://App.Example.ORG:443/".to_string()];
        cfg.validate(Some("0123456789abcdef")).unwrap();
        assert_eq!(cfg.cors.allowed_origins, vec!["https://app.example.org"]);

        cfg.cors.allowed_origins = vec!["https://app.example.org/path".to_string()];
        assert!(cfg.validate(Some("0123456789abcdef")).is_err());
        cfg.cors.allowed_origins = vec!["*".to_string()];
        assert!(cfg.validate(Some("0123456789abcdef")).is_err());
    }

    #[test]
    fn egress_limit_is_validated() {
        let mut cfg = base();
        cfg.limits.max_egress_bytes_per_second = 1;
        assert!(cfg.validate(Some("0123456789abcdef")).is_ok());
        cfg.limits.max_egress_bytes_per_second = 10_000_000_001;
        assert!(cfg.validate(Some("0123456789abcdef")).is_err());
    }

    #[test]
    fn http2_stream_limit_is_validated() {
        let mut cfg = base();
        cfg.server.http2 = true;
        cfg.server.http2_max_concurrent_streams = 0;
        assert!(cfg.validate(Some("0123456789abcdef")).is_err());
        cfg.server.http2_max_concurrent_streams = 100;
        assert!(cfg.validate(Some("0123456789abcdef")).is_ok());
    }

    #[test]
    fn signing_ttls_are_validated() {
        let mut cfg = base();
        cfg.signing.max_ttl_secs = 10;
        cfg.signing.default_ttl_secs = 20;
        assert!(cfg.validate(Some("0123456789abcdef")).is_err());
    }

    #[test]
    fn defaults_are_applied_for_minimal_rule() {
        let mut cfg: Config = toml::from_str(
            r#"
[[allowlist]]
host = "example.org"
"#,
        )
        .unwrap();
        assert_eq!(cfg.allowlist[0].schemes, vec!["https"]);
        assert_eq!(cfg.allowlist[0].ports, vec![443]);
        assert!(cfg.security.block_non_global_ips);
        assert!(cfg.security.block_ip_literal_hosts);
        assert!(cfg.security.no_system_proxy);
        assert_eq!(cfg.auth.mode, AuthMode::Bearer);
        assert_eq!(cfg.logging.format, LogFormat::Json);
        assert!(!cfg.server.http2);
        assert_eq!(cfg.limits.max_egress_bytes_per_second, 0);
        cfg.validate(Some("0123456789abcdef")).unwrap();
    }

    #[test]
    fn rejects_out_of_range_and_invalid_fields() {
        let cases: Vec<fn(&mut Config)> = vec![
            |c| c.server.listen = "not-an-address".into(),
            |c| c.metrics.listen = "not-an-address".into(),
            |c| c.server.shutdown_grace_secs = 0,
            |c| c.server.header_read_timeout_secs = 0,
            |c| c.server.max_header_bytes = 1,
            |c| c.server.max_connections = 0,
            |c| c.server.http2_max_concurrent_streams = 0,
            |c| c.proxy.max_redirects = 11,
            |c| c.proxy.max_url_length = 1,
            |c| c.proxy.connect_timeout_secs = 0,
            |c| c.proxy.response_header_timeout_secs = 0,
            |c| c.proxy.read_idle_timeout_secs = 0,
            |c| c.proxy.max_response_bytes = 1,
            |c| c.proxy.user_agent.clear(),
            |c| c.limits.max_active_streams = 0,
            |c| c.limits.max_active_streams_per_host = 0,
            |c| {
                c.limits.max_active_streams = 1;
                c.limits.max_active_streams_per_host = 2;
            },
            |c| c.limits.max_idle_connections_per_host = 0,
            |c| c.limits.idle_connection_timeout_secs = 0,
            |c| c.limits.max_requests_per_second = 1_000_001,
            |c| c.limits.max_requests_per_second_per_ip = 1_000_001,
            |c| c.limits.max_egress_bytes_per_second = 10_000_000_001,
            |c| c.limits.trusted_proxies = vec!["bad-cidr".into()],
            |c| c.signing.default_ttl_secs = 0,
            |c| c.signing.max_ttl_secs = 0,
            |c| {
                c.signing.default_ttl_secs = 20;
                c.signing.max_ttl_secs = 10;
            },
            |c| c.signing.max_token_bytes = 1,
            |c| c.security.block_non_global_ips = false,
            |c| c.security.block_ip_literal_hosts = false,
            |c| c.security.no_system_proxy = false,
            |c| c.security.default_content_disposition = "inline".into(),
            |c| c.logging.redact_url_query = false,
            |c| c.logging.level = "[".into(),
            |c| c.cors.allowed_origins = vec!["https://a.example".into(); 65],
            |c| c.cors.max_age_secs = 86_401,
            |c| c.allowlist[0].host = "bad host".into(),
            |c| c.allowlist[0].schemes.clear(),
            |c| c.allowlist[0].schemes = vec!["ftp".into()],
            |c| c.allowlist[0].ports.clear(),
            |c| c.allowlist[0].ports = vec![0],
            |c| c.allowlist[0].path_prefixes = vec!["/x/".into(); 17],
        ];
        for (index, mutate) in cases.into_iter().enumerate() {
            let mut cfg = base();
            mutate(&mut cfg);
            assert!(
                cfg.validate(Some("0123456789abcdef")).is_err(),
                "case {index} should be rejected"
            );
        }
    }

    #[test]
    fn short_bearer_token_is_rejected() {
        let mut cfg = base();
        assert!(cfg.validate(Some("short")).is_err());
    }

    #[test]
    fn empty_allowlist_is_allowed_with_warning() {
        let mut cfg = base();
        cfg.allowlist.clear();
        cfg.validate(Some("0123456789abcdef")).unwrap();
    }

    #[test]
    fn validate_signing_rejects_short_secret_and_warns_when_disabled() {
        let mut cfg = base();
        cfg.signing.enabled = true;
        assert!(cfg.validate_signing(&[vec![7u8; 31]]).is_err());

        cfg.signing.enabled = false;
        assert!(cfg.validate_signing(&[vec![7u8; 32]]).is_ok());
    }

    #[test]
    fn loaded_config_debug_redacts_secrets() {
        let loaded = LoadedConfig {
            config: base(),
            api_token: Some("super-secret-token".into()),
            signing_secrets: vec![vec![7u8; 32]],
        };
        let text = format!("{loaded:?}");
        assert!(text.contains("<redacted>"));
        assert!(!text.contains("super-secret-token"));
    }

    #[test]
    fn load_reports_missing_file() {
        let path = std::path::Path::new("/nonexistent/alps-test-config.toml");
        let error = Config::load(path).unwrap_err();
        assert!(matches!(error, ConfigError::Read { .. }));
    }

    #[test]
    fn decode_secret_accepts_base64_and_raw_bytes() {
        let encoded = STANDARD.encode([7u8; 32]);
        assert_eq!(decode_secret(&encoded).unwrap().len(), 32);
        let raw = "x".repeat(32);
        assert_eq!(decode_secret(&raw).unwrap().len(), 32);
        assert!(decode_secret("too-short").is_none());
    }

    #[test]
    fn normalize_host_rejects_malformed_names() {
        for host in ["", "*.example.org", "a/b", "a@b", "a..b", "127.0.0.1"] {
            assert!(normalize_host(host).is_none(), "{host}");
        }
        let long_label = "a".repeat(64);
        assert!(normalize_host(&format!("{long_label}.example.org")).is_none());
        assert_eq!(
            normalize_host("EXAMPLE.ORG.").as_deref(),
            Some("example.org")
        );
    }

    #[test]
    fn accessors_and_summary_work() {
        let mut cfg = base();
        cfg.metrics.enabled = true;
        cfg.metrics.listen = "127.0.0.1:9091".into();
        assert_eq!(cfg.listen_addr(), "0.0.0.0:8080".parse().unwrap());
        assert_eq!(cfg.metrics.listen_addr(), "127.0.0.1:9091".parse().unwrap());
        assert_eq!(cfg.proxy.connect_timeout().as_secs(), 5);
        assert_eq!(cfg.proxy.response_header_timeout().as_secs(), 15);
        assert_eq!(cfg.proxy.read_idle_timeout().as_secs(), 30);
        assert_eq!(cfg.server.shutdown_grace().as_secs(), 20);
        assert_eq!(cfg.server.header_read_timeout().as_secs(), 10);
        assert_eq!(cfg.limits.idle_connection_timeout().as_secs(), 30);
        assert!(!cfg.cors.allows("https://app.example.org"));
        assert!(cfg.summary().contains("allowlist_rules=1"));
    }
}
