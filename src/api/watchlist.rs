use crate::api::error::{ApiError, ApiResult};
use crate::api::AppState;
use axum::http::StatusCode;
use axum::{
    extract::{Path, State},
    response::Json,
    routing::{delete, get},
    Router,
};
use serde::Deserialize;
use serde_json::json;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/watchlist", get(list).post(add))
        .route("/v1/watchlist/:domain", delete(remove))
}

#[derive(Deserialize)]
struct AddBody {
    domain: String,
    notes: Option<String>,
}

async fn list(State(s): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    if s.config.watchlist_mode != "db" {
        return Err(ApiError::BadRequest("watchlist not in db mode".into()));
    }
    let domains = crate::store::watchlist::list(&s.pool).await?;
    Ok(Json(json!({ "domains": domains })))
}

async fn add(
    State(s): State<AppState>,
    Json(b): Json<AddBody>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    if s.config.watchlist_mode != "db" {
        return Err(ApiError::BadRequest("watchlist not in db mode".into()));
    }
    let normalized =
        crate::parse::registered_domain(&b.domain).unwrap_or_else(|| b.domain.to_ascii_lowercase());
    let warn = if normalized != b.domain.to_ascii_lowercase() {
        Some(format!("normalized to registered_domain {normalized}"))
    } else {
        None
    };
    let added = crate::store::watchlist::add(&s.pool, &normalized, b.notes.as_deref()).await?;
    Ok((
        if added {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(json!({ "domain": normalized, "added": added, "warning": warn })),
    ))
}

async fn remove(Path(domain): Path<String>, State(s): State<AppState>) -> ApiResult<StatusCode> {
    if s.config.watchlist_mode != "db" {
        return Err(ApiError::BadRequest("watchlist not in db mode".into()));
    }
    let normalized =
        crate::parse::registered_domain(&domain).unwrap_or_else(|| domain.to_ascii_lowercase());
    let removed = crate::store::watchlist::remove(&s.pool, &normalized).await?;
    Ok(if removed {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    })
}
