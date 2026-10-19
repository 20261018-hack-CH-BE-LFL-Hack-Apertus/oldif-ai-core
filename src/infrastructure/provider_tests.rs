use super::provider::Provider;
use crate::{config::Config, error::CoreError};
use axum::{Router, http::StatusCode, routing::post};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, time::Duration};

fn config(base: String, directory: PathBuf) -> Config {
    Config {
        bind: String::new(),
        grpc_bind: String::new(),
        internal_token: "fixture-internal-token".into(),
        covenant_token: None,
        cscs_base: base.clone(),
        cscs_key: Some("fixture-secret-do-not-retain".into()),
        openai_base: base,
        openai_key: Some("fixture-comparator-token".into()),
        default_model: "apertus-v1-5-70b".into(),
        reference_model: "reference".into(),
        realtime_model: "realtime".into(),
        transcription_model: "transcribe".into(),
        voice: "fixture".into(),
        run_dir: directory,
        timeout_secs: 1,
        output_tokens: 128,
        max_sources: 2,
        max_input_bytes: 12000,
        max_tool_calls: 1,
        max_concurrency: 1,
        max_runs: 10,
        voice_enabled: true,
        execution_profile: "submission".into(),
    }
}

#[tokio::test]
async fn submission_policy_rejects_proprietary_routes_before_network() {
    let p = Provider::new(config("http://127.0.0.1:1".into(), PathBuf::new())).unwrap();
    for model in ["reference", "realtime", "transcribe", "unknown"] {
        assert!(matches!(p.route(model), Err(CoreError::Policy(_))));
    }
    for model in [
        "apertus-v1-5-8b",
        "apertus-v1-5-70b",
        "apertus-v1-5-8b-thinking",
        "apertus-v1-5-70b-thinking",
    ] {
        assert!(p.route(model).is_ok());
    }
    assert!(
        p.models()["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["challenge_model"] == false)
            .all(|m| m["available"] == false)
    );
    let mut research = p.config.clone();
    research.execution_profile = "research".into();
    assert!(Provider::new(research).unwrap().route("reference").is_ok());
}

#[tokio::test]
async fn raw_response_receipts_survive_success_error_and_malformed_json() {
    const RAW: &str = "{\n\"model\":\"apertus-v1-5-70b\",\"choices\":[{\"message\":{\"content\":\"exact reply\"}}]}";
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new()
        .route("/ok/chat/completions", post(|| async { RAW }))
        .route(
            "/bad/chat/completions",
            post(|| async { (StatusCode::SERVICE_UNAVAILABLE, "upstream unavailable") }),
        )
        .route(
            "/malformed/chat/completions",
            post(|| async { "truncated{" }),
        )
        .route(
            "/slow/chat/completions",
            post(|| async {
                tokio::time::sleep(Duration::from_secs(2)).await;
                RAW
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let directory =
        std::env::temp_dir().join(format!("oldif-provider-receipts-{}", uuid::Uuid::new_v4()));
    let request = vec![json!({"role":"user","content":"private fixture text"})];
    for (route, success) in [("ok", true), ("bad", false), ("malformed", false)] {
        let p = Provider::new(config(
            format!("http://{address}/{route}"),
            directory.clone(),
        ))
        .unwrap();
        let result = p
            .chat("apertus-v1-5-70b", request.clone(), None, true)
            .await;
        assert_eq!(result.is_ok(), success);
        if let Ok(value) = result {
            assert_eq!(
                value["_oldif_receipt"]["raw_response_sha256"],
                format!("{:x}", Sha256::digest(RAW.as_bytes()))
            );
            assert_eq!(
                value["_oldif_receipt"]["revision_status"],
                "not_exposed_by_provider"
            );
        }
    }
    let p = Provider::new(config(format!("http://{address}/slow"), directory.clone())).unwrap();
    assert!(matches!(
        p.chat("apertus-v1-5-70b", request, None, true).await,
        Err(CoreError::Timeout)
    ));
    let mut files = tokio::fs::read_dir(directory.join("provider-receipts"))
        .await
        .unwrap();
    let mut count = 0;
    while let Some(file) = files.next_entry().await.unwrap() {
        let bytes = tokio::fs::read(file.path()).await.unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(!text.contains("private fixture text"));
        assert!(!text.contains("fixture-secret"));
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["raw_content_retained"], false);
        assert_eq!(value["configuration"]["max_tokens"], 128);
        assert_eq!(value["request_sha256"].as_str().unwrap().len(), 64);
        count += 1;
    }
    assert_eq!(count, 3);
    server.abort();
    tokio::fs::remove_dir_all(directory).await.unwrap();
}
