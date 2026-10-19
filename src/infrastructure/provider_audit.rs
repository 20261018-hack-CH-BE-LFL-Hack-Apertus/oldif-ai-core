use super::provider::Provider;
use crate::error::CoreError;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

impl Provider {
    pub async fn audit(
        &self,
        model: &str,
        api: &str,
        request: &Value,
        status: u16,
        raw: &[u8],
    ) -> Result<Value, CoreError> {
        let parsed: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
        let mut configuration = json!({});
        for key in [
            "model",
            "stream",
            "max_tokens",
            "max_completion_tokens",
            "max_output_tokens",
            "response_format",
            "text",
            "reasoning",
            "tool_choice",
            "store",
        ] {
            if let Some(value) = request.get(key) {
                configuration[key] = value.clone();
            }
        }
        let tools = request.get("tools").cloned().unwrap_or(Value::Null);
        let record = json!({
            "schema_version":"1.0",
            "receipt_id":uuid::Uuid::new_v4().to_string(),
            "recorded_at":chrono::Utc::now().to_rfc3339(),
            "requested_model":model,
            "served_model":parsed.get("model"),
            "provider":if model.starts_with("apertus-") {"keera"} else {"openai-compatible"},
            "api":api,
            "execution_profile":self.config.execution_profile,
            "http_status":status,
            "raw_response_sha256":format!("{:x}",Sha256::digest(raw)),
            "raw_response_bytes":raw.len(),
            "request_sha256":format!("{:x}",Sha256::digest(request.to_string().as_bytes())),
            "tools_sha256":format!("{:x}",Sha256::digest(tools.to_string().as_bytes())),
            "configuration":configuration,
            "request_timeout_seconds":self.config.timeout_secs,
            "provider_revision":parsed.get("model_revision").or_else(|| parsed.get("revision")),
            "revision_status":if parsed.get("model_revision").or_else(|| parsed.get("revision")).is_some() {"provider_reported"} else {"not_exposed_by_provider"},
            "system_fingerprint":parsed.get("system_fingerprint"),
            "raw_content_retained":false,
            "credentials_retained":false
        });
        let directory = self.config.run_dir.join("provider-receipts");
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|_| CoreError::Storage)?;
        let path = directory.join(format!("{}.json", record["receipt_id"].as_str().unwrap()));
        let bytes = serde_json::to_vec(&record).map_err(|_| CoreError::Storage)?;
        tokio::fs::write(path, bytes)
            .await
            .map_err(|_| CoreError::Storage)?;
        Ok(record)
    }
}
