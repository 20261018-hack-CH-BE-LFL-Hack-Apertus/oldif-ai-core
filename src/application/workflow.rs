use super::{stages, validation};
use crate::{
    api::dto::RunRequest,
    config::Config,
    error::CoreError,
    infrastructure::{
        provider::Provider,
        store::{Run, Store},
    },
};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::Ordering};
use tokio::sync::Semaphore;
#[derive(Clone)]
pub struct Engine {
    pub config: Config,
    pub provider: Provider,
    pub store: Store,
    pub permits: Arc<Semaphore>,
}
impl Engine {
    pub async fn new(config: Config) -> Result<Self, CoreError> {
        let provider = Provider::new(config.clone())?;
        provider.probe_catalogs().await;
        Ok(Self {
            provider,
            store: Store::open(config.run_dir.clone()).await?,
            permits: Arc::new(Semaphore::new(config.max_concurrency)),
            config,
        })
    }
    pub async fn start(&self, request: RunRequest) -> Result<Value, CoreError> {
        validation::request(&request, &self.config)?;
        self.provider.route(&request.model)?;
        if self.store.runs.lock().await.len() >= self.config.max_runs {
            return Err(CoreError::Unavailable(
                "Run storage limit reached; archive completed runs before retrying".into(),
            ));
        }
        let (run, created) = self.store.create(request).await?;
        let response = {
            let s = run.snapshot.lock().await;
            json!({"run_id":s.run_id,"status":s.status})
        };
        if created {
            let engine = self.clone();
            tokio::spawn(async move {
                engine.execute(run).await;
            });
        }
        Ok(response)
    }
    async fn execute(&self, run: Arc<Run>) {
        let outcome = self.execute_inner(&run).await;
        if let Err(error) = outcome {
            let status = if matches!(error, CoreError::Cancelled) {
                "cancelled"
            } else {
                "failed"
            };
            if self
                .store
                .finish(&run, status, json!({"error":error.to_string()}))
                .await
                .is_err()
            {
                eprintln!("run_store_write_failed");
            }
        }
    }
    async fn execute_inner(&self, run: &Run) -> Result<(), CoreError> {
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|_| CoreError::Cancelled)?;
        Self::check_cancel(run)?;
        {
            run.snapshot.lock().await.status = "running".into();
        }
        self.store.emit(run,"run.started","manager",json!({"workflow":"archival-evidence-v1","agents":["planner","researcher","verifier","dreamer"],"tools":["search_sources","read_source"]})).await?;
        let request = run.snapshot.lock().await.request.clone();
        let plan = stages::plan(self, run, &request).await?;
        let steering_version = run.steering.lock().await.len();
        let candidate = stages::research(self, run, &request, &plan).await?;
        let mut grounded = self.validate_candidate(run, &request, candidate).await?;
        if run.steering.lock().await.len() > steering_version {
            self.store
                .emit(
                    run,
                    "steering.applied",
                    "manager",
                    json!({"stage":"research_revision"}),
                )
                .await?;
            let revised = stages::research(self, run, &request, &plan).await?;
            grounded = self.validate_candidate(run, &request, revised).await?;
        }
        self.store.emit(run,"validation.completed","outcome-owner",json!({"exact_quotes":true,"citations":grounded["citations"].as_array().map(Vec::len)})).await?;
        let review = stages::verify(self, run, &request, &grounded).await?;
        grounded["verification"] = review;
        grounded["proposals"] = stages::reflect(self, run, &request, &grounded).await?;
        Self::check_cancel(run)?;
        self.store.finish(run, "needs_review", grounded).await
    }
    async fn validate_candidate(
        &self,
        run: &Run,
        request: &RunRequest,
        candidate: Value,
    ) -> Result<Value, CoreError> {
        match validation::ground(candidate.clone(), request) {
            Ok(value) => Ok(value),
            Err(error) => {
                self.store
                    .emit(
                        run,
                        "validation.failed",
                        "outcome-owner",
                        json!({"error":error.to_string(),"candidate":candidate,"repair_budget":1}),
                    )
                    .await?;
                let repaired =
                    stages::repair(self, run, request, candidate, &error.to_string()).await?;
                validation::ground(repaired, request)
            }
        }
    }
    pub fn check_cancel(run: &Run) -> Result<(), CoreError> {
        if run.cancelled.load(Ordering::SeqCst) {
            Err(CoreError::Cancelled)
        } else {
            Ok(())
        }
    }
    pub async fn context(&self, run: &Run, r: &RunRequest) -> Result<Value, CoreError> {
        Self::check_cancel(run)?;
        let steering = run.steering.lock().await.clone();
        Ok(
            json!({"question":r.question,"locale":r.locale,"prior_conversation_data":r.history,"user_steering":steering,"sources":r.sources}),
        )
    }
    pub async fn call(
        &self,
        run: &Run,
        agent: &str,
        model: &str,
        messages: Vec<Value>,
        tools: Option<Value>,
    ) -> Result<Value, CoreError> {
        Self::check_cancel(run)?;
        let started = std::time::Instant::now();
        let call_id = uuid::Uuid::new_v4().to_string();
        self.store.emit(run,"model.started",agent,json!({"call_id":call_id,"model":model,"tools_enabled":tools.is_some(),"max_output_tokens":self.config.output_tokens})).await?;
        let result = tokio::select! {result=self.provider.chat(model,messages,tools)=>result,_=cancelled(run)=>Err(CoreError::Cancelled)};
        let response = match result {
            Ok(value) => value,
            Err(error) => {
                self.store.emit(run,"model.failed",agent,json!({"call_id":call_id,"model":model,"error":error.to_string(),"duration_ms":started.elapsed().as_millis()})).await?;
                return Err(error);
            }
        };
        Self::check_cancel(run)?;
        self.store.emit(run,"model.completed",agent,json!({"call_id":call_id,"requested_model":model,"served_model":response["model"],"provider_response_id":response["id"],"api":response.get("api").cloned().unwrap_or(json!("chat/completions")),"usage":response["usage"],"duration_ms":started.elapsed().as_millis()})).await?;
        Ok(response["choices"][0]["message"].clone())
    }
}
async fn cancelled(run: &Run) {
    loop {
        if run.cancelled.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
