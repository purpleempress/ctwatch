use axum::{routing::get, Router};

use crate::api::AppState;
use crate::stream::handler::upgrade;

pub fn routes() -> Router<AppState> {
    Router::new().route("/v1/stream", get(upgrade))
}
