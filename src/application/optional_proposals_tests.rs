use super::workflow::Engine;
use crate::{
    api::dto::RunRequest,
    config::Config,
    domain,
    infrastructure::{provider::Provider, store::Store},
};
use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

#[derive(Clone, Copy)]
enum OptionalResponse {
    Truncated,
    MissingContent,
    ProviderFailure,
}

async fn provider(
    State(optional): State<OptionalResponse>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let instruction = body["messages"][0]["content"].as_str().unwrap();
    let mut finish_reason = "stop";
    let message = if instruction.contains("Propose at most two") {
        match optional {
            OptionalResponse::ProviderFailure => {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            OptionalResponse::MissingContent => json!({"role":"assistant","content":null}),
            OptionalResponse::Truncated => {
                finish_reason = "length";
                json!({"role":"assistant","content":"{\"proposals\":["})
            }
        }
    } else if instruction.contains("Independently verify") {
        json!({"role":"assistant","content":json!({"verdict":"supported","findings":[],"contradictions":[],"historical_bias":[]}).to_string()})
    } else if body.get("tools").is_some() {
        json!({"role":"assistant","content":null,"tool_calls":[{"id":"read-fixture","function":{"name":"read_source","arguments":"{\"source_id\":\"s1\"}"}}]})
    } else if instruction.contains("Plan at most four") {
        json!({"role":"assistant","content":"{\"steps\":[\"Read the source\"]}"})
    } else {
        json!({"role":"assistant","content":json!({"answer":"The record mentions a school in 1799.","citations":[{"source_id":"s1","passage_id":"p001"}],"entities":[],"timeline":[],"uncertainty":[],"abstained":false}).to_string()})
    };
    Json(json!({"model":"apertus-v1-5-8b","choices":[{"message":message,"finish_reason":finish_reason}],"usage":{"prompt_tokens":10,"completion_tokens":10}})).into_response()
}

async fn scenario(optional: OptionalResponse) -> Value {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new()
        .route("/chat/completions", post(provider))
        .with_state(optional);
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let dir = std::env::temp_dir().join(format!("oldif-optional-test-{}", uuid::Uuid::new_v4()));
    let config = Config {
        bind: String::new(),
        grpc_bind: String::new(),
        internal_token: String::new(),
        covenant_token: None,
        cscs_base: format!("http://{address}"),
        cscs_key: Some("fixture-not-a-credential".into()),
        openai_base: String::new(),
        openai_key: None,
        default_model: "apertus-v1-5-8b".into(),
        reference_model: "reference".into(),
        realtime_model: String::new(),
        transcription_model: String::new(),
        voice: String::new(),
        run_dir: dir.clone(),
        timeout_secs: 5,
        catalog_refresh_secs: 1,
        output_tokens: 128,
        max_sources: 30,
        max_input_bytes: 120000,
        max_tool_calls: 4,
        max_concurrency: 1,
        max_runs: 10,
        voice_enabled: false,
        execution_profile: "research".into(),
    };
    let store = Store::open(dir.clone()).await.unwrap();
    let engine = Engine {
        provider: Provider::new(config.clone()).unwrap(),
        config,
        store: store.clone(),
        permits: Arc::new(Semaphore::new(1)),
    };
    let request: RunRequest = serde_json::from_value(json!({
        "question":"What year is named in the school record?", "locale":"en", "model":"apertus-v1-5-8b",
        "sources":[{"id":"s1","title":"Source","text":"School in 1799.","url":"https://example.org","page":null,"sha256":"a".repeat(64),"licence":"CC0"}],
        "session_id":"fixture", "user_id":"fixture", "history":[]
    })).unwrap();
    let started = engine.start(request).await.unwrap();
    let run = store
        .get(started["run_id"].as_str().unwrap())
        .await
        .unwrap();
    let snapshot = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = run.snapshot.lock().await.clone();
            if domain::terminal(&snapshot.status) {
                break serde_json::to_value(snapshot).unwrap();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    server.abort();
    std::fs::remove_dir_all(dir).unwrap();
    snapshot
}

#[tokio::test]
async fn invalid_optional_output_preserves_validated_result_and_explicit_trace() {
    for response in [
        OptionalResponse::Truncated,
        OptionalResponse::MissingContent,
    ] {
        let snapshot = scenario(response).await;
        assert_eq!(snapshot["status"], "needs_review", "{snapshot}");
        assert!(
            snapshot["result"]["answer"]
                .as_str()
                .unwrap()
                .contains("School in 1799")
        );
        assert_eq!(snapshot["result"]["evidence_only"], true);
        assert_eq!(
            snapshot["result"]["interpretation_draft"]["human_reviewed"],
            false
        );
        assert_eq!(
            snapshot["result"]["citations"][0]["quote"],
            "School in 1799"
        );
        assert_eq!(snapshot["result"]["citations"][0]["source_id"], "s1");
        assert_eq!(snapshot["result"]["citations"][0]["sha256"], "a".repeat(64));
        assert_eq!(snapshot["result"]["proposals"], json!([]));
        let events = snapshot["events"].as_array().unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event["kind"] == "model.started")
                .count(),
            5
        );
        let unavailable: Vec<_> = events
            .iter()
            .filter(|event| event["kind"] == "proposals.unavailable")
            .collect();
        assert_eq!(unavailable.len(), 1);
        assert_eq!(
            unavailable[0]["data"],
            json!({"reason":"invalid_or_truncated_json","proposals_published":0,"retry":false})
        );
        assert!(
            !events
                .iter()
                .any(|event| event["kind"] == "proposals.created")
        );
        if matches!(response, OptionalResponse::Truncated) {
            assert!(events.iter().any(|event| event["kind"] == "model.completed"
                && event["agent"] == "dreamer"
                && event["data"]["finish_reason"] == "length"));
        }
    }
}

#[tokio::test]
async fn optional_provider_failure_remains_a_failed_run() {
    let snapshot = scenario(OptionalResponse::ProviderFailure).await;
    assert_eq!(snapshot["status"], "failed");
    let events = snapshot["events"].as_array().unwrap();
    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "model.failed" && event["agent"] == "dreamer")
    );
    assert!(
        !events
            .iter()
            .any(|event| event["kind"] == "proposals.unavailable")
    );
}
