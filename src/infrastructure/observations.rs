use super::provider::Provider;
use serde_json::{Value, json};
use std::collections::HashSet;
impl Provider {
    pub async fn refresh_catalogs(&self) {
        let mut last_attempt = self.catalog_refresh.lock().await;
        let interval = std::time::Duration::from_secs(self.config.catalog_refresh_secs);
        if last_attempt.is_some_and(|started| started.elapsed() < interval) {
            return;
        }
        *last_attempt = Some(std::time::Instant::now());
        self.probe_catalogs().await;
        // Waiting callers reuse even a probe that took longer than the interval.
        *last_attempt = Some(std::time::Instant::now());
    }
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
        let profile_permitted = self.config.execution_profile != "submission" || challenge;
        let available = configured
            && profile_permitted
            && !matches!(
                observation["access"].as_str(),
                Some("access_denied" | "provider_error" | "model_not_listed" | "invalid_catalog")
            );
        json!({"id":model,"name":model,"provider":provider,"tools":crate::domain::supports_tools(model),"modality":modality,"configured":configured,"available":available,"access":observation["access"],"observation":observation,"challenge_model":challenge,"unavailable_reason":if !profile_permitted{"Disabled by the Apertus-only submission profile"}else if !configured{"Provider credentials or modality are not configured"}else if observation["access"] == "model_not_listed"{"Model is not listed by the configured provider"}else if observation["access"] == "invalid_catalog"{"Provider returned an invalid model catalog"}else if !available{"Provider rejected the configured access; no replacement model is used"}else{""}})
    }
    pub async fn probe_catalogs(&self) {
        self.probe_catalog(
            "apertus-v1-5-8b",
            &[
                "apertus-v1-5-8b",
                "apertus-v1-5-8b-thinking",
                "apertus-v1-5-70b",
                "apertus-v1-5-70b-thinking",
            ],
            true,
        )
        .await;
        self.probe_catalog(
            self.config.reference_model.as_str(),
            &[
                self.config.reference_model.as_str(),
                self.config.realtime_model.as_str(),
            ],
            false,
        )
        .await;
    }
    async fn probe_catalog(&self, route: &str, models: &[&str], inspect_ids: bool) {
        let Ok((base, key)) = self.route(route) else {
            return;
        };
        let result = self
            .client
            .get(format!("{}/models", base.trim_end_matches('/')))
            .bearer_auth(key)
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await;
        let status = result
            .as_ref()
            .map(|response| response.status().as_u16())
            .unwrap_or(0);
        if inspect_ids && status == 200 {
            let catalog = match result {
                Ok(response) => response.json::<Value>().await.ok(),
                Err(_) => None,
            };
            self.record_apertus_catalog(models, catalog.as_ref());
        } else {
            for model in models {
                self.record_access(model, status, "catalog");
            }
        }
    }
    fn record_apertus_catalog(&self, models: &[&str], catalog: Option<&Value>) {
        let listed = catalog.and_then(catalog_ids);
        if let Ok(mut observations) = self.observations.write() {
            for model in models {
                let access = match &listed {
                    Some(ids) if ids.contains(model) => "catalog_reachable",
                    Some(_) => "model_not_listed",
                    None => "invalid_catalog",
                };
                observations.insert((*model).into(), json!({"access":access,"http_status":200,"operation":"catalog","observed_at":chrono::Utc::now().to_rfc3339()}));
            }
        }
    }
}

fn catalog_ids(catalog: &Value) -> Option<HashSet<&str>> {
    catalog
        .get("data")?
        .as_array()?
        .iter()
        .map(|model| {
            let id = model.get("id")?.as_str()?;
            (!id.trim().is_empty()).then_some(id)
        })
        .collect()
}
