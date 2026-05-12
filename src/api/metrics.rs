use crate::api::AppState;
use axum::{extract::State, response::IntoResponse, routing::get, Router};

pub fn routes() -> Router<AppState> {
    Router::new().route("/v1/metrics", get(handler))
}

async fn handler(State(state): State<AppState>) -> impl IntoResponse {
    state.metrics_handle.render()
}
