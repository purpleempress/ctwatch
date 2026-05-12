// Mock the CT log over a real TCP listener using axum (already a dep) instead of
// pulling in wiremock 0.6.5, which requires edition 2024.
use axum::{routing::get, Json, Router};
use ctwatch::ingest::client::CtClient;
use serde_json::json;
use tokio::net::TcpListener;

#[tokio::test]
async fn get_sth_parses_signed_tree_head() {
    let app = Router::new().route(
        "/ct/v1/get-sth",
        get(|| async {
            Json(json!({
                "tree_size": 12345_u64,
                "timestamp": 1700000000000_u64,
                "sha256_root_hash": "AAAA",
                "tree_head_signature": "BBBB"
            }))
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let base_url = format!("http://{addr}/");
    let client = CtClient::new();
    let sth = client.get_sth(&base_url).await.expect("ok");
    assert_eq!(sth.tree_size, 12345);
    assert_eq!(sth.timestamp_ms, 1700000000000);
}
