use crate::{api::dto::RunRequest, config::Config, domain, error::CoreError};
use serde_json::{Value, json};
use std::collections::HashSet;
pub fn request(r: &RunRequest, c: &Config) -> Result<(), CoreError> {
    if r.question.trim().is_empty() || r.question.len() > 8000 {
        return Err(CoreError::Invalid(
            "Question must contain 1–8000 bytes".into(),
        ));
    }
    if !domain::valid_locale(&r.locale) {
        return Err(CoreError::Invalid("Unsupported locale".into()));
    }
    if r.sources.is_empty() || r.sources.len() > c.max_sources {
        return Err(CoreError::Invalid(
            "Select a nonempty, bounded set of archival sources".into(),
        ));
    }
    if r.history.len() > 20
        || serde_json::to_vec(r)
            .map_err(|_| CoreError::Invalid("Invalid request".into()))?
            .len()
            > c.max_input_bytes
    {
        return Err(CoreError::Invalid(
            "Context exceeds configured byte budget".into(),
        ));
    }
    let mut ids = HashSet::new();
    for s in &r.sources {
        if !ids.insert(&s.id) || s.id.is_empty() || s.text.is_empty() || s.licence.is_empty() {
            return Err(CoreError::Invalid(
                "Sources require unique IDs, text and licence provenance".into(),
            ));
        }
        if s.sha256.len() != 64 || !s.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(CoreError::Invalid(
                "Every source requires an original SHA-256 hash".into(),
            ));
        }
        if !s.url.starts_with("https://") && !s.url.starts_with("http://") {
            return Err(CoreError::Invalid(
                "Every source requires an HTTP(S) provenance URL".into(),
            ));
        }
    }
    Ok(())
}
pub fn parse_json(content: &str) -> Result<Value, CoreError> {
    let text = content.trim();
    let text = if text.starts_with("```") {
        text.lines()
            .skip(1)
            .collect::<Vec<_>>()
            .join(
                "
",
            )
            .trim_end_matches("```")
            .trim()
            .to_string()
    } else {
        text.to_string()
    };
    let value: Value = serde_json::from_str(&text).map_err(|_| {
        CoreError::Invalid(
            "Model response did not satisfy the JSON output contract; retry explicitly".into(),
        )
    })?;
    if !value.is_object() {
        return Err(CoreError::Invalid("Model JSON must be an object".into()));
    }
    Ok(value)
}
pub fn ground(candidate: Value, request: &RunRequest) -> Result<Value, CoreError> {
    let answer = candidate
        .get("answer")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CoreError::Invalid("Model response requires answer text".into()))?;
    let input = candidate
        .get("citations")
        .and_then(Value::as_array)
        .ok_or_else(|| CoreError::Invalid("Model response requires a citations array".into()))?;
    let mut citations = vec![];
    let mut failures = vec![];
    for cite in input.iter().take(30) {
        let id = cite["source_id"].as_str().unwrap_or_default();
        let quote = cite["quote"].as_str().unwrap_or_default();
        if let Some(source) = request.sources.iter().find(|s| s.id == id) {
            if domain::cited_quote_exists(&source.text, quote) {
                citations.push(json!({"source_id":id,"quote":quote,"page":source.page,"url":source.url,"title":source.title,"sha256":source.sha256,"licence":source.licence,"locator_kind":if source.page.is_some(){"page"}else{"record"}}));
                continue;
            }
        }
        failures.push(format!(
            "Citation {} has an unknown source or a quote absent from the source",
            citations.len() + failures.len() + 1
        ));
    }
    if !failures.is_empty() {
        return Err(CoreError::Invalid(failures.join("; ")));
    }
    let abstained = candidate["abstained"].as_bool().unwrap_or(false);
    if citations.is_empty() && !abstained {
        return Err(CoreError::Invalid(
            "Unsupported answer: citations are required unless the model explicitly abstains"
                .into(),
        ));
    }
    Ok(
        json!({"answer":answer,"citations":citations,"entities":grounded_items(&candidate["entities"],request),"timeline":grounded_items(&candidate["timeline"],request),"uncertainty":candidate.get("uncertainty").cloned().unwrap_or(json!([])),"abstained":abstained,"source_fidelity":"exact_quotes_checked_human_review_required","model":request.model,"locale":request.locale}),
    )
}
fn grounded_items(items: &Value, request: &RunRequest) -> Vec<Value> {
    items
        .as_array()
        .into_iter()
        .flatten()
        .take(30)
        .filter(|item| {
            item["source_id"]
                .as_str()
                .is_some_and(|id| request.sources.iter().any(|s| s.id == id))
        })
        .cloned()
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    fn req() -> RunRequest {
        RunRequest {
            question: "When?".into(),
            locale: "en".into(),
            model: "test".into(),
            sources: vec![crate::api::dto::Source {
                id: "s1".into(),
                title: "Source".into(),
                text: "School in 1799".into(),
                url: "https://example.org".into(),
                page: Some(1),
                sha256: "a".repeat(64),
                licence: "CC0".into(),
            }],
            session_id: "s".into(),
            user_id: "u".into(),
            history: vec![],
            idempotency_key: None,
        }
    }
    #[test]
    fn rejects_invented_quote() {
        assert!(
            ground(
                json!({"answer":"1798","citations":[{"source_id":"s1","quote":"1798"}]}),
                &req()
            )
            .is_err()
        );
    }
    #[test]
    fn retains_original_provenance() {
        let v=ground(json!({"answer":"1799","citations":[{"source_id":"s1","quote":"1799","url":"https://evil.example"}]}),&req()).unwrap();
        assert_eq!(v["citations"][0]["url"], "https://example.org");
        assert_eq!(v["citations"][0]["page"], 1);
    }
    #[test]
    fn unsupported_answer_requires_abstention() {
        assert!(ground(json!({"answer":"Unknown","citations":[]}), &req()).is_err());
        assert!(
            ground(
                json!({"answer":"Unknown","citations":[],"abstained":true}),
                &req()
            )
            .is_ok()
        );
    }
}

