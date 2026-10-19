use crate::{application::workflow::Engine, domain};
use futures_util::Stream;
use std::{pin::Pin, time::Duration};
use tonic::{Request, Response, Status};
pub mod proto {
    tonic::include_proto!("oldif.ai.v1");
}
use proto::{
    Empty, JsonPayload, RunRef,
    core_server::{Core, CoreServer},
};
#[derive(Clone)]
struct Service {
    engine: Engine,
}
impl Service {
    fn auth<T>(&self, r: &Request<T>) -> Result<(), Status> {
        use subtle::ConstantTimeEq;
        let v = r
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .unwrap_or_default();
        if bool::from(
            v.as_bytes()
                .ct_eq(self.engine.config.internal_token.as_bytes()),
        ) {
            Ok(())
        } else {
            Err(Status::unauthenticated("Internal authentication required"))
        }
    }
}
#[tonic::async_trait]
impl Core for Service {
    async fn get_models(&self, r: Request<Empty>) -> Result<Response<JsonPayload>, Status> {
        self.auth(&r)?;
        Ok(Response::new(JsonPayload {
            json: self.engine.provider.models().to_string(),
        }))
    }
    async fn start_run(&self, r: Request<JsonPayload>) -> Result<Response<RunRef>, Status> {
        self.auth(&r)?;
        let request = serde_json::from_str(&r.into_inner().json)
            .map_err(|_| Status::invalid_argument("Invalid run JSON"))?;
        let value = self
            .engine
            .start(request)
            .await
            .map_err(|e| Status::failed_precondition(e.to_string()))?;
        Ok(Response::new(RunRef {
            run_id: value["run_id"].as_str().unwrap_or_default().into(),
            after_sequence: 0,
            status: value["status"].as_str().unwrap_or_default().into(),
        }))
    }
    type StreamEventsStream =
        Pin<Box<dyn Stream<Item = Result<JsonPayload, Status>> + Send + 'static>>;
    async fn stream_events(
        &self,
        r: Request<RunRef>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        self.auth(&r)?;
        let input = r.into_inner();
        let mut cursor = input.after_sequence;
        let run = self
            .engine
            .store
            .get(&input.run_id)
            .await
            .map_err(|_| Status::not_found("Run not found"))?;
        let stream = async_stream::stream! {loop{
         let(events,done)={let s=run.snapshot.lock().await;(s.events.iter().filter(|v|v.sequence>cursor).cloned().collect::<Vec<_>>(),domain::terminal(&s.status))};
         for event in events{cursor=event.sequence;yield Ok(JsonPayload{json:serde_json::to_string(&event).unwrap_or_default()});}
         if done{break;}tokio::time::sleep(Duration::from_millis(100)).await;
        }};
        Ok(Response::new(Box::pin(stream)))
    }
}
pub async fn serve(engine: Engine) -> Result<(), Box<dyn std::error::Error>> {
    let addr = engine.config.grpc_bind.parse()?;
    tonic::transport::Server::builder()
        .add_service(CoreServer::new(Service { engine }))
        .serve(addr)
        .await?;
    Ok(())
}
