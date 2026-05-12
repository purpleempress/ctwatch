use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Default)]
struct RingBuf {
    samples: Vec<(Instant, u64)>,
    capacity: usize,
}

impl RingBuf {
    fn new(capacity: usize) -> Self {
        Self {
            samples: Vec::with_capacity(capacity),
            capacity,
        }
    }

    fn push(&mut self, value: u64) {
        let now = Instant::now();
        if self.samples.len() == self.capacity {
            self.samples.remove(0);
        }
        self.samples.push((now, value));
    }

    fn rate_per_sec(&self, window: Duration) -> f64 {
        let now = Instant::now();
        let cutoff = now.checked_sub(window).unwrap_or(now);
        let in_window: u64 = self
            .samples
            .iter()
            .filter(|(t, _)| *t >= cutoff)
            .map(|(_, v)| *v)
            .sum();
        in_window as f64 / window.as_secs_f64()
    }
}

#[derive(Clone)]
pub struct Counters {
    inner: Arc<Inner>,
}

struct Inner {
    started_at: Instant,
    pub certs_ingested: AtomicU64,
    pub duplicates_dropped: AtomicU64,
    pub watchlist_matches: AtomicU64,
    pub writer_queue_depth: AtomicI64,
    pub stream_subscribers: AtomicI64,
    pub stream_messages_sent: AtomicU64,
    pub stream_subscribers_dropped: AtomicU64,
    pub webhook_delivered: AtomicU64,
    pub webhook_retry: AtomicU64,
    pub webhook_failed: AtomicU64,

    // ring buffers for rate calc; pushed once per second by a tick task
    rb_certs: RwLock<RingBuf>,
    rb_dups: RwLock<RingBuf>,
    rb_stream_msgs: RwLock<RingBuf>,
}

impl Counters {
    pub fn new() -> Self {
        let inner = Inner {
            started_at: Instant::now(),
            certs_ingested: 0.into(),
            duplicates_dropped: 0.into(),
            watchlist_matches: 0.into(),
            writer_queue_depth: 0.into(),
            stream_subscribers: 0.into(),
            stream_messages_sent: 0.into(),
            stream_subscribers_dropped: 0.into(),
            webhook_delivered: 0.into(),
            webhook_retry: 0.into(),
            webhook_failed: 0.into(),
            rb_certs: RwLock::new(RingBuf::new(3600)),
            rb_dups: RwLock::new(RingBuf::new(3600)),
            rb_stream_msgs: RwLock::new(RingBuf::new(3600)),
        };
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn incr_certs(&self, n: u64) {
        self.inner.certs_ingested.fetch_add(n, Ordering::Relaxed);
    }
    pub fn incr_dups(&self, n: u64) {
        self.inner
            .duplicates_dropped
            .fetch_add(n, Ordering::Relaxed);
    }
    pub fn incr_watchlist(&self) {
        self.inner.watchlist_matches.fetch_add(1, Ordering::Relaxed);
    }
    pub fn set_queue_depth(&self, d: i64) {
        self.inner.writer_queue_depth.store(d, Ordering::Relaxed);
    }
    pub fn set_subscribers(&self, n: i64) {
        self.inner.stream_subscribers.store(n, Ordering::Relaxed);
    }
    pub fn incr_stream_sent(&self, n: u64) {
        self.inner
            .stream_messages_sent
            .fetch_add(n, Ordering::Relaxed);
    }
    pub fn incr_stream_dropped(&self) {
        self.inner
            .stream_subscribers_dropped
            .fetch_add(1, Ordering::Relaxed);
    }
    pub fn incr_webhook_delivered(&self) {
        self.inner.webhook_delivered.fetch_add(1, Ordering::Relaxed);
    }
    pub fn incr_webhook_retry(&self) {
        self.inner.webhook_retry.fetch_add(1, Ordering::Relaxed);
    }
    pub fn incr_webhook_failed(&self) {
        self.inner.webhook_failed.fetch_add(1, Ordering::Relaxed);
    }
    pub fn webhook_delivered(&self) -> u64 {
        self.inner.webhook_delivered.load(Ordering::Relaxed)
    }
    pub fn webhook_retry(&self) -> u64 {
        self.inner.webhook_retry.load(Ordering::Relaxed)
    }
    pub fn webhook_failed(&self) -> u64 {
        self.inner.webhook_failed.load(Ordering::Relaxed)
    }

    pub fn started_at(&self) -> Instant {
        self.inner.started_at
    }
    pub fn certs(&self) -> u64 {
        self.inner.certs_ingested.load(Ordering::Relaxed)
    }
    pub fn dups(&self) -> u64 {
        self.inner.duplicates_dropped.load(Ordering::Relaxed)
    }
    pub fn watchlist_hits(&self) -> u64 {
        self.inner.watchlist_matches.load(Ordering::Relaxed)
    }
    pub fn queue_depth(&self) -> i64 {
        self.inner.writer_queue_depth.load(Ordering::Relaxed)
    }
    pub fn subscribers(&self) -> i64 {
        self.inner.stream_subscribers.load(Ordering::Relaxed)
    }
    pub fn stream_sent(&self) -> u64 {
        self.inner.stream_messages_sent.load(Ordering::Relaxed)
    }
    pub fn stream_dropped(&self) -> u64 {
        self.inner
            .stream_subscribers_dropped
            .load(Ordering::Relaxed)
    }

    pub async fn rate_certs(&self, window: Duration) -> f64 {
        self.inner.rb_certs.read().await.rate_per_sec(window)
    }
    pub async fn rate_dups(&self, window: Duration) -> f64 {
        self.inner.rb_dups.read().await.rate_per_sec(window)
    }
    pub async fn rate_stream(&self, window: Duration) -> f64 {
        self.inner.rb_stream_msgs.read().await.rate_per_sec(window)
    }

    /// Spawn a background task that snapshots delta counters every second into the rate ring buffers.
    pub fn spawn_ticker(&self) {
        let this = self.clone();
        tokio::spawn(async move {
            let mut last_certs = 0u64;
            let mut last_dups = 0u64;
            let mut last_stream = 0u64;
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                let c = this.certs();
                let d = this.dups();
                let s = this.stream_sent();
                this.inner
                    .rb_certs
                    .write()
                    .await
                    .push(c.saturating_sub(last_certs));
                this.inner
                    .rb_dups
                    .write()
                    .await
                    .push(d.saturating_sub(last_dups));
                this.inner
                    .rb_stream_msgs
                    .write()
                    .await
                    .push(s.saturating_sub(last_stream));
                last_certs = c;
                last_dups = d;
                last_stream = s;
            }
        });
    }
}

impl Default for Counters {
    fn default() -> Self {
        Self::new()
    }
}

pub mod snapshot;
