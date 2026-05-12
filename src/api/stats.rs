use crate::api::error::ApiResult;
use crate::api::AppState;
use axum::{extract::State, response::Json, routing::get, Router};

pub fn routes() -> Router<AppState> {
    Router::new().route("/v1/stats", get(handler))
}

async fn handler(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let v = crate::stats::snapshot::build(
        &state.pool,
        &state.counters,
        state.config.retention_days_hot,
    )
    .await?;
    Ok(Json(v))
}
