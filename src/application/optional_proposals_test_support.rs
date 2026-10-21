use super::{stages, workflow::Engine};
use crate::{
    api::dto::RunRequest,
    config::Config,
    domain,
    error::CoreError,
    infrastructure::{
        provider::Provider,
        store::{Run, Store},
    },
};
use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{Mutex, Semaphore};

#[derive(Clone, Copy)]
pub enum OptionalResponse {
    Valid,
    Truncated,
    MissingContent,
    ProviderFailure,
}

#[derive(Clone)]
struct Mock {
    optional: OptionalResponse,
    requests: Arc<Mutex<Vec<Value>>>,
}

fn optional_response(optional: OptionalResponse) -> axum::response::Response {
    let (message, finish) = match optional {
        OptionalResponse::ProviderFailure => {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        OptionalResponse::MissingContent => (json!({"role":"assistant","content":null}), "stop"),
        OptionalResponse::Truncated => (
            json!({"role":"assistant","content":"{\"proposals\":["}),
            "length",
        ),
        OptionalResponse::Valid => (
            text(
                json!({"proposals":[{"kind":"source_review","description":"Review the transcription.","rationale":"Check source fidelity."}]}),
            ),
            "stop",
        ),
    };
    response(message, finish)
}

fn text(value: Value) -> Value {
    json!({"role":"assistant","content":value.to_string()})
}

fn response(message: Value, finish: &str) -> axum::response::Response {
    Json(json!({"model":"apertus-v1-5-8b","choices":[{"message":message,"finish_reason":finish}],"usage":{"prompt_tokens":10,"completion_tokens":10}})).into_response()
}

async fn provider(State(mock): State<Mock>, Json(body): Json<Value>) -> axum::response::Response {
    mock.requests.lock().await.push(body.clone());
    let instruction = body["messages"][0]["content"].as_str().unwrap();
    if instruction.contains("Propose at most two") {
        return optional_response(mock.optional);
    }
    let message = if instruction.contains("Independently verify") {
        text(json!({"verdict":"supported"}))
    } else if body.get("tools").is_some() {
        json!({"role":"assistant","content":null,"tool_calls":[{"id":"read-fixture","function":{"name":"read_source","arguments":"{\"source_id\":\"s1\"}"}}]})
    } else if instruction.contains("Plan ") {
        text(json!({"steps":["Read the source"]}))
    } else {
        text(
            json!({"answer":"The record mentions a school in 1799.","citations":[{"source_id":"s1","passage_id":"p001"}],"abstained":false}),
        )
    };
    response(message, "stop")
}

fn config(address: String, run_dir: PathBuf) -> Config {
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
        output_tokens: 128,
        max_sources: 30,
        max_input_bytes: 120000,
        max_tool_calls: 4,
        max_concurrency: 1,
        max_runs: 10,
        voice_enabled: false,
        execution_profile: "research".into(),
    }
}

fn request() -> RunRequest {
    serde_json::from_value(json!({
        "question":"What year is named in the school record?", "locale":"en", "model":"apertus-v1-5-8b",
        "sources":[{"id":"s1","title":"Source","text":"School in 1799.","url":"https://example.org","page":null,"sha256":"a".repeat(64),"licence":"CC0"}],
        "session_id":"fixture", "user_id":"fixture", "history":[]
    })).unwrap()
}

pub async fn snapshot(run: &Run) -> Value {
    serde_json::to_value(run.snapshot.lock().await.clone()).unwrap()
}

async fn completed(run: &Run) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let value = snapshot(run).await;
            if domain::terminal(value["status"].as_str().unwrap()) {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}

pub struct Fixture {
    engine: Engine,
    requests: Arc<Mutex<Vec<Value>>>,
    server: tokio::task::JoinHandle<()>,
    pub directory: PathBuf,
}

impl Fixture {
    pub async fn new(optional: OptionalResponse) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let router = Router::new()
            .route("/chat/completions", post(provider))
            .with_state(Mock {
                optional,
                requests: requests.clone(),
            });
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let directory =
            std::env::temp_dir().join(format!("oldif-optional-test-{}", uuid::Uuid::new_v4()));
        let config = config(format!("http://{address}"), directory.clone());
        let engine = Engine {
            provider: Provider::new(config.clone()).unwrap(),
            store: Store::open(directory.clone()).await.unwrap(),
            config,
            permits: Arc::new(Semaphore::new(1)),
        };
        Self {
            engine,
            requests,
            server,
            directory,
        }
    }

    pub async fn workflow(&self) -> Value {
        let started = self.engine.start(request()).await.unwrap();
        let run = self
            .engine
            .store
            .get(started["run_id"].as_str().unwrap())
            .await
            .unwrap();
        completed(&run).await
    }

    pub async fn run(&self) -> Arc<Run> {
        self.engine.store.create(request()).await.unwrap().0
    }

    pub async fn reflect(&self, run: &Run, result: &Value) -> Result<Value, CoreError> {
        stages::reflect(&self.engine, run, &request(), result).await
    }

    pub async fn request_count(&self) -> usize {
        self.requests.lock().await.len()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}
