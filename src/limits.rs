//! Concurrency permits and request rate limiting.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use ipnet::IpNet;
use thiserror::Error;
use tokio::sync::{Mutex as AsyncMutex, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant as AsyncInstant;

use crate::config::LimitsConfig;

#[derive(Debug, Error)]
#[error("capacity exceeded")]
pub struct CapacityExceeded;

struct HostSlot {
    sem: Arc<Semaphore>,
    max: usize,
}

struct RateBucket {
    tokens: f64,
    last: Instant,
}

struct GlobalRate {
    rate: f64,
    burst: f64,
    bucket: Mutex<RateBucket>,
}

struct PerIpRate {
    rate: f64,
    burst: f64,
    buckets: Mutex<HashMap<IpAddr, RateBucket>>,
}

struct EgressBucket {
    tokens: f64,
    last: AsyncInstant,
}

/// Global egress token bucket shared by every response body.
pub struct EgressLimiter {
    rate: f64,
    capacity: u64,
    bucket: AsyncMutex<EgressBucket>,
}

impl EgressLimiter {
    pub(crate) fn new(bytes_per_second: u64) -> Self {
        let capacity = bytes_per_second.max(64 * 1024);
        Self {
            rate: bytes_per_second as f64,
            capacity,
            bucket: AsyncMutex::new(EgressBucket {
                tokens: capacity as f64,
                last: AsyncInstant::now(),
            }),
        }
    }

    pub fn bytes_per_second(&self) -> u64 {
        self.rate as u64
    }

    /// Wait until the requested number of bytes may be sent. Large chunks are
    /// split so a chunk can exceed the burst capacity without stalling forever.
    pub async fn acquire(&self, bytes: u64) {
        let mut remaining = bytes;
        while remaining > 0 {
            let take = remaining.min(self.capacity);
            self.acquire_up_to(take).await;
            remaining -= take;
        }
    }

    async fn acquire_up_to(&self, bytes: u64) {
        loop {
            let wait = {
                let mut bucket = self.bucket.lock().await;
                let now = AsyncInstant::now();
                let elapsed = now.duration_since(bucket.last).as_secs_f64();
                bucket.tokens = (bucket.tokens + elapsed * self.rate).min(self.capacity as f64);
                bucket.last = now;
                if bucket.tokens >= bytes as f64 {
                    bucket.tokens -= bytes as f64;
                    return;
                }
                let missing = bytes as f64 - bucket.tokens;
                Duration::from_secs_f64(missing / self.rate)
            };
            if wait.is_zero() {
                tokio::task::yield_now().await;
            } else {
                tokio::time::sleep(wait).await;
            }
        }
    }
}

pub struct Limits {
    global: Arc<Semaphore>,
    max_streams: usize,
    per_host: Arc<Mutex<HashMap<String, Arc<HostSlot>>>>,
    per_host_max: usize,
    rate: Option<GlobalRate>,
    per_ip: Option<PerIpRate>,
    egress: Option<Arc<EgressLimiter>>,
    trusted_proxies: Vec<IpNet>,
}

impl std::fmt::Debug for Limits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Limits")
            .field("max_streams", &self.max_streams)
            .field("per_host_max", &self.per_host_max)
            .field("per_ip_enabled", &self.per_ip.is_some())
            .field("egress_enabled", &self.egress.is_some())
            .finish_non_exhaustive()
    }
}

impl Limits {
    pub fn new(config: &LimitsConfig) -> Self {
        let rate = (config.max_requests_per_second > 0).then(|| {
            let burst = f64::from(config.max_requests_per_second.max(1));
            GlobalRate {
                rate: f64::from(config.max_requests_per_second),
                burst,
                bucket: Mutex::new(RateBucket {
                    tokens: burst,
                    last: Instant::now(),
                }),
            }
        });
        let per_ip = (config.max_requests_per_second_per_ip > 0).then(|| {
            let burst = f64::from(config.max_requests_per_second_per_ip.max(1));
            PerIpRate {
                rate: f64::from(config.max_requests_per_second_per_ip),
                burst,
                buckets: Mutex::new(HashMap::new()),
            }
        });
        let egress = (config.max_egress_bytes_per_second > 0)
            .then(|| Arc::new(EgressLimiter::new(config.max_egress_bytes_per_second)));
        let trusted_proxies = config
            .trusted_proxies
            .iter()
            .filter_map(|cidr| cidr.parse::<IpNet>().ok())
            .collect();
        Self {
            global: Arc::new(Semaphore::new(config.max_active_streams)),
            max_streams: config.max_active_streams,
            per_host: Arc::new(Mutex::new(HashMap::new())),
            per_host_max: config.max_active_streams_per_host,
            rate,
            per_ip,
            egress,
            trusted_proxies,
        }
    }

