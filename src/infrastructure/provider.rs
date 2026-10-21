use crate::{config::Config, domain, error::CoreError};
use serde_json::{Value, json};
use std::time::Duration;
#[derive(Clone)]
pub struct Provider {
    pub client: reqwest::Client,
    pub config: Config,
    pub observations: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, Value>>>,
    pub(super) catalog_refresh: std::sync::Arc<tokio::sync::Mutex<Option<std::time::Instant>>>,
}
impl Provider {
    pub fn new(config: Config) -> Result<Self, CoreError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| CoreError::Transport)?;
        Ok(Self {
            client,
            config,
            observations: Default::default(),
            catalog_refresh: Default::default(),
        })
    }
    pub fn route(&self, model: &str) -> Result<(&str, &str), CoreError> {
        if [
            "apertus-v1-5-8b",
            "apertus-v1-5-8b-thinking",
            "apertus-v1-5-70b",
            "apertus-v1-5-70b-thinking",
        ]
        .contains(&model)
        {
            return self
                .config
                .cscs_key
                .as_deref()
                .map(|k| (self.config.cscs_base.as_str(), k))
                .ok_or_else(|| {
                    CoreError::Unavailable(
                        "CSCS_API_KEY is not configured; no replacement model was used".into(),
                    )
                });
        }
        if self.config.execution_profile == "submission" {
            return Err(CoreError::Policy("Submission profile permits Apertus only; comparison and proprietary voice are disabled".into()));
        }
        if model == self.config.reference_model
            || model == self.config.realtime_model
            || model == self.config.transcription_model
        {
            return self
                .config
                .openai_key
                .as_deref()
                .map(|k| (self.config.openai_base.as_str(), k))
                .ok_or_else(|| CoreError::Unavailable("OPENAI_API_KEY is not configured".into()));
        }
        Err(CoreError::Invalid(
            "Model is not permitted by the configured route allowlist".into(),
        ))
    }
    pub fn models(&self) -> Value {
        let mut models: Vec<Value> = [
            "apertus-v1-5-8b",
            "apertus-v1-5-8b-thinking",
            "apertus-v1-5-70b",
            "apertus-v1-5-70b-thinking",
        ]
        .iter()
        .map(|m| self.model_metadata(m, "cscs", "text", true, self.config.cscs_key.is_some()))
        .collect();
        models.push(self.model_metadata(
            &self.config.reference_model,
            "openai-compatible",
            "text",
            false,
            self.config.openai_key.is_some(),
        ));
        models.push(self.model_metadata(
            &self.config.realtime_model,
            "openai-compatible",
            "audio",
            false,
            self.config.openai_key.is_some() && self.config.voice_enabled,
        ));
        json!({"data":models,"default_model":self.config.default_model,"automatic_fallback":false})
    }
    pub async fn chat(
        &self,
        model: &str,
        messages: Vec<Value>,
        tools: Option<Value>,
        json_mode: bool,
    ) -> Result<Value, CoreError> {
        if model == self.config.reference_model
            && (tools.is_some()
                || messages
                    .iter()
                    .any(|m| m.get("_responses_output").is_some()))
        {
            return super::responses::complete(self, model, messages, tools, json_mode).await;
        }
        let (base, key) = self.route(model)?;
        if tools.is_some() && !domain::supports_tools(model) {
            return Err(CoreError::Invalid(
                "Thinking routes do not permit provider tools".into(),
            ));
        }
        let first_tool_round =
            tools.is_some() && !messages.iter().any(|message| message["role"] == "tool");
        let mut body = json!({"model":model,"messages":messages,"stream":false});
        if model.starts_with("apertus-") {
            body["max_tokens"] = json!(self.config.output_tokens);
        } else {
            body["max_completion_tokens"] = json!(self.config.output_tokens);
        }
        if json_mode && tools.is_none() {
            body["response_format"] = json!({"type":"json_object"});
        }
        if let Some(tools) = tools {
            body["tools"] = tools;
            body["tool_choice"] = json!(if first_tool_round { "required" } else { "auto" });
        }
        let response = self
            .client
            .post(format!("{}/chat/completions", base.trim_end_matches('/')))
            .bearer_auth(key)
            .json(&body)
            .send()
            .await
            .map_err(CoreError::from)?;
        let status = response.status();
        self.record_access(model, status.as_u16(), "inference");
        let raw = response.bytes().await.map_err(CoreError::from)?;
        let audit = self
            .audit(model, "chat/completions", &body, status.as_u16(), &raw)
            .await?;
        if !status.is_success() {
            return Err(CoreError::Provider(status.as_u16()));
        }
        let mut body: Value = serde_json::from_slice(&raw).map_err(|_| CoreError::Transport)?;
        if body.pointer("/choices/0/message").is_none() {
            return Err(CoreError::Unavailable(
                "Provider returned no completion message".into(),
            ));
        }
        body["_oldif_receipt"] = audit;
        Ok(body)
    }
}
