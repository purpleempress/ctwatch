use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use serde::Deserialize;
use std::collections::HashSet;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;

use crate::api::AppState;
use crate::stream::CertEvent;

#[derive(Deserialize)]
pub struct StreamQuery {
    #[serde(default)]
    pub domains: Option<String>,
}

pub async fn upgrade(
    ws: WebSocketUpgrade,
    Query(q): Query<StreamQuery>,
    State(state): State<AppState>,
) -> Response {
    if !state.config.stream_enabled {
        return axum::http::Response::builder()
            .status(axum::http::StatusCode::SERVICE_UNAVAILABLE)
            .body(axum::body::Body::from("stream disabled"))
            .unwrap();
    }
    // Subscriber cap.
    let subs = state.counters.subscribers();
    if (subs as usize) >= state.config.stream_max_subscribers {
        return axum::http::Response::builder()
            .status(axum::http::StatusCode::TOO_MANY_REQUESTS)
            .body(axum::body::Body::from("subscriber cap reached"))
            .unwrap();
    }
    let filter: Option<HashSet<String>> = q.domains.as_ref().map(|s| {
        s.split(',')
            .map(|d| d.trim().to_ascii_lowercase())
            .filter(|d| !d.is_empty())
            .collect()
    });
    let rx = state.stream_tx.subscribe();
    let counters = state.counters.clone();
    ws.on_upgrade(move |socket| handle(socket, rx, filter, counters))
}

async fn handle(
    mut socket: WebSocket,
    mut rx: tokio::sync::broadcast::Receiver<CertEvent>,
    filter: Option<HashSet<String>>,
    counters: crate::stats::Counters,
) {
    counters.set_subscribers(counters.subscribers() + 1);
    let mut hb = tokio::time::interval(Duration::from_secs(30));
    hb.tick().await; // skip immediate
    let mut last_pong = std::time::Instant::now();

    loop {
        tokio::select! {
            biased;
            ev = rx.recv() => {
                match ev {
                    Ok(evt) => {
                        if let Some(f) = &filter {
                            let matches = evt.cert.registered_domains.iter().any(|d| f.contains(d));
                            if !matches { continue; }
                        }
                        let payload = match serde_json::to_string(&evt) {
                            Ok(s) => s,
                            Err(_) => continue,
                        };
                        if socket.send(Message::Text(payload)).await.is_err() { break; }
                    }
                    Err(RecvError::Lagged(_)) => {
                        let _ = socket.send(Message::Close(Some(axum::extract::ws::CloseFrame {
                            code: 1013,
                            reason: "subscriber lagged".into(),
                        }))).await;
                        counters.incr_stream_dropped();
                        break;
                    }
                    Err(RecvError::Closed) => break,
                }
            }
            _ = hb.tick() => {
                if last_pong.elapsed() > Duration::from_secs(90) {
                    let _ = socket.send(Message::Close(Some(axum::extract::ws::CloseFrame {
                        code: 1011, reason: "no pong".into(),
                    }))).await;
                    break;
                }
                if socket.send(Message::Ping(vec![])).await.is_err() { break; }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Pong(_))) => { last_pong = std::time::Instant::now(); }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }
    counters.set_subscribers((counters.subscribers() - 1).max(0));
}