    pub fn acquire_global(&self) -> Result<GlobalPermit, CapacityExceeded> {
        self.global
            .clone()
            .try_acquire_owned()
            .map(GlobalPermit::new)
            .map_err(|_| CapacityExceeded)
    }

    pub fn acquire_host(&self, host: &str) -> Result<HostPermit, CapacityExceeded> {
        let slot = {
            let mut map = self.per_host.lock().unwrap_or_else(|e| e.into_inner());
            map.entry(host.to_string())
                .or_insert_with(|| {
                    Arc::new(HostSlot {
                        sem: Arc::new(Semaphore::new(self.per_host_max)),
                        max: self.per_host_max,
                    })
                })
                .clone()
        };
        let permit = slot
            .sem
            .clone()
            .try_acquire_owned()
            .map_err(|_| CapacityExceeded)?;
        Ok(HostPermit {
            permit: Some(permit),
            slot: Some(slot),
            key: host.to_string(),
            per_host: Arc::downgrade(&self.per_host),
        })
    }

    /// Global rate limit only. Used by unit tests and simple callers.
    pub fn check_rate(&self) -> bool {
        let Some(rate) = &self.rate else {
            return true;
        };
        take_token(&rate.bucket, rate.rate, rate.burst)
    }

    /// Global egress limiter, if configured.
    pub fn egress(&self) -> Option<Arc<EgressLimiter>> {
        self.egress.clone()
    }

    /// Global plus per-client-IP rate limit.
    pub fn check_rate_for(&self, client_ip: IpAddr) -> bool {
        if !self.check_rate() {
            return false;
        }
        let Some(per_ip) = &self.per_ip else {
            return true;
        };
        let now = Instant::now();
        let mut buckets = per_ip.buckets.lock().unwrap_or_else(|e| e.into_inner());
        if buckets.len() > 10_000 {
            buckets.retain(|_, bucket| now.duration_since(bucket.last) < Duration::from_secs(300));
        }
        let bucket = buckets.entry(client_ip).or_insert(RateBucket {
            tokens: per_ip.burst,
            last: now,
        });
        refill(bucket, per_ip.rate, per_ip.burst, now)
    }

    /// Resolve the client IP, using X-Forwarded-For only when the peer is a
    /// configured trusted proxy.
    pub fn client_ip(&self, peer: IpAddr, forwarded_for: Option<&str>) -> IpAddr {
        if self.trusted_proxies.is_empty()
            || !self.trusted_proxies.iter().any(|cidr| cidr.contains(&peer))
        {
            return peer;
        }
        let Some(value) = forwarded_for else {
            return peer;
        };
        let mut fallback = peer;
        for candidate in value.split(',').map(str::trim).rev() {
            let Ok(ip) = candidate.parse::<IpAddr>() else {
                continue;
            };
            fallback = ip;
            if !self.trusted_proxies.iter().any(|cidr| cidr.contains(&ip)) {
                return ip;
            }
        }
        fallback
    }

    /// Number of permits currently held by active responses.
    pub fn active_streams(&self) -> usize {
        self.max_streams
            .saturating_sub(self.global.available_permits())
    }

