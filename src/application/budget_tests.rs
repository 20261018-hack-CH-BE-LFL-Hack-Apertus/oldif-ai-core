use super::{stages, workflow::Engine};
use crate::{
    api::dto::RunRequest,
    config::Config,
    infrastructure::{provider::Provider, store::Store},
};
use axum::{Json, Router, extract::State, routing::post};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};

// The provider intentionally requests three tools with only two available slots.
// Its final response is accepted only if the next request removes tools.
#[tokio::test]
async fn tool_budget_finalizes_without_executing_excess_calls_and_survives_revision() {
    async fn provider(
        State(requests): State<Arc<Mutex<Vec<Value>>>>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        requests.lock().await.push(body.clone());
        let message = if body.get("tools").is_some() {
            json!({"role":"assistant","content":null,"tool_calls":(0..3).map(|i|json!({"id":format!("call-{i}"),"function":{"name":"search_sources","arguments":"{\"query\":\"school\"}"}})).collect::<Vec<_>>()})
        } else {
            assert_eq!(body["response_format"]["type"], "json_object");
            json!({"role":"assistant","content":"{\"answer\":\"Insufficient evidence\",\"citations\":[],\"abstained\":true}"})
        };
        Json(json!({"model":"apertus-v1-5-8b","choices":[{"message":message}]}))
    }
    let requests = Arc::new(Mutex::new(vec![]));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new()
        .route("/chat/completions", post(provider))
        .with_state(requests.clone());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let dir = std::env::temp_dir().join(format!("oldif-budget-test-{}", uuid::Uuid::new_v4()));
    let config = Config {
        bind: String::new(),
        grpc_bind: String::new(),
        internal_token: String::new(),
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
        output_tokens: 128,
        max_sources: 30,
        max_input_bytes: 120000,
        max_tool_calls: 2,
        max_concurrency: 1,
        max_runs: 10,
        voice_enabled: false,
    };
    let store = Store::open(dir.clone()).await.unwrap();
    let engine = Engine {
        provider: Provider::new(config.clone()).unwrap(),
        config,
        store: store.clone(),
        permits: Arc::new(Semaphore::new(1)),
    };
    let request = RunRequest {
        question: "Find school evidence".into(),
        locale: "en".into(),
        model: "apertus-v1-5-8b".into(),
        sources: vec![],
        session_id: "fixture".into(),
        user_id: "fixture".into(),
        history: vec![],
        idempotency_key: None,
    };
    let (run, _) = store.create(request.clone()).await.unwrap();
    let answer = stages::research(&engine, &run, &request, &json!({}))
        .await
        .unwrap();
    assert_eq!(answer["abstained"], true);
    stages::research(&engine, &run, &request, &json!({}))
        .await
        .unwrap();
    let snapshot = run.snapshot.lock().await;
    assert_eq!(
        snapshot
            .events
            .iter()
            .filter(|e| e.kind == "tool.started")
            .count(),
        2
    );
    assert_eq!(
        snapshot
            .events
            .iter()
            .filter(|e| e.kind == "tool.denied")
            .count(),
        1
    );
    let received = requests.lock().await;
    assert_eq!(received.len(), 3);
    assert!(received[1].get("tools").is_none());
    assert!(received[2].get("tools").is_none());
    drop(snapshot);
    server.abort();
    std::fs::remove_dir_all(dir).unwrap();
}
