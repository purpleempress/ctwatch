use axum::Router;
use sqlx::PgPool;
use std::sync::Arc;

pub mod admin;
pub mod certs;
pub mod error;
pub mod health;
pub mod logs;
pub mod lookup;
pub mod metrics;
pub mod stats;
pub mod stream;
pub mod watchlist;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub counters: crate::stats::Counters,
    pub metrics_handle: metrics_exporter_prometheus::PrometheusHandle,
    pub config: Arc<crate::config::Config>,
    pub stream_tx: crate::stream::CertEventSender,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(health::routes())
        .merge(metrics::routes())
        .merge(logs::routes())
        .merge(lookup::routes())
        .merge(certs::routes())
        .merge(stats::routes())
        .merge(watchlist::routes())
        .merge(admin::routes())
        .merge(stream::routes())
        .with_state(state)
}
