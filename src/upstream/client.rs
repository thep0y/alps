//! Shared upstream HTTP client construction.

use std::sync::Arc;

use crate::config::Config;
use crate::upstream::resolver::SecureResolver;

/// Build the single shared upstream client.
pub fn build(
    config: &Config,
    resolver: Arc<SecureResolver>,
) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .no_gzip()
        .no_brotli()
        .no_zstd()
        .no_deflate()
        .referer(false)
        .connect_timeout(config.proxy.connect_timeout())
        .read_timeout(config.proxy.read_idle_timeout())
        .pool_max_idle_per_host(config.limits.max_idle_connections_per_host)
        .pool_idle_timeout(config.limits.idle_connection_timeout())
        .dns_resolver(resolver)
        .user_agent(config.proxy.user_agent.clone())
        .build()
}
