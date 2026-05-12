use crate::api::error::{ApiError, ApiResult};
use crate::api::AppState;
use axum::{
    extract::{Path, Query, State},
    response::Json,
    routing::get,
    Router,
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::json;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/certs", get(list))
        .route("/v1/cert/:hash_hex", get(one))
        .route("/v1/cert/:hash_hex/raw", get(raw))
        .route("/v1/cert/:hash_hex/observations", get(observations))
}

#[derive(Deserialize)]
struct ListParams {
    domain: String,
    since: Option<DateTime<Utc>>,
    limit: Option<i64>,
}

async fn list(
    Query(p): Query<ListParams>,
    State(state): State<AppState>,
) -> ApiResult<Json<serde_json::Value>> {
    let regdom =
        crate::parse::registered_domain(&p.domain).unwrap_or_else(|| p.domain.to_ascii_lowercase());
    let since = p
        .since
        .unwrap_or_else(|| Utc::now() - Duration::days(state.config.retention_days_hot as i64));
    let limit = p.limit.unwrap_or(500).clamp(1, 5000);

    let rows =
        crate::store::certs::list_by_registered_domain(&state.pool, &regdom, since, limit).await?;
    Ok(Json(json!({
        "domain": regdom,
        "total": rows.len(),
        "certs": rows.iter().map(serialize_cert).collect::<Vec<_>>(),
    })))
}

async fn one(
    Path(hash_hex): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<Json<serde_json::Value>> {
    let hash =
        hex::decode(&hash_hex).map_err(|_| ApiError::BadRequest("hash_hex must be hex".into()))?;
    let row = crate::store::certs::get_one(&state.pool, &hash)
        .await?
        .ok_or_else(|| ApiError::NotFound("cert not in window".into()))?;
    Ok(Json(serialize_cert(&row)))
}

fn serialize_cert(r: &crate::store::certs::CertRow) -> serde_json::Value {
    json!({
        "cert_hash": hex::encode(&r.cert_hash),
        "issuer_cn": r.issuer_cn,
        "not_before": r.not_before,
        "not_after": r.not_after,
        "sans": r.sans,
        "registered_domains": r.registered_domains,
    })
}

async fn raw(
    Path(hash_hex): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<axum::response::Response> {
    let _hash =
        hex::decode(&hash_hex).map_err(|_| ApiError::BadRequest("hash_hex must be hex".into()))?;
    let url = format!("{}/?d={hash_hex}", state.config.crtsh_base_url);
    let resp = reqwest::get(&url)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))?;
    if !resp.status().is_success() {
        return Err(ApiError::Internal(anyhow::anyhow!(
            "crt.sh status {}",
            resp.status()
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))?;
    Ok(axum::response::Response::builder()
        .status(200)
        .header("Content-Type", "application/pkix-cert")
        .body(axum::body::Body::from(bytes))
        .unwrap())
}

async fn observations(
    Path(hash_hex): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<axum::response::Response> {
    let _hash =
        hex::decode(&hash_hex).map_err(|_| ApiError::BadRequest("hash_hex must be hex".into()))?;
    let url = format!("{}/?q={hash_hex}&output=json", state.config.crtsh_base_url);
    let resp = reqwest::get(&url)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))?;
    if !resp.status().is_success() {
        return Err(ApiError::Internal(anyhow::anyhow!(
            "crt.sh status {}",
            resp.status()
        )));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))?;
    Ok(axum::response::Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(body))
        .unwrap())
}
