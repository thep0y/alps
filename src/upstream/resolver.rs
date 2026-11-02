//! Secure DNS resolution: only verified, publicly routable addresses reach the
//! connector.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use thiserror::Error;

use crate::policy::IpPolicy;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Error)]
pub enum ResolveError {
    #[error("dns resolution timed out")]
    Timeout,
    #[error("dns resolution failed: {0}")]
    Io(#[from] io::Error),
    #[error("destination address is not permitted")]
    IpDenied,
    #[error("host is not configured in the static test resolver")]
    NotFound,
}

/// Pluggable DNS backend so tests can simulate rebinding and mixed answers.
pub trait DnsBackend: Send + Sync + 'static {
    fn resolve(
        &self,
        host: String,
        port: u16,
    ) -> Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send>>;
}

#[derive(Debug, Default)]
pub struct GaiBackend;

impl DnsBackend for GaiBackend {
    fn resolve(
        &self,
        host: String,
        port: u16,
    ) -> Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send>> {
        Box::pin(async move {
            let addrs = tokio::net::lookup_host((host.as_str(), port)).await?;
            Ok(addrs.collect())
        })
    }
}

/// Static resolver used by integration tests.
#[cfg(feature = "test-util")]
use std::collections::HashMap;

#[cfg(feature = "test-util")]
#[derive(Debug, Default, Clone)]
pub struct StaticDnsBackend {
    map: HashMap<String, Vec<SocketAddr>>,
}

#[cfg(feature = "test-util")]
impl StaticDnsBackend {
    pub fn new(map: HashMap<String, Vec<SocketAddr>>) -> Self {
        Self { map }
    }
}

#[cfg(feature = "test-util")]
impl DnsBackend for StaticDnsBackend {
    fn resolve(
        &self,
        host: String,
        _port: u16,
    ) -> Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send>> {
        let addrs = self.map.get(&host).cloned();
        Box::pin(async move {
            addrs.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "host not found"))
        })
    }
}

#[derive(Clone)]
pub struct SecureResolver {
    backend: Arc<dyn DnsBackend>,
    policy: IpPolicy,
    timeout: Duration,
}

impl SecureResolver {
    pub fn new(backend: Arc<dyn DnsBackend>, policy: IpPolicy, timeout: Duration) -> Self {
        Self {
            backend,
            policy,
            timeout,
        }
    }

    /// Resolve a host and return only permitted addresses.
    ///
    /// The returned addresses use port 0; reqwest replaces it with the port of
    /// the request URL.
    pub async fn resolve_allowed(&self, host: &str) -> Result<Vec<SocketAddr>, ResolveError> {
        let addrs = tokio::time::timeout(self.timeout, self.backend.resolve(host.to_string(), 0))
            .await
            .map_err(|_| ResolveError::Timeout)??;
        let allowed: Vec<SocketAddr> = addrs
            .into_iter()
            .filter(|addr| self.policy.is_permitted(addr.ip()))
            .collect();
        if allowed.is_empty() {
            return Err(ResolveError::IpDenied);
        }
        Ok(allowed)
    }
}

impl Resolve for SecureResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        let backend = Arc::clone(&self.backend);
        let policy = self.policy;
        let timeout = self.timeout;
        Box::pin(async move {
            let addrs = tokio::time::timeout(timeout, backend.resolve(host, 0))
                .await
                .map_err(|_| Box::new(ResolveError::Timeout) as BoxError)?
                .map_err(|error| Box::new(ResolveError::Io(error)) as BoxError)?;
            let allowed: Vec<SocketAddr> = addrs
                .into_iter()
                .filter(|addr| policy.is_permitted(addr.ip()))
                .collect();
            if allowed.is_empty() {
                return Err(Box::new(ResolveError::IpDenied) as BoxError);
            }
            Ok(Box::new(allowed.into_iter()) as Addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    #[derive(Debug)]
    struct FakeBackend {
        addrs: Vec<SocketAddr>,
        delay: Option<Duration>,
    }

    impl DnsBackend for FakeBackend {
        fn resolve(
            &self,
            _host: String,
            _port: u16,
        ) -> Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send>> {
            let addrs = self.addrs.clone();
            let delay = self.delay;
            Box::pin(async move {
                if let Some(delay) = delay {
                    tokio::time::sleep(delay).await;
                }
                Ok(addrs)
            })
        }
    }

    fn resolver(addrs: &[&str], delay: Option<Duration>) -> SecureResolver {
        let addrs = addrs
            .iter()
            .map(|value| SocketAddr::new(value.parse::<IpAddr>().unwrap(), 0))
            .collect();
        SecureResolver::new(
            Arc::new(FakeBackend { addrs, delay }),
            IpPolicy::strict(),
            Duration::from_millis(200),
        )
    }

    #[tokio::test]
    async fn filters_private_addresses_and_keeps_public_ones() {
        let resolver = resolver(&["127.0.0.1", "10.0.0.1", "8.8.8.8"], None);
        let allowed = resolver.resolve_allowed("example.org").await.unwrap();
        assert_eq!(allowed.len(), 1);
        assert_eq!(allowed[0].ip().to_string(), "8.8.8.8");
    }

    #[tokio::test]
    async fn rejects_when_every_address_is_private() {
        let resolver = resolver(&["127.0.0.1", "192.168.1.1"], None);
        assert!(matches!(
            resolver.resolve_allowed("example.org").await,
            Err(ResolveError::IpDenied)
        ));
    }

    #[tokio::test]
    async fn times_out_slow_resolution() {
        let resolver = resolver(&["8.8.8.8"], Some(Duration::from_secs(5)));
        assert!(matches!(
            resolver.resolve_allowed("example.org").await,
            Err(ResolveError::Timeout)
        ));
    }

    #[cfg(feature = "test-util")]
    #[test]
    fn static_backend_only_allows_loopback_when_permissive() {
        let mut map = HashMap::new();
        map.insert(
            "upstream.test".to_string(),
            vec![SocketAddr::new("127.0.0.1".parse().unwrap(), 0)],
        );
        let resolver = SecureResolver::new(
            Arc::new(StaticDnsBackend::new(map)),
            IpPolicy::permissive_for_tests(),
            Duration::from_secs(1),
        );
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            assert!(resolver.resolve_allowed("upstream.test").await.is_ok());
        });
    }
}
