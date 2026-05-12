use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use ctwatch::api::{router, AppState};
use ctwatch::stats::Counters;
use ctwatch::store::{self, names};
use http_body_util::BodyExt;
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;

async fn make_state(pool: PgPool) -> AppState {
    store::migrate(&pool).await.unwrap();
    let (stream_tx, _rx) = ctwatch::stream::channel(64);
    // Use build_recorder() instead of install_recorder() to avoid conflicts when
    // multiple tests run in parallel (install_recorder panics on re-installation).
    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let metrics_handle = recorder.handle();
    let cfg = ctwatch::config::Config {
        database_url: "unused".into(),
        listen_addr: "0.0.0.0:0".into(),
        log_list_url: "".into(),
        log_operators: "*".into(),
        log_poll_interval: std::time::Duration::from_secs(30),
        log_entry_batch_size: 1000,
        retention_days_hot: 90,
        retention_days_cold: 400,
        watchlist_mode: "db".into(),
        stream_enabled: true,
        stream_max_lag: 64,
        stream_max_subscribers: 100,
        crtsh_base_url: "https://crt.sh".into(),
        metrics_enabled: true,
        log_level: "info".into(),
        webhook_url: None,
        webhook_hmac_secret: None,
        webhook_max_attempts: 7,
        watchlist_file: None,
        watchlist_url: None,
        watchlist_refresh: std::time::Duration::from_secs(60),
    };
    AppState {
        pool,
        counters: Counters::new(),
        metrics_handle,
        config: Arc::new(cfg),
        stream_tx,
    }
}

#[sqlx::test]
async fn lookup_returns_seeded_names(pool: PgPool) {
    let state = make_state(pool.clone()).await;
    let now = Utc::now();
    names::upsert(&pool, "api.example.com", "example.com", now)
        .await
        .unwrap();
    names::upsert(&pool, "www.example.com", "example.com", now)
        .await
        .unwrap();

    let app = router(state);
    let req = Request::builder()
        .uri("/v1/lookup?domain=example.com")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["registered_domain"], "example.com");
    assert_eq!(v["total"], 2);
}

use chrono::Duration as ChDur;
use ctwatch::store::certs::{self, CertRow};

#[sqlx::test]
async fn certs_endpoint_returns_seeded_cert(pool: PgPool) {
    let state = make_state(pool.clone()).await;
    let cert = CertRow {
        cert_hash: vec![0x11; 32],
        issuer_hash: vec![0x22; 32],
        issuer_cn: "Let's Encrypt R10".into(),
        not_before: Utc::now() - ChDur::hours(1),
        not_after: Utc::now() + ChDur::days(90),
        sans: vec!["www.example.com".into()],
        registered_domains: vec!["example.com".into()],
    };
    certs::insert_batch(&pool, &[cert]).await.unwrap();

    let app = router(state);
    let req = Request::builder()
        .uri("/v1/certs?domain=example.com")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["total"], 1);
    assert_eq!(v["certs"][0]["issuer_cn"], "Let's Encrypt R10");
}

#[sqlx::test]
async fn cert_by_hash_404_when_missing(pool: PgPool) {
    let state = make_state(pool).await;
    let app = router(state);
    let req = Request::builder()
        .uri("/v1/cert/00aa")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn stats_returns_stable_shape(pool: PgPool) {
    let state = make_state(pool).await;
    let app = router(state);
    let req = Request::builder()
        .uri("/v1/stats")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    for key in &["totals", "ingest", "stream", "logs", "backfill"] {
        assert!(v.get(*key).is_some(), "missing {key}");
    }
    for key in &[
        "certs_in_window",
        "unique_names_400d",
        "watchlist_size",
        "webhook_outbox_pending",
    ] {
        assert!(v["totals"].get(*key).is_some(), "missing totals.{key}");
    }
}

#[sqlx::test]
async fn watchlist_crud_roundtrip(pool: PgPool) {
    let state = make_state(pool).await;
    let app = router(state);

    let add_req = Request::builder()
        .uri("/v1/watchlist")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"domain":"foo.example.com"}"#))
        .unwrap();
    let resp = app.clone().oneshot(add_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        v["domain"], "example.com",
        "should normalize to registered_domain"
    );

    let list_req = Request::builder()
        .uri("/v1/watchlist")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(list_req).await.unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["domains"][0], "example.com");

    let del_req = Request::builder()
        .uri("/v1/watchlist/example.com")
        .method("DELETE")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(del_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}
