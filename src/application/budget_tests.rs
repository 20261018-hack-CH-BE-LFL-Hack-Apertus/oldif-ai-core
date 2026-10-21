use super::{stages, validation, workflow::Engine};
use crate::{
    api::dto::RunRequest,
    config::Config,
    infrastructure::{provider::Provider, store::Store},
};
use axum::{Json, Router, extract::State, routing::post};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};

#[tokio::test]
async fn tool_budget_finalizes_without_executing_excess_calls_and_survives_revision() {
    finalization_scenario(2).await;
}
#[tokio::test]
async fn all_sources_read_finalizes_even_with_tool_budget_remaining() {
    finalization_scenario(4).await;
}
// A model requests three reads; finalization must remove tools after the evidence
// scope is complete, including when the configured call budget is not exhausted.
async fn finalization_scenario(budget: usize) {
    async fn provider(
        State(requests): State<Arc<Mutex<Vec<Value>>>>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        requests.lock().await.push(body.clone());
        let message = if body.get("tools").is_some() {
            json!({"role":"assistant","content":null,"tool_calls":(0..3).map(|i|json!({"id":format!("call-{i}"),"function":{"name":"read_source","arguments":"{\"source_id\":\"s1\"}"}})).collect::<Vec<_>>()})
        } else {
            assert_eq!(body["response_format"]["type"], "json_object");
            json!({"role":"assistant","content":json!({"answer":"Latin was taught","citations":[],"abstained":true,"verdict":"supported","findings":["Latin was taught"],"proposals":[{"kind":"source_review","description":"Latin was taught","rationale":"Latin was taught"}]}).to_string()})
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
        max_tool_calls: budget,
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
    let request = RunRequest {
        question: "Find school evidence".into(),
        locale: "en".into(),
        model: "apertus-v1-5-8b".into(),
        sources: vec![serde_json::from_value(json!({"id":"s1","title":"Source","text":"school source","url":"https://example.org","page":null,"sha256":"a".repeat(64),"licence":"CC0"})).unwrap()],
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
    let grounded = validation::ground(answer, &request).unwrap();
    let review = stages::verify(&engine, &run, &request, &grounded)
        .await
        .unwrap();
    let proposals = stages::reflect(&engine, &run, &request, &grounded)
        .await
        .unwrap();
    assert_eq!(review["verdict"], "abstain");
    assert_eq!(review["output_policy"], "deterministic_abstention");
    assert!(!review.to_string().contains("Latin"));
    assert_eq!(proposals, json!([]));
    let snapshot = run.snapshot.lock().await;
    assert!(
        snapshot
            .events
            .iter()
            .any(|event| event.kind == "review.output_bounded")
    );
    assert!(
        snapshot
            .events
            .iter()
            .any(|event| event.kind == "proposals.withheld")
    );
    assert_eq!(
        snapshot
            .events
            .iter()
            .filter(|e| e.kind == "tool.started")
            .count(),
        budget.min(3)
    );
    assert_eq!(
        snapshot
            .events
            .iter()
            .filter(|e| e.kind == "tool.denied")
            .count(),
        3_usize.saturating_sub(budget)
    );
    let received = requests.lock().await;
    assert_eq!(received.len(), 5);
    assert_eq!(received[0]["tool_choice"], "required");
    assert_eq!(received[0]["tools"][0]["function"]["name"], "read_source");
    assert_eq!(
        received[0]["tools"][0]["function"]["parameters"]["properties"]["source_id"]["enum"],
        json!(["s1"])
    );
    assert!(received[1].get("tools").is_none());
    assert!(received[2].get("tools").is_none());
    drop(snapshot);
    server.abort();
    std::fs::remove_dir_all(dir).unwrap();
}
