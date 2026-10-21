use super::{grpc, http};
use crate::{
    application::workflow::Engine,
    config::Config,
    infrastructure::{provider::Provider, store::Store},
};
use std::sync::Arc;
use tokio::sync::Semaphore;
const BACK: &str = "fixture-backend-distinct-token-123456";
const COV: &str = "fixture-covenant-distinct-token-12345";
pub(super) fn config(directory: std::path::PathBuf) -> Config {
    Config {
        bind: String::new(),
        grpc_bind: String::new(),
        internal_token: BACK.into(),
        covenant_token: Some(COV.into()),
        cscs_base: "http://127.0.0.1:1".into(),
        cscs_key: None,
        openai_base: "http://127.0.0.1:1".into(),
        openai_key: None,
        default_model: "apertus-v1-5-70b".into(),
        reference_model: "gpt-6-astra".into(),
        realtime_model: "voice".into(),
        transcription_model: "transcribe".into(),
        voice: "fixture".into(),
        run_dir: directory,
        timeout_secs: 1,
        catalog_refresh_secs: 1,
        output_tokens: 32,
        max_sources: 30,
        max_input_bytes: 120000,
        max_tool_calls: 4,
        max_concurrency: 1,
        max_runs: 10,
        voice_enabled: false,
        execution_profile: "research".into(),
    }
}
async fn engine() -> Engine {
    let directory = std::env::temp_dir().join(format!("oldif-auth-test-{}", uuid::Uuid::new_v4()));
    let config = config(directory.clone());
    Engine {
        provider: Provider::new(config.clone()).unwrap(),
        config,
        store: Store::open(directory).await.unwrap(),
        permits: Arc::new(Semaphore::new(1)),
    }
}
#[tokio::test]
async fn actual_http_rejects_covenant_control_without_provider_access() {
    let engine = engine().await;
    let directory = engine.config.run_dir.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server =
        tokio::spawn(async move { axum::serve(listener, http::router(engine)).await.unwrap() });
    let client = reqwest::Client::new();
    for path in [
        "/v1/runs",
        "/v1/realtime/session",
        "/v1/diarization",
        "/v1/runs/031de6bf-c9de-482a-a9fb-47627e2eceb0/cancel",
        "/v1/runs/031de6bf-c9de-482a-a9fb-47627e2eceb0/steer",
    ] {
        assert_eq!(
            client
                .post(format!("{base}{path}"))
                .bearer_auth(COV)
                .json(&serde_json::json!({}))
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
    }
    for token in [BACK, COV] {
        assert_eq!(
            client
                .get(format!("{base}/v1/models"))
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
    }
    assert_eq!(
        client
            .get(format!("{base}/v1/models"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .get(format!(
                "{base}/v1/runs/031de6bf-c9de-482a-a9fb-47627e2eceb0"
            ))
            .bearer_auth(COV)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        client
            .get(format!(
                "{base}/v1/runs/031de6bf-c9de-482a-a9fb-47627e2eceb0/events"
            ))
            .bearer_auth(COV)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    server.abort();
    std::fs::remove_dir_all(directory).unwrap();
}
#[tokio::test]
async fn actual_grpc_rejects_covenant_credential_on_every_rpc() {
    let engine = engine().await;
    let directory = engine.config.run_dir.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let incoming =
        async_stream::stream! { loop { yield listener.accept().await.map(|(stream, _)| stream); } };
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc::service(engine))
            .serve_with_incoming(incoming)
            .await
            .unwrap()
    });
    let mut client = grpc::proto::core_client::CoreClient::connect(base)
        .await
        .unwrap();
    let mut models = tonic::Request::new(grpc::proto::Empty {});
    models
        .metadata_mut()
        .insert("authorization", format!("Bearer {COV}").parse().unwrap());
    assert_eq!(
        client.get_models(models).await.unwrap_err().code(),
        tonic::Code::Unauthenticated
    );
    let mut start = tonic::Request::new(grpc::proto::JsonPayload { json: "{}".into() });
    start
        .metadata_mut()
        .insert("authorization", format!("Bearer {COV}").parse().unwrap());
    assert_eq!(
        client.start_run(start).await.unwrap_err().code(),
        tonic::Code::Unauthenticated
    );
    let mut events = tonic::Request::new(grpc::proto::RunRef {
        run_id: "fixture".into(),
        after_sequence: 0,
        status: String::new(),
    });
    events
        .metadata_mut()
        .insert("authorization", format!("Bearer {COV}").parse().unwrap());
    assert_eq!(
        client.stream_events(events).await.unwrap_err().code(),
        tonic::Code::Unauthenticated
    );
    let mut allowed = tonic::Request::new(grpc::proto::Empty {});
    allowed
        .metadata_mut()
        .insert("authorization", format!("Bearer {BACK}").parse().unwrap());
    assert!(client.get_models(allowed).await.is_ok());
    server.abort();
    std::fs::remove_dir_all(directory).unwrap();
}