/// Exhausted output validation yields an explicit refusal to assert historical facts.
/// It never turns a rejected quote into a normalized or silently substituted quote.
pub fn abstention(request: &RunRequest, reason: &str) -> Value {
    let answer = match request.locale.as_str() {
        "es" => {
            "No puedo ofrecer una respuesta verificada a partir de estas fuentes. Revise los documentos originales."
        }
        "de" | "de-CH" => {
            "Ich kann aus diesen Quellen keine verifizierte Antwort geben. Bitte prüfen Sie die Originaldokumente."
        }
        "fr" => {
            "Je ne peux pas fournir de réponse vérifiée à partir de ces sources. Veuillez consulter les documents originaux."
        }
        "it" => {
            "Non posso fornire una risposta verificata sulla base di queste fonti. Consultare i documenti originali."
        }
        "rm" => {
            "Jau na poss betg dar ina resposta verifitgada a basa da questas funtaunas. Consultai ils documents originals."
        }
        _ => {
            "I cannot provide a verified answer from these sources. Please review the original records."
        }
    };
    json!({"answer":answer,"citations":[],"entities":[],"timeline":[],"uncertainty":[answer],"abstained":true,"human_review_required":true,"system_validation":{"accepted":false,"reason":reason,"repair_exhausted":true}})
}

#[cfg(test)]
mod abstention_tests {
    use super::*;
    #[test]
    fn exhausted_grounding_abstains_without_publishing_rejected_claims() {
        let request:RunRequest=serde_json::from_value(json!({"question":"Q","locale":"es","model":"apertus-v1-5-8b","sources":[],"session_id":"fixture","user_id":"fixture"})).unwrap();
        let result = abstention(&request, "quote absent from source");
        assert_eq!(result["abstained"], true);
        assert_eq!(result["system_validation"]["accepted"], false);
        assert_eq!(result["citations"], json!([]));
        assert!(result["answer"].as_str().unwrap().starts_with("No puedo"));
        assert!(
            result["answer"]
                .as_str()
                .unwrap()
                .find("quote absent")
                .is_none()
        );
    }
}
