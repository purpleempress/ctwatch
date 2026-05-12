use chrono::{Duration, Utc};
use ctwatch::store::{
    self,
    certs::{self, CertRow},
    cursors, names, outbox, watchlist,
};
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test]
async fn cursors_upsert_then_advance(pool: PgPool) {
    store::migrate(&pool).await.unwrap();
    let log_id = vec![1u8; 32];
    cursors::upsert(&pool, &log_id, "https://ct.example/log/", "Example")
        .await
        .unwrap();
    cursors::advance(&pool, &log_id, 100).await.unwrap();
    cursors::advance(&pool, &log_id, 50).await.unwrap(); // older — should be ignored
    let list = cursors::list(&pool).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].last_tree_size, 100);
}

#[sqlx::test]
async fn certs_insert_dedup_and_lookup(pool: PgPool) {
    store::migrate(&pool).await.unwrap();
    let row = CertRow {
        cert_hash: vec![0xABu8; 32],
        issuer_hash: vec![0xCDu8; 32],
        issuer_cn: "Let's Encrypt R10".into(),
        not_before: Utc::now() - Duration::hours(1),
        not_after: Utc::now() + Duration::days(90),
        sans: vec!["www.example.com".into(), "example.com".into()],
        registered_domains: vec!["example.com".into()],
    };
    let n = certs::insert_batch(&pool, &[row.clone()]).await.unwrap();
    assert_eq!(n, 1);
    let n2 = certs::insert_batch(&pool, &[row.clone()]).await.unwrap();
    assert_eq!(n2, 0, "duplicate should be skipped");

    let by_domain =
        certs::list_by_registered_domain(&pool, "example.com", Utc::now() - Duration::days(1), 100)
            .await
            .unwrap();
    assert_eq!(by_domain.len(), 1);
    assert_eq!(by_domain[0].cert_hash, vec![0xABu8; 32]);
}

#[sqlx::test]
async fn names_upsert_increments_count_and_advances_last_seen(pool: PgPool) {
    store::migrate(&pool).await.unwrap();
    let now = Utc::now();
    names::upsert(&pool, "api.example.com", "example.com", now)
        .await
        .unwrap();
    names::upsert(
        &pool,
        "api.example.com",
        "example.com",
        now + Duration::hours(1),
    )
    .await
    .unwrap();
    let list =
        names::list_by_registered_domain(&pool, "example.com", now - Duration::days(1), None, 100)
            .await
            .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].cert_count, 2);
}

#[sqlx::test]
async fn watchlist_add_idempotent(pool: PgPool) {
    store::migrate(&pool).await.unwrap();
    assert!(watchlist::add(&pool, "example.com", None).await.unwrap());
    assert!(!watchlist::add(&pool, "example.com", None).await.unwrap());
    assert_eq!(watchlist::list(&pool).await.unwrap(), vec!["example.com"]);
    assert!(watchlist::remove(&pool, "example.com").await.unwrap());
}

#[sqlx::test]
async fn outbox_enqueue_then_count(pool: PgPool) {
    store::migrate(&pool).await.unwrap();
    let _e = outbox::enqueue(&pool, &json!({"hello": "world"}))
        .await
        .unwrap();
    assert_eq!(outbox::pending_count(&pool).await.unwrap(), 1);
}