    pub fn tracked_hosts(&self) -> usize {
        self.per_host
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    pub fn tracked_client_ips(&self) -> usize {
        self.per_ip
            .as_ref()
            .map(|per_ip| {
                per_ip
                    .buckets
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .len()
            })
            .unwrap_or(0)
    }
}

fn refill(bucket: &mut RateBucket, rate: f64, burst: f64, now: Instant) -> bool {
    let elapsed = now.duration_since(bucket.last).as_secs_f64();
    bucket.tokens = (bucket.tokens + elapsed * rate).min(burst);
    bucket.last = now;
    if bucket.tokens >= 1.0 {
        bucket.tokens -= 1.0;
        true
    } else {
        false
    }
}

fn take_token(bucket: &Mutex<RateBucket>, rate: f64, burst: f64) -> bool {
    let mut bucket = bucket.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    refill(&mut bucket, rate, burst, now)
}

pub struct GlobalPermit {
    _permit: OwnedSemaphorePermit,
}

impl GlobalPermit {
    fn new(permit: OwnedSemaphorePermit) -> Self {
        Self { _permit: permit }
    }
}

pub struct HostPermit {
    permit: Option<OwnedSemaphorePermit>,
    slot: Option<Arc<HostSlot>>,
    key: String,
    per_host: Weak<Mutex<HashMap<String, Arc<HostSlot>>>>,
}

impl Drop for HostPermit {
    fn drop(&mut self) {
        self.permit.take();
        let key = std::mem::take(&mut self.key);
        let slot = self.slot.take();
        let map = self.per_host.upgrade();
        drop(slot);
        let Some(map) = map else {
            return;
        };
        let mut map = map.lock().unwrap_or_else(|e| e.into_inner());
        let removable = map
            .get(&key)
            .map(|slot| Arc::strong_count(slot) == 1 && slot.sem.available_permits() == slot.max)
            .unwrap_or(false);
        if removable {
            map.remove(&key);
        }
    }
}

/// Both permits that must live for as long as a response body is being streamed.
pub struct StreamPermits {
    _global: GlobalPermit,
    _host: HostPermit,
}

impl StreamPermits {
    pub fn new(global: GlobalPermit, host: HostPermit) -> Self {
        Self {
            _global: global,
            _host: host,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> LimitsConfig {
        LimitsConfig {
            max_active_streams: 2,
            max_active_streams_per_host: 1,
            max_idle_connections_per_host: 1,
            idle_connection_timeout_secs: 1,
            max_requests_per_second: 0,
            max_requests_per_second_per_ip: 0,
            max_egress_bytes_per_second: 0,
            trusted_proxies: Vec::new(),
        }
    }

    #[test]
    fn global_limit_is_enforced_and_released() {
        let limits = Limits::new(&config());
        let first = limits.acquire_global().unwrap();
        let second = limits.acquire_global().unwrap();
        assert!(limits.acquire_global().is_err());
        assert_eq!(limits.active_streams(), 2);
        drop(first);
        let third = limits.acquire_global().unwrap();
        drop((second, third));
        assert_eq!(limits.active_streams(), 0);
    }

    #[test]
    fn per_host_limit_is_enforced_independently() {
        let limits = Limits::new(&config());
        let a = limits.acquire_host("a.example").unwrap();
        assert!(limits.acquire_host("a.example").is_err());
        let b = limits.acquire_host("b.example").unwrap();
        drop((a, b));
    }

    #[test]
    fn host_slots_are_cleaned_up() {
        let limits = Limits::new(&config());
        {
            let _permit = limits.acquire_host("a.example").unwrap();
            assert_eq!(limits.tracked_hosts(), 1);
        }
        assert_eq!(limits.tracked_hosts(), 0);
    }

    #[test]
    fn rate_limiter_allows_burst_then_rejects() {
        let mut cfg = config();
        cfg.max_requests_per_second = 3;
        let limits = Limits::new(&cfg);
        for _ in 0..3 {
            assert!(limits.check_rate());
        }
        assert!(!limits.check_rate());
    }

    #[test]
    fn disabled_rate_limiter_allows_everything() {
        let limits = Limits::new(&config());
        for _ in 0..100 {
            assert!(limits.check_rate());
        }
    }

    #[test]
    fn per_ip_rate_limit_is_independent() {
        let mut cfg = config();
        cfg.max_requests_per_second_per_ip = 2;
        let limits = Limits::new(&cfg);
        let a: IpAddr = "203.0.113.1".parse().unwrap();
        let b: IpAddr = "203.0.113.2".parse().unwrap();
        assert!(limits.check_rate_for(a));
        assert!(limits.check_rate_for(a));
        assert!(!limits.check_rate_for(a));
        assert!(limits.check_rate_for(b));
        assert_eq!(limits.tracked_client_ips(), 2);
    }

    #[test]
    fn egress_limiter_is_built_from_config() {
        let mut cfg = config();
        cfg.max_egress_bytes_per_second = 4096;
        let limits = Limits::new(&cfg);
        let egress = limits.egress().expect("egress limiter");
        assert_eq!(egress.bytes_per_second(), 4096);
    }

    #[tokio::test(start_paused = true)]
    async fn egress_limiter_shapes_global_bytes() {
        let limiter = EgressLimiter::new(64 * 1024);
        let start = AsyncInstant::now();
        limiter.acquire(64 * 1024).await;
        assert!(start.elapsed() < Duration::from_millis(50));
        limiter.acquire(64 * 1024).await;
        assert!(
            start.elapsed() >= Duration::from_millis(900),
            "elapsed {:?}",
            start.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn high_egress_rate_is_not_capped_by_a_millisecond_floor() {
        let limiter = EgressLimiter::new(100 * 1024 * 1024);
        limiter.acquire(100 * 1024 * 1024).await;
        let start = AsyncInstant::now();
        for _ in 0..100 {
            limiter.acquire(8 * 1024).await;
        }
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "elapsed {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn xff_is_only_used_for_trusted_peers() {
        let mut cfg = config();
        cfg.trusted_proxies = vec!["127.0.0.1/32".to_string()];
        let limits = Limits::new(&cfg);
        let peer: IpAddr = "127.0.0.1".parse().unwrap();
        let client: IpAddr = "203.0.113.7".parse().unwrap();
        assert_eq!(
            limits.client_ip(peer, Some("203.0.113.7, 127.0.0.1")),
            client
        );
        let untrusted: IpAddr = "198.51.100.9".parse().unwrap();
        assert_eq!(limits.client_ip(untrusted, Some("203.0.113.7")), untrusted);
        assert_eq!(limits.client_ip(peer, None), peer);
    }
}
