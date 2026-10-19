mod api;
mod application;
mod config;
mod domain;
mod error;
mod infrastructure;
use application::workflow::Engine;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = config::Config::from_env()?;
    let listener = tokio::net::TcpListener::bind(&config.bind).await?;
    let engine = Engine::new(config).await?;
    let grpc = api::grpc::serve(engine.clone());
    let http = axum::serve(listener, api::http::router(engine));
    tokio::select! {result=http=>result?,result=grpc=>result?};
    Ok(())
}
