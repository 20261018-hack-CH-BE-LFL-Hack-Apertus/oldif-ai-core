use super::workflow::Engine;
use crate::{
    api::dto::RunRequest,
    config::Config,
    domain,
    infrastructure::{provider::Provider, store::Store},
};
use axum::{Json, Router, extract::State, routing::post};
use serde_json::{Value, json};
use std::{collections::VecDeque, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{Mutex, Semaphore};

#[derive(Clone, Copy)]
pub enum Candidate {
    Valid,
    InvalidJson,
    UnknownPassage,
}

#[derive(Clone)]
struct Mock {
    requests: Arc<Mutex<Vec<Value>>>,
    responses: Arc<Mutex<VecDeque<Value>>>,
}

async fn respond(State(mock): State<Mock>, Json(body): Json<Value>) -> Json<Value> {
    mock.requests.lock().await.push(body);
    let message = mock
        .responses
        .lock()
        .await
        .pop_front()
        .expect("unexpected model call");
    Json(json!({"model":"apertus-v1-5-8b","choices":[{"message":message,"finish_reason":"stop"}]}))
}

fn message(content: Value) -> Value {
    json!({"role":"assistant","content":content.to_string()})
}

fn answer() -> Value {
    json!({"answer":"The record names 1799.","citations":[{"source_id":"s1","passage_id":"p001"}],"abstained":false,"assertions":[{"kind":"date","subject":"s1","predicate":"date_mentioned","value":"1799","source_id":"s1","passage_id":"p001"}]})
}

fn responses(candidate: Candidate) -> VecDeque<Value> {
    let mut responses = VecDeque::from([
        message(json!({"steps":["Read the record and locate its date"]})),
        json!({"role":"assistant","content":null,"tool_calls":[{"id":"read-fixture","function":{"name":"read_source","arguments":"{\"source_id\":\"s1\"}"}}]}),
    ]);
    match candidate {
        Candidate::Valid => {}
        Candidate::InvalidJson => responses.push_back(json!({"role":"assistant","content":"{"})),
        Candidate::UnknownPassage => {
            let mut invalid = answer();
            invalid["citations"][0]["passage_id"] = json!("p999");
            responses.push_back(message(invalid));
        }
    }
    responses.extend([
        message(answer()),
        message(json!({"verdict":"supported"})),
    ]);
    responses
}

fn config(address: String, run_dir: PathBuf, output_tokens: u32) -> Config {
    Config {
        bind: String::new(),
        grpc_bind: String::new(),
        internal_token: String::new(),
        covenant_token: None,
        cscs_base: address,
        cscs_key: Some("fixture-not-a-credential".into()),
        openai_base: String::new(),
        openai_key: None,
        default_model: "apertus-v1-5-8b".into(),
        reference_model: "reference".into(),
        realtime_model: String::new(),
        transcription_model: String::new(),
        voice: String::new(),
        run_dir,
        timeout_secs: 5,
        catalog_refresh_secs: 1,
        output_tokens,
        max_sources: 30,
        max_input_bytes: 120000,
        max_tool_calls: 4,
        max_concurrency: 1,
        max_runs: 10,
        voice_enabled: false,
        execution_profile: "research".into(),
    }
}

async fn engine(config: Config) -> Engine {
    Engine {
        provider: Provider::new(config.clone()).unwrap(),
        store: Store::open(config.run_dir.clone()).await.unwrap(),
        config,
        permits: Arc::new(Semaphore::new(1)),
    }
}

pub struct Fixture {
    engine: Engine,
    mock: Mock,
    server: tokio::task::JoinHandle<()>,
    directory: PathBuf,
}

impl Fixture {
    pub async fn without_tools(mut self) -> Self {
        self.engine.config.max_tool_calls = 0;
        self.mock.responses.lock().await.remove(1).unwrap();
        self
    }

    pub async fn new(output_tokens: u32, candidate: Candidate) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mock = Mock {
            requests: Default::default(),
            responses: Arc::new(Mutex::new(responses(candidate))),
        };
        let router = Router::new()
            .route("/chat/completions", post(respond))
            .with_state(mock.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let directory =
            std::env::temp_dir().join(format!("oldif-compact-test-{}", uuid::Uuid::new_v4()));
        let config = config(
            format!("http://{address}"),
            directory.clone(),
            output_tokens,
        );
        let engine = engine(config).await;
        Self {
            engine,
            mock,
            server,
            directory,
        }
    }

    pub async fn run(&self) -> Value {
        let request: RunRequest = serde_json::from_value(json!({
            "question":"What year is named? Include its attributed date assertion.",
            "locale":"en", "model":"apertus-v1-5-8b",
            "sources":[{"id":"s1","title":"Source","text":"School in 1799.","url":"https://example.org","page":1,"sha256":"a".repeat(64),"licence":"CC0"}],
            "session_id":"fixture", "user_id":"fixture", "history":[]
        })).unwrap();
        let started = self.engine.start(request).await.unwrap();
        let run = self
            .engine
            .store
            .get(started["run_id"].as_str().unwrap())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let snapshot = run.snapshot.lock().await.clone();
                if domain::terminal(&snapshot.status) {
                    return serde_json::to_value(snapshot).unwrap();
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap()
    }

    pub async fn requests(&self) -> Vec<Value> {
        assert!(self.mock.responses.lock().await.is_empty());
        self.mock.requests.lock().await.clone()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}
