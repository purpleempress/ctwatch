// src/api/admin.rs
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::post,
    Router,
};
use uuid::Uuid;

use crate::api::error::{ApiError, ApiResult};
use crate::api::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/v1/admin/webhook/:event_id/redeliver", post(redeliver))
}

async fn redeliver(
    Path(event_id): Path<String>,
    State(s): State<AppState>,
) -> ApiResult<StatusCode> {
    let id = Uuid::parse_str(&event_id)
        .map_err(|_| ApiError::BadRequest("event_id must be a UUID".into()))?;
    let found = crate::store::outbox::force_redeliver(&s.pool, id).await?;
    if found {
        Ok(StatusCode::ACCEPTED)
    } else {
        Err(ApiError::NotFound("no such event".into()))
    }
}
