use crate::api::error::ApiResult;
use crate::api::AppState;
use axum::{extract::State, response::Json, routing::get, Router};
use serde_json::json;

pub fn routes() -> Router<AppState> {
    Router::new().route("/v1/logs", get(handler))
}

async fn handler(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let cursors = crate::store::cursors::list(&state.pool).await?;
    let logs: Vec<_> = cursors
        .into_iter()
        .map(|c| {
            json!({
                "log_id": hex::encode(&c.log_id),
                "operator": c.operator,
                "url": c.log_url,
                "state": c.state,
                "cursor": c.last_tree_size,
            })
        })
        .collect();
    Ok(Json(json!({ "logs": logs })))
}
