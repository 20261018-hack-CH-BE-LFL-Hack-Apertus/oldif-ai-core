use super::{grpc, http};
use crate::application::workflow::Engine;
use axum::{Router, http::StatusCode, routing::get};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU16, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{net::TcpListener, task::JoinHandle};

struct Fixture {
    status: Arc<AtomicU16>,
    calls: Arc<AtomicUsize>,
    http: String,
    grpc: String,
    token: String,
    covenant_token: String,
    directory: PathBuf,
    servers: [JoinHandle<()>; 3],
}

async fn upstream(status: Arc<AtomicU16>, calls: Arc<AtomicUsize>) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let router = Router::new().route(
        "/v1/models",
        get(move || {
            let (status, calls) = (status.clone(), calls.clone());
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(1100)).await;
                (
                    StatusCode::from_u16(status.load(Ordering::SeqCst)).unwrap(),
                    r#"{"data":[{"id":"apertus-v1-5-8b"}]}"#,
                )
            }
        }),
    );
    (
        url,
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() }),
    )
}

async fn http_server(engine: Engine) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    (
        url,
        tokio::spawn(async move { axum::serve(listener, http::router(engine)).await.unwrap() }),
    )
}

async fn grpc_server(engine: Engine) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let incoming = async_stream::stream! {
        loop {
            yield listener.accept().await.map(|(stream, _)| stream);
        }
    };
    (
        url,
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(grpc::service(engine))
                .serve_with_incoming(incoming)
                .await
                .unwrap()
        }),
    )
}

async fn configured_engine(url: String) -> Engine {
    let directory =
        std::env::temp_dir().join(format!("oldif-catalog-test-{}", uuid::Uuid::new_v4()));
    let mut config = super::auth_tests::config(directory);
    config.cscs_base = url;
    config.cscs_key = Some("fixture-provider-token".into());
    Engine::new(config).await.unwrap()
}

impl Fixture {
    async fn new() -> Self {
        let status = Arc::new(AtomicU16::new(503));
        let calls = Arc::new(AtomicUsize::new(0));
        let (url, provider) = upstream(status.clone(), calls.clone()).await;
        let engine = configured_engine(url).await;
        let directory = engine.config.run_dir.clone();
        let token = engine.config.internal_token.clone();
        let covenant_token = engine.config.covenant_token.clone().unwrap();
        let (http, http_server) = http_server(engine.clone()).await;
        let (grpc, grpc_server) = grpc_server(engine).await;
        Self {
            status,
            calls,
            http,
            grpc,
            token,
            covenant_token,
            directory,
            servers: [provider, http_server, grpc_server],
        }
    }

    async fn models(&self, use_http: bool) -> Value {
        if use_http {
            return self.http_models(&self.token).await;
        }
        let mut client = grpc::proto::core_client::CoreClient::connect(self.grpc.clone())
            .await
            .unwrap();
        let mut request = tonic::Request::new(grpc::proto::Empty {});
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {}", self.token).parse().unwrap(),
        );
        serde_json::from_str(&client.get_models(request).await.unwrap().into_inner().json).unwrap()
    }

    async fn http_models(&self, token: &str) -> Value {
        reqwest::Client::new()
            .get(format!("{}/v1/models", self.http))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for server in &self.servers {
            server.abort();
        }
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn model(catalog: &Value, id: &str) -> Value {
    catalog["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id)
        .unwrap()
        .clone()
}

fn assert_recovered(catalog: &Value) {
    assert_eq!(model(catalog, "apertus-v1-5-8b")["available"], true);
    assert_eq!(
        model(catalog, "apertus-v1-5-8b")["access"],
        "catalog_reachable"
    );
    assert_eq!(model(catalog, "apertus-v1-5-70b")["available"], false);
    assert_eq!(
        model(catalog, "apertus-v1-5-70b")["access"],
        "model_not_listed"
    );
    assert_eq!(catalog["automatic_fallback"], false);
}

async fn assert_unauthorized_does_not_refresh(fixture: &Fixture) {
    let before = fixture.count();
    let denied = reqwest::Client::new()
        .get(format!("{}/v1/models", fixture.http))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        fixture.count(),
        before,
        "unauthorized callers must not refresh providers"
    );
}

async fn assert_access_denial_refresh(fixture: &Fixture) {
    fixture.status.store(401, Ordering::SeqCst);
    assert_recovered(&fixture.models(false).await);
    assert_eq!(
        fixture.count(),
        2,
        "rapid reloads must respect the refresh interval"
    );
    tokio::time::sleep(Duration::from_millis(1050)).await;
    let denied = fixture.models(false).await;
    assert_eq!(model(&denied, "apertus-v1-5-8b")["available"], false);
    assert_eq!(model(&denied, "apertus-v1-5-8b")["access"], "access_denied");
    assert_eq!(
        model(&denied, "apertus-v1-5-8b")["observation"]["http_status"],
        401
    );
    assert_eq!(fixture.count(), 3);
}

#[tokio::test]
async fn http_models_reload_recovers_after_failed_startup_probe() {
    let fixture = Fixture::new().await;
    assert_eq!(
        model(&fixture.models(true).await, "apertus-v1-5-8b")["access"],
        "provider_error"
    );
    fixture.status.store(200, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(1050)).await;
    assert_eq!(
        fixture.count(),
        1,
        "idle time must not probe or keep the provider warm"
    );
    assert_unauthorized_does_not_refresh(&fixture).await;
    assert_recovered(&fixture.models(true).await);
    assert_recovered(&fixture.models(false).await);
    assert_eq!(
        fixture.count(),
        2,
        "HTTP and gRPC must share the refresh interval"
    );
}

#[tokio::test]
async fn concurrent_model_reloads_share_refresh_and_preserve_access_denial() {
    let fixture = Fixture::new().await;
    fixture.status.store(200, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(1050)).await;
    let catalogs = futures_util::future::join_all((0..8).map(|i| fixture.models(i % 2 == 0))).await;
    for catalog in catalogs {
        assert_recovered(&catalog);
    }
    assert_eq!(
        fixture.count(),
        2,
        "waiting reloads must not each issue a probe"
    );
    assert_access_denial_refresh(&fixture).await;
}

async fn assert_covenant_reads_cached(fixture: &Fixture, expected: &Value) {
    let before = fixture.count();
    for _ in 0..3 {
        let catalog = fixture.http_models(&fixture.covenant_token).await;
        assert_eq!(
            fixture.count(),
            before,
            "health reads must not contact the provider"
        );
        assert_eq!(&catalog, expected);
    }
}

#[tokio::test]
async fn covenant_health_catalog_reads_neither_wake_nor_postpone_provider_refresh() {
    let fixture = Fixture::new().await;
    let initial = fixture.models(true).await;
    fixture.status.store(200, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(1050)).await;
    assert_covenant_reads_cached(&fixture, &initial).await;
    assert_unauthorized_does_not_refresh(&fixture).await;
    let refreshed = fixture.models(true).await;
    assert_recovered(&refreshed);
    assert_eq!(fixture.count(), 2);
    fixture.status.store(401, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(1050)).await;
    assert_covenant_reads_cached(&fixture, &refreshed).await;
    let denied = fixture.models(false).await;
    assert_eq!(model(&denied, "apertus-v1-5-8b")["access"], "access_denied");
    assert_eq!(
        fixture.count(),
        3,
        "backend gRPC must still perform its due refresh"
    );
}
