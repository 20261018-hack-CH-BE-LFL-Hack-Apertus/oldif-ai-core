use super::dto::{EventQuery, RunRequest, Steering};
use crate::{application::workflow::Engine, domain, error::CoreError};
use axum::{
    Json, Router,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{
        Response, Sse,
        sse::{Event as SseEvent, KeepAlive},
    },
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{convert::Infallible, sync::atomic::Ordering, time::Duration};
use subtle::ConstantTimeEq;
pub fn router(engine: Engine) -> Router {
    let protected = Router::new()
        .route("/v1/models", get(models))
        .route("/v1/runs", post(start))
        .route("/v1/runs/{id}", get(snapshot))
        .route("/v1/runs/{id}/events", get(events))
        .route("/v1/runs/{id}/steer", post(steer))
        .route("/v1/runs/{id}/cancel", post(cancel))
        .route("/v1/realtime/session", post(super::voice::session))
        .route(
            "/v1/diarization",
            get(super::diarization::capability)
                .post(super::diarization::process)
                .layer(axum::extract::DefaultBodyLimit::max(2_900_000)),
        )
        .route("/v1/chat/completions", post(super::compatibility::chat))
        .route_layer(middleware::from_fn_with_state(engine.clone(), authenticate));
    Router::new()
        .route(
            "/health",
            get(|| async { Json(json!({"status":"ok","service":"oldif-ai-core"})) }),
        )
        .merge(protected)
        .layer(axum::extract::DefaultBodyLimit::max(
            engine.config.max_input_bytes + 65536,
        ))
        .with_state(engine)
}
pub fn authorized(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .is_some_and(|v| bool::from(v.as_bytes().ct_eq(token.as_bytes())))
}
async fn authenticate(
    State(e): State<Engine>,
    req: Request,
    next: Next,
) -> Result<Response, CoreError> {
    if !authorized(req.headers(), &e.config.internal_token) {
        return Err(CoreError::Unauthorized);
    }
    Ok(next.run(req).await)
}
async fn models(State(e): State<Engine>) -> Json<Value> {
    Json(e.provider.models())
}
async fn start(
    State(e): State<Engine>,
    Json(r): Json<RunRequest>,
) -> Result<(StatusCode, Json<Value>), CoreError> {
    Ok((StatusCode::ACCEPTED, Json(e.start(r).await?)))
}
async fn snapshot(
    State(e): State<Engine>,
    Path(id): Path<String>,
) -> Result<Json<Value>, CoreError> {
    let run = e.store.get(&id).await?;
    let s = run.snapshot.lock().await;
    Ok(Json(
        json!({"run_id":s.run_id,"status":s.status,"result":s.result,"events":s.events}),
    ))
}
async fn events(
    State(e): State<Engine>,
    Path(id): Path<String>,
    Query(query): Query<EventQuery>,
    headers: HeaderMap,
) -> Result<Sse<impl futures_util::Stream<Item = Result<SseEvent, Infallible>>>, CoreError> {
    let run = e.store.get(&id).await?;
    let mut cursor = headers
        .get("last-event-id")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(query.after);
    let stream = async_stream::stream! {loop {
     let(events,done)={let s=run.snapshot.lock().await;(s.events.iter().filter(|v|v.sequence>cursor).cloned().collect::<Vec<_>>(),domain::terminal(&s.status))};
     for event in events{cursor=event.sequence;yield Ok(SseEvent::default().id(cursor.to_string()).event(event.kind.clone()).data(serde_json::to_string(&event).unwrap_or_default()));}
     if done{break;}tokio::time::sleep(Duration::from_millis(100)).await;
    }};
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
async fn steer(
    State(e): State<Engine>,
    Path(id): Path<String>,
    Json(input): Json<Steering>,
) -> Result<Json<Value>, CoreError> {
    if input.message.trim().is_empty() || input.message.len() > 4000 {
        return Err(CoreError::Invalid(
            "Steering message must contain 1–4000 bytes".into(),
        ));
    }
    let run = e.store.get(&id).await?;
    if domain::terminal(&run.snapshot.lock().await.status) {
        return Err(CoreError::Invalid(
            "This run has finished; start a new run with conversation context".into(),
        ));
    }
    let mut steering = run.steering.lock().await;
    if steering.len() >= 4 {
        return Err(CoreError::Invalid(
            "Steering limit reached for this run".into(),
        ));
    }
    steering.push(input.message.clone());
    drop(steering);
    e.store
        .emit(
            &run,
            "steering.received",
            "manager",
            json!({"message":input.message,"applies_at":"next_stage"}),
        )
        .await?;
    Ok(Json(json!({"accepted":true})))
}
async fn cancel(State(e): State<Engine>, Path(id): Path<String>) -> Result<Json<Value>, CoreError> {
    let run = e.store.get(&id).await?;
    run.cancelled.store(true, Ordering::SeqCst);
    e.store
        .emit(&run, "cancellation.requested", "manager", json!({}))
        .await?;
    Ok(Json(json!({"accepted":true})))
}
