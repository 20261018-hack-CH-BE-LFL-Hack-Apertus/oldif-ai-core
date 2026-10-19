use super::provider::Provider;
use serde_json::{Value, json};
impl Provider {
    pub fn record_access(&self, model: &str, status: u16, operation: &str) {
        let access = if status == 200 {
            if operation == "inference" {
                "verified_inference"
            } else {
                "catalog_reachable"
            }
        } else if status == 401 || status == 403 {
            "access_denied"
        } else {
            "provider_error"
        };
        if let Ok(mut observations) = self.observations.write() {
            observations.insert(model.into(),json!({"access":access,"http_status":status,"operation":operation,"observed_at":chrono::Utc::now().to_rfc3339()}));
        }
    }
    pub fn model_metadata(
        &self,
        model: &str,
        provider: &str,
        modality: &str,
        challenge: bool,
        configured: bool,
    ) -> Value {
        let observation = self
            .observations
            .read()
            .ok()
            .and_then(|m| m.get(model).cloned())
            .unwrap_or(json!({"access":"not_probed"}));
        let available = configured
            && !matches!(
                observation["access"].as_str(),
                Some("access_denied" | "provider_error")
            );
        json!({"id":model,"name":model,"provider":provider,"tools":crate::domain::supports_tools(model),"modality":modality,"configured":configured,"available":available,"access":observation["access"],"observation":observation,"challenge_model":challenge,"unavailable_reason":if !configured{"Provider credentials or modality are not configured"}else if !available{"Provider rejected the configured access; no replacement model is used"}else{""}})
    }
    pub async fn probe_catalogs(&self) {
        let routes = [
            (
                "apertus-v1-5-8b",
                vec![
                    "apertus-v1-5-8b",
                    "apertus-v1-5-8b-thinking",
                    "apertus-v1-5-70b",
                    "apertus-v1-5-70b-thinking",
                ],
            ),
            (
                self.config.reference_model.as_str(),
                vec![
                    self.config.reference_model.as_str(),
                    self.config.realtime_model.as_str(),
                ],
            ),
        ];
        for (route, models) in routes {
            if let Ok((base, key)) = self.route(route) {
                let result = self
                    .client
                    .get(format!("{}/models", base.trim_end_matches('/')))
                    .bearer_auth(key)
                    .timeout(std::time::Duration::from_secs(8))
                    .send()
                    .await;
                let status = result.map(|r| r.status().as_u16()).unwrap_or(0);
                for model in models {
                    self.record_access(model, status, "catalog");
                }
            }
        }
    }
}
