use crate::{application::workflow::Engine, domain, error::CoreError};
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
pub(super) struct Service {
    engine: Engine,
}
impl Service {
    fn auth<T>(&self, r: &Request<T>) -> Result<(), Status> {
        let value = r
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok());
        if super::auth::backend_authorized(value, &self.engine.config.internal_token) {
            Ok(())
        } else {
            Err(Status::unauthenticated(
                "UNAUTHORIZED: Backend authentication required",
            ))
        }
    }
}
#[tonic::async_trait]
impl Core for Service {
    async fn get_models(&self, r: Request<Empty>) -> Result<Response<JsonPayload>, Status> {
        self.auth(&r)?;
        self.engine.provider.refresh_catalogs().await;
        Ok(Response::new(JsonPayload {
            json: self.engine.provider.models().to_string(),
        }))
    }
    async fn start_run(&self, r: Request<JsonPayload>) -> Result<Response<RunRef>, Status> {
        self.auth(&r)?;
        let request = serde_json::from_str(&r.into_inner().json)
            .map_err(|_| Status::invalid_argument("INVALID_REQUEST: Invalid run JSON"))?;
        let value = self.engine.start(request).await.map_err(status)?;
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
        let run = self.engine.store.get(&input.run_id).await.map_err(status)?;
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
        .add_service(service(engine))
        .serve(addr)
        .await?;
    Ok(())
}

fn status(error: CoreError) -> Status {
    let message = format!("{}: {}", error.code(), error);
    match error {
        CoreError::Invalid(_) => Status::invalid_argument(message),
        CoreError::Unauthorized => Status::unauthenticated(message),
        CoreError::NotFound => Status::not_found(message),
        CoreError::Cancelled => Status::cancelled(message),
        CoreError::Timeout => Status::deadline_exceeded(message),
        CoreError::Backpressure(_) => Status::resource_exhausted(message),
        CoreError::Policy(_) => Status::permission_denied(message),
        CoreError::Unavailable(_) | CoreError::Provider(_) | CoreError::Transport => {
            Status::unavailable(message)
        }
        CoreError::Storage | CoreError::Config(_) => Status::internal(message),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grpc_preserves_typed_failure_codes() {
        for (error, expected, code) in [
            (CoreError::Timeout, tonic::Code::DeadlineExceeded, "TIMEOUT"),
            (CoreError::Cancelled, tonic::Code::Cancelled, "CANCELLED"),
            (
                CoreError::Backpressure("busy".into()),
                tonic::Code::ResourceExhausted,
                "BACKPRESSURE",
            ),
            (
                CoreError::Policy("restricted".into()),
                tonic::Code::PermissionDenied,
                "POLICY_RESTRICTED",
            ),
        ] {
            let result = status(error);
            assert_eq!(result.code(), expected);
            assert!(result.message().starts_with(code));
        }
    }
}

pub(super) fn service(engine: Engine) -> CoreServer<Service> {
    CoreServer::new(Service { engine })
}
