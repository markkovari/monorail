//! HTTP/SSE API mounted by `monorail-sink` (ADR 0011).
//!
//! Contains no SQL — it calls typed functions from store/predict/coach.
//! Request/response bodies are `monorail-core` types so the Leptos UI
//! deserializes the exact structs this side serializes.
//!
//! Surface (per ADR 0011):
//! - `POST /api/v1/plans` (goal → generated plan), `GET /api/v1/plans`
//! - `GET /api/v1/plans/{id}`, `POST /api/v1/plans/{id}/push`
//! - `GET /api/v1/sessions`, `GET /api/v1/sessions/{id}`
//! - `GET /api/v1/predictions`
//! - `GET /api/v1/rowers/{id}/status`
//! - `GET /api/v1/live/{rower_id}` (SSE fan-out)

use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

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

/// Build the API router. Handlers grow state (store handle, NATS client,
/// live broadcast channels) via `Router::with_state` as they land.
pub fn router() -> Router {
    Router::new().route("/api/v1/health", get(health))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn health_endpoint_responds_ok() {
        let response = router()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
    }
}
