use crate::api::error::{ApiError, ApiResult};
use crate::api::AppState;
use axum::{
    extract::{Query, State},
    response::Json,
    routing::get,
    Router,
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::json;

pub fn routes() -> Router<AppState> {
    Router::new().route("/v1/lookup", get(handler))
}

#[derive(Deserialize)]
struct Params {
    domain: String,
    since: Option<DateTime<Utc>>,
    contains: Option<String>,
    limit: Option<i64>,
}

async fn handler(
    Query(p): Query<Params>,
    State(state): State<AppState>,
) -> ApiResult<Json<serde_json::Value>> {
    let domain = p.domain.trim().to_ascii_lowercase();
    if domain.is_empty() {
        return Err(ApiError::BadRequest("domain required".into()));
    }

    // Normalize input to registered_domain (eTLD+1).
    let regdom = crate::parse::registered_domain(&domain).unwrap_or_else(|| domain.clone());

    let since = p
        .since
        .unwrap_or_else(|| Utc::now() - Duration::days(state.config.retention_days_cold as i64));
    let limit = p.limit.unwrap_or(1000).clamp(1, 10000);

    let rows = crate::store::names::list_by_registered_domain(
        &state.pool,
        &regdom,
        since,
        p.contains.as_deref(),
        limit,
    )
    .await?;

    Ok(Json(json!({
        "domain": domain,
        "registered_domain": regdom,
        "total": rows.len(),
        "names": rows.iter().map(|r| json!({
            "name": r.name,
            "first_seen": r.first_seen,
            "last_seen": r.last_seen,
            "cert_count": r.cert_count,
        })).collect::<Vec<_>>(),
    })))
}
