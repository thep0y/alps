//! Prometheus metrics with a bounded label set.

use std::sync::OnceLock;

use prometheus::{
    Encoder, Histogram, HistogramOpts, IntCounter, IntCounterVec, IntGauge, Opts, Registry,
    TextEncoder,
};

pub struct Metrics {
    registry: Registry,
    requests: IntCounterVec,
    policy_denied: IntCounterVec,
    redirects: IntCounter,
    bytes_sent: IntCounter,
    stream_failures: IntCounterVec,
    active_streams: IntGauge,
    upstream_duration: Histogram,
}

impl Metrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();
        let requests = IntCounterVec::new(
            Opts::new(
                "alps_requests_total",
                "Proxy requests by response status class",
            ),
            &["status_class"],
        )?;
        let policy_denied = IntCounterVec::new(
            Opts::new(
                "alps_policy_denied_total",
                "Requests rejected by policy or token checks",
            ),
            &["reason"],
        )?;
        let redirects = IntCounter::new("alps_redirects_total", "Upstream redirect hops followed")?;
        let bytes_sent = IntCounter::new(
            "alps_bytes_sent_total",
            "Response body bytes sent to clients",
        )?;
        let stream_failures = IntCounterVec::new(
            Opts::new(
                "alps_stream_failures_total",
                "Response streams that ended abnormally",
            ),
            &["reason"],
        )?;
        let active_streams = IntGauge::new(
            "alps_active_streams",
            "Currently active proxied response streams",
        )?;
        let upstream_duration = Histogram::with_opts(
            HistogramOpts::new(
                "alps_upstream_duration_seconds",
                "Time spent in the upstream fetch including redirects",
            )
            .buckets(vec![
                0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0,
            ]),
        )?;

        registry.register(Box::new(requests.clone()))?;
        registry.register(Box::new(policy_denied.clone()))?;
        registry.register(Box::new(redirects.clone()))?;
        registry.register(Box::new(bytes_sent.clone()))?;
        registry.register(Box::new(stream_failures.clone()))?;
        registry.register(Box::new(active_streams.clone()))?;
        registry.register(Box::new(upstream_duration.clone()))?;

        Ok(Self {
            registry,
            requests,
            policy_denied,
            redirects,
            bytes_sent,
            stream_failures,
            active_streams,
            upstream_duration,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let encoder = TextEncoder::new();
        let mut buffer = Vec::new();
        encoder.encode(&self.registry.gather(), &mut buffer)?;
        Ok(buffer)
    }

    pub fn record_request(&self, status: u16) {
        self.requests
            .with_label_values(&[status_class(status)])
            .inc();
    }

    pub fn record_policy_denied(&self, reason: &str) {
        self.policy_denied.with_label_values(&[reason]).inc();
    }

    pub fn record_redirect(&self) {
        self.redirects.inc();
    }

    pub fn record_bytes(&self, bytes: u64) {
        self.bytes_sent.inc_by(bytes);
    }

    pub fn record_stream_failure(&self, reason: &str) {
        self.stream_failures.with_label_values(&[reason]).inc();
    }

    pub fn set_active_streams(&self, value: usize) {
        self.active_streams.set(value as i64);
    }

    pub fn observe_upstream(&self, seconds: f64) {
        self.upstream_duration.observe(seconds);
    }
}

fn status_class(status: u16) -> &'static str {
    match status / 100 {
        1 => "1xx",
        2 => "2xx",
        3 => "3xx",
        4 => "4xx",
        5 => "5xx",
        _ => "other",
    }
}

static METRICS: OnceLock<Metrics> = OnceLock::new();

/// Process-wide registry. Metrics survive configuration reloads.
pub fn global() -> &'static Metrics {
    METRICS.get_or_init(|| Metrics::new().expect("static Prometheus registry must be valid"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_classes_are_bounded() {
        assert_eq!(status_class(200), "2xx");
        assert_eq!(status_class(304), "3xx");
        assert_eq!(status_class(404), "4xx");
        assert_eq!(status_class(503), "5xx");
        assert_eq!(status_class(99), "other");
    }

    #[test]
    fn encodes_text_exposition() {
        global().record_request(200);
        global().record_policy_denied("destination_denied");
        global().set_active_streams(0);
        let body = String::from_utf8(global().encode().unwrap()).unwrap();
        assert!(body.contains("alps_requests_total"));
        assert!(body.contains("alps_active_streams"));
        assert!(body.contains("alps_policy_denied_total"));
    }
}
