//! Response body bridging with byte accounting and permit lifetime.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use axum::body::Body;
use bytes::Bytes;
use futures_util::Stream;
use thiserror::Error;

use crate::limits::{EgressLimiter, StreamPermits};

#[derive(Debug, Error)]
pub enum StreamError {
    #[error("upstream response exceeded the configured byte limit")]
    TooLarge,
    #[error("upstream body stream failed")]
    Upstream,
}

/// Owns the access-log state for one proxied response. Logging happens when the
/// body finishes, fails, or is dropped because the client went away.
pub struct StreamObserver {
    request_id: String,
    method: String,
    host: String,
    status: u16,
    hops: u8,
    started: Instant,
    bytes: u64,
    outcome: &'static str,
    finished: bool,
}

impl StreamObserver {
    pub fn new(
        request_id: String,
        method: String,
        host: String,
        status: u16,
        hops: u8,
        started: Instant,
    ) -> Self {
        Self {
            request_id,
            method,
            host,
            status,
            hops,
            started,
            bytes: 0,
            outcome: "incomplete",
            finished: false,
        }
    }

    pub fn record(&mut self, bytes: usize) {
        self.bytes = self.bytes.saturating_add(bytes as u64);
    }

    pub fn finish(&mut self, outcome: &'static str) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.outcome = outcome;
        let metrics = crate::metrics::global();
        metrics.record_request(self.status);
        if self.bytes > 0 {
            metrics.record_bytes(self.bytes);
        }
        if self.outcome != "complete" {
            metrics.record_stream_failure(self.outcome);
        }
        tracing::info!(
            request_id = %self.request_id,
            method = %self.method,
            host = %self.host,
            status = self.status,
            hops = self.hops,
            bytes_sent = self.bytes,
            duration_ms = self.started.elapsed().as_millis() as u64,
            outcome = self.outcome,
            "request finished"
        );
    }
}

impl Drop for StreamObserver {
    fn drop(&mut self) {
        self.finish("client_disconnect");
    }
}

/// Bridges the reqwest body into an axum Body while keeping the permits and the
/// observer alive for exactly as long as the body is alive.
pub struct GuardedStream {
    inner: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
    permits: Option<StreamPermits>,
    observer: StreamObserver,
    sent: u64,
    limit: u64,
    throttle: Option<Arc<EgressLimiter>>,
    pending: Option<Bytes>,
    wait: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl GuardedStream {
    pub fn new(
        inner: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
        permits: StreamPermits,
        limit: u64,
        observer: StreamObserver,
        throttle: Option<Arc<EgressLimiter>>,
    ) -> Self {
        Self {
            inner,
            permits: Some(permits),
            observer,
            sent: 0,
            limit,
            throttle,
            pending: None,
            wait: None,
        }
    }
}

impl Stream for GuardedStream {
    type Item = Result<Bytes, StreamError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if let Some(chunk) = this.pending.take() {
                let next_sent = this.sent.saturating_add(chunk.len() as u64);
                if this.limit > 0 && next_sent > this.limit {
                    this.observer.finish("too_large");
                    this.permits.take();
                    return Poll::Ready(Some(Err(StreamError::TooLarge)));
                }
                if let Some(limiter) = this.throttle.clone() {
                    if this.wait.is_none() {
                        let bytes = chunk.len() as u64;
                        this.wait = Some(Box::pin(async move {
                            limiter.acquire(bytes).await;
                        }));
                    }
                    let ready = this.wait.as_mut().expect("wait future").as_mut().poll(cx);
                    match ready {
                        Poll::Ready(()) => this.wait = None,
                        Poll::Pending => {
                            this.pending = Some(chunk);
                            return Poll::Pending;
                        }
                    }
                }
                this.sent = next_sent;
                this.observer.record(chunk.len());
                return Poll::Ready(Some(Ok(chunk)));
            }

            match this.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(chunk))) => this.pending = Some(chunk),
                Poll::Ready(Some(Err(error))) => {
                    let error = error.without_url();
                    tracing::warn!(error = %error, "upstream body stream failed");
                    this.observer.finish("upstream_error");
                    this.permits.take();
                    this.pending = None;
                    this.wait = None;
                    return Poll::Ready(Some(Err(StreamError::Upstream)));
                }
                Poll::Ready(None) => {
                    this.observer.finish("complete");
                    this.permits.take();
                    this.pending = None;
                    this.wait = None;
                    return Poll::Ready(None);
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

pub fn body_from_upstream(
    response: reqwest::Response,
    permits: StreamPermits,
    limit: u64,
    observer: StreamObserver,
    throttle: Option<Arc<EgressLimiter>>,
) -> Body {
    let stream = GuardedStream::new(
        Box::pin(response.bytes_stream()),
        permits,
        limit,
        observer,
        throttle,
    );
    Body::from_stream(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    use crate::config::LimitsConfig;
    use crate::limits::Limits;

    fn permits() -> StreamPermits {
        let limits = Limits::new(&LimitsConfig {
            max_active_streams: 1,
            max_active_streams_per_host: 1,
            max_idle_connections_per_host: 1,
            idle_connection_timeout_secs: 1,
            max_requests_per_second: 0,
            max_requests_per_second_per_ip: 0,
            max_egress_bytes_per_second: 0,
            trusted_proxies: Vec::new(),
        });
        let global = limits.acquire_global().unwrap();
        let host = limits.acquire_host("example.org").unwrap();
        StreamPermits::new(global, host)
    }

    fn observer() -> StreamObserver {
        StreamObserver::new(
            "req-1".into(),
            "GET".into(),
            "example.org".into(),
            200,
            0,
            Instant::now(),
        )
    }

    fn boxed(
        chunks: Vec<Result<Bytes, std::io::Error>>,
    ) -> Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>> {
        // Convert io errors into a reqwest error is impossible directly, so use
        // only Ok chunks in this helper.
        assert!(chunks.iter().all(Result::is_ok));
        let items: Vec<reqwest::Result<Bytes>> =
            chunks.into_iter().map(|chunk| Ok(chunk.unwrap())).collect();
        Box::pin(futures_util::stream::iter(items))
    }

    #[tokio::test]
    async fn streams_chunks_and_finishes() {
        let mut stream = GuardedStream::new(
            boxed(vec![
                Ok(Bytes::from_static(b"abc")),
                Ok(Bytes::from_static(b"def")),
            ]),
            permits(),
            0,
            observer(),
            None,
        );
        assert_eq!(
            stream.next().await.unwrap().unwrap(),
            Bytes::from_static(b"abc")
        );
        assert_eq!(
            stream.next().await.unwrap().unwrap(),
            Bytes::from_static(b"def")
        );
        assert!(stream.next().await.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn throttles_large_chunks() {
        let limiter = Arc::new(EgressLimiter::new(64 * 1024));
        let mut stream = GuardedStream::new(
            boxed(vec![Ok(Bytes::from(vec![0u8; 128 * 1024]))]),
            permits(),
            0,
            observer(),
            Some(limiter),
        );
        let start = tokio::time::Instant::now();
        let chunk = stream.next().await.unwrap().unwrap();
        assert_eq!(chunk.len(), 128 * 1024);
        assert!(
            start.elapsed() >= std::time::Duration::from_millis(900),
            "elapsed {:?}",
            start.elapsed()
        );
    }

    #[tokio::test]
    async fn enforces_byte_limit() {
        let mut stream = GuardedStream::new(
            boxed(vec![Ok(Bytes::from_static(b"abcdef"))]),
            permits(),
            3,
            observer(),
            None,
        );
        assert!(stream.next().await.unwrap().is_err());
    }
}
