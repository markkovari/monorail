//! HTTP/SSE API mounted by `monorail-sink` (ADR 0011).
//!
//! Contains no SQL — it calls typed functions from store/predict/coach.
//! Response bodies are `monorail-core` (or store row) types so the Leptos UI
//! deserializes the exact structs this side serializes.
//!
//! Remaining surface from ADR 0011 (plans CRUD/push, predictions, rower
//! status) lands with the coach/command-plane wiring.

use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::{Json, Router};
use futures::Stream;
use monorail_store::{SessionSummaryRow, Store};
use serde::Serialize;
use tokio::sync::broadcast;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

/// One telemetry message fanned out to live dashboard subscribers.
/// `payload` is the wire envelope JSON, passed through verbatim.
#[derive(Debug, Clone)]
pub struct LiveEvent {
    pub rower: String,
    /// SSE event name: `monitor`, `stroke`, or `workout_event`.
    pub kind: &'static str,
    pub payload: String,
}

/// Shared state behind the router.
#[derive(Clone)]
pub struct AppState {
    live: broadcast::Sender<LiveEvent>,
    store: Arc<Mutex<Store>>,
}

impl AppState {
    pub fn new(live: broadcast::Sender<LiveEvent>, store: Arc<Mutex<Store>>) -> Self {
        Self { live, store }
    }
}

#[derive(Debug, Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn sessions(
    State(state): State<AppState>,
) -> Result<Json<Vec<SessionSummaryRow>>, StatusCode> {
    let store = state.store.lock().map_err(|_| {
        tracing::error!("store mutex poisoned");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    store.session_summaries().map(Json).map_err(|error| {
        tracing::error!(%error, "session query failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

/// SSE fan-out of live telemetry for one rower. Lagging subscribers drop
/// messages (broadcast semantics) — fine for a dashboard; the store is the
/// system of record.
async fn live(
    Path(rower_id): Path<String>,
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.live.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(move |item| match item {
        Ok(event) if event.rower == rower_id => {
            Some(Ok(Event::default().event(event.kind).data(event.payload)))
        }
        Ok(_) => None,
        Err(BroadcastStreamRecvError::Lagged(skipped)) => {
            tracing::debug!(skipped, "live subscriber lagged");
            None
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Build the API router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/sessions", get(sessions))
        .route("/api/v1/live/{rower_id}", get(live))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;

    fn test_state() -> AppState {
        let (live, _) = broadcast::channel(16);
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        AppState::new(live, store)
    }

    async fn get_json(uri: &str) -> (StatusCode, serde_json::Value) {
        let response = router(test_state())
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    #[tokio::test]
    async fn health_endpoint_responds_ok() {
        let (status, json) = get_json("/api/v1/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn sessions_endpoint_returns_empty_list() {
        let (status, json) = get_json("/api/v1/sessions").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json, serde_json::json!([]));
    }

    #[tokio::test]
    async fn live_endpoint_is_event_stream() {
        let response = router(test_state())
            .oneshot(
                Request::builder()
                    .uri("/api/v1/live/erg-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "text/event-stream");
    }

    #[tokio::test]
    async fn unknown_route_is_404() {
        let (status, _) = get_json("/api/v1/nope").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
