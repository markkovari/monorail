//! HTTP/SSE API mounted by `monorail-sink` (ADR 0011).
//!
//! Contains no SQL — it calls typed functions from store/predict/coach.
//! Response bodies are `monorail-core` (or store row) types so the Leptos UI
//! deserializes the exact structs this side serializes.
//!
//! Remaining surface from ADR 0011 (plans CRUD/push, predictions, rower
//! status) lands with the coach/command-plane wiring.

use std::convert::Infallible;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use futures::Stream;
use monorail_core::plan::{PlanRequest, WorkoutPlan};
use monorail_core::wire::CommandReply;
use monorail_core::PlanId;
use monorail_store::{PlanRow, SessionSummaryRow, Store};
use serde::Serialize;
use tokio::sync::broadcast;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use uuid::Uuid;

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
    /// Core NATS client for the command plane (ADR 0010); `None` in tests
    /// or when the sink runs without NATS, making pushes 503.
    nats: Option<async_nats::Client>,
}

impl AppState {
    pub fn new(
        live: broadcast::Sender<LiveEvent>,
        store: Arc<Mutex<Store>>,
        nats: Option<async_nats::Client>,
    ) -> Self {
        Self { live, store, nats }
    }

    fn store(&self) -> Result<MutexGuard<'_, Store>, StatusCode> {
        self.store.lock().map_err(|_| {
            tracing::error!("store mutex poisoned");
            StatusCode::INTERNAL_SERVER_ERROR
        })
    }
}

fn internal(error: impl std::fmt::Display) -> StatusCode {
    tracing::error!(%error, "request failed");
    StatusCode::INTERNAL_SERVER_ERROR
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
    state
        .store()?
        .session_summaries()
        .map(Json)
        .map_err(internal)
}

/// Generate a plan from a goal (templates + feasibility, ADR 0009), persist
/// it as `recommended`, return it. Feasibility is judged by a critical-power
/// model fitted from the athlete's session history (ADR 0007); with too
/// little history the fit declines and feasibility stays `Unchecked`.
async fn create_plan(
    State(state): State<AppState>,
    Json(request): Json<PlanRequest>,
) -> Result<(StatusCode, Json<WorkoutPlan>), StatusCode> {
    let store = state.store()?;
    let judge = store
        .session_efforts()
        .map_err(internal)
        .map(|efforts| monorail_predict::CriticalPowerModel::fit(&efforts))?;
    let plan = monorail_coach::generate_plan(
        request.rower_id,
        request.goal,
        judge
            .as_ref()
            .map(|j| j as &dyn monorail_predict::FeasibilityJudge),
    );
    store
        .save_plan(&plan, "recommended", Utc::now())
        .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(plan)))
}

/// Stored per-segment compliance for a session; 404 until the session ends
/// and gets scored (or when it ran without a plan).
async fn session_compliance(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<Vec<monorail_store::ComplianceRow>>, StatusCode> {
    let session_id = Uuid::parse_str(&id)
        .map(monorail_core::SessionId)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let rows = state
        .store()?
        .get_compliance(session_id)
        .map_err(internal)?;
    if rows.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Json(rows))
}

async fn list_plans(State(state): State<AppState>) -> Result<Json<Vec<PlanRow>>, StatusCode> {
    state.store()?.list_plans().map(Json).map_err(internal)
}

fn parse_plan_id(id: &str) -> Result<PlanId, StatusCode> {
    Uuid::parse_str(id)
        .map(PlanId)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

async fn get_plan(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<WorkoutPlan>, StatusCode> {
    let plan_id = parse_plan_id(&id)?;
    state
        .store()?
        .get_plan(plan_id)
        .map_err(internal)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// Push a plan to its rower over the command plane (ADR 0010). On ack the
/// plan moves to `scheduled`; a nack passes the Pi's reason through as 409.
async fn push_plan(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<CommandReply>), StatusCode> {
    let plan_id = parse_plan_id(&id)?;
    let Some(nats) = state.nats.clone() else {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };
    let plan = state
        .store()?
        .get_plan(plan_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let reply = monorail_stream::commands::push_plan(&nats, &plan)
        .await
        .map_err(internal)?;

    match &reply {
        CommandReply::Ack { .. } => {
            state
                .store()?
                .set_plan_status(plan_id, "scheduled")
                .map_err(internal)?;
            Ok((StatusCode::OK, Json(reply)))
        }
        CommandReply::Nack { .. } => Ok((StatusCode::CONFLICT, Json(reply))),
    }
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
        .route("/api/v1/sessions/{id}/compliance", get(session_compliance))
        .route("/api/v1/plans", post(create_plan).get(list_plans))
        .route("/api/v1/plans/{id}", get(get_plan))
        .route("/api/v1/plans/{id}/push", post(push_plan))
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
        AppState::new(live, store, None)
    }

    async fn request(
        router: Router,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let request = match body {
            Some(json) => Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(json.to_string()))
                .unwrap(),
            None => Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        };
        let response = router.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    async fn get_json(uri: &str) -> (StatusCode, serde_json::Value) {
        request(router(test_state()), "GET", uri, None).await
    }

    fn ut2_request() -> serde_json::Value {
        serde_json::json!({
            "rower_id": "erg-1",
            "goal": {
                "zone": "ut2",
                "extent": { "time": { "seconds": 2400 } },
                "target_split_s": 120.0,
                "target_spm": 20,
                "hr_cap_bpm": null
            }
        })
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

    #[tokio::test]
    async fn plan_create_list_get_round_trip() {
        // One state shared across calls so the in-memory store persists.
        let state = test_state();

        let (status, plan) = request(
            router(state.clone()),
            "POST",
            "/api/v1/plans",
            Some(ut2_request()),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        // 40min UT2 -> build/core/push template (ADR 0009).
        assert_eq!(plan["segments"].as_array().unwrap().len(), 3);
        let id = plan["plan_id"].as_str().unwrap().to_string();

        let (status, list) = request(router(state.clone()), "GET", "/api/v1/plans", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list[0]["plan_id"], id.as_str());
        assert_eq!(list[0]["status"], "recommended");

        let (status, fetched) = request(
            router(state.clone()),
            "GET",
            &format!("/api/v1/plans/{id}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(fetched, plan);

        let (status, _) = request(
            router(state),
            "GET",
            "/api/v1/plans/00000000-0000-4000-8000-0000000000ff",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn push_without_nats_is_503() {
        let state = test_state();
        let (status, plan) = request(
            router(state.clone()),
            "POST",
            "/api/v1/plans",
            Some(ut2_request()),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let id = plan["plan_id"].as_str().unwrap();

        let (status, _) = request(
            router(state),
            "POST",
            &format!("/api/v1/plans/{id}/push"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn malformed_plan_id_is_400() {
        let (status, _) = request(
            router(test_state()),
            "GET",
            "/api/v1/plans/not-a-uuid",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
