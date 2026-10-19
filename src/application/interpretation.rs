use crate::api::dto::RunRequest;
use serde_json::{Value, json};

/// Model verification is advisory and never substitutes for a human decision.
pub fn review(raw: Value) -> Value {
    if raw["verdict"] == "supported" {
        return json!({"verdict":"supported","authority":"advisory_model","output_policy":"extractive_pending_human_review","human_review_required":true,"independent_pass":false,"same_model":true});
    }
    let verdict = if raw["verdict"] == "abstain" {
        "abstain"
    } else {
        "needs_revision"
    };
    let reason = if matches!(raw["verdict"].as_str(), Some("needs_revision" | "abstain")) {
        "model_interpretation_not_verified"
    } else {
        "invalid_verification_verdict"
    };
    json!({"verdict":verdict,"reason":reason,"authority":"advisory_model","output_policy":"evidence_only","human_review_required":true,"independent_pass":false,"same_model":true})
}

pub fn bound(mut result: Value, request: &RunRequest) -> Value {
    if result["abstained"] == true {
        return result;
    }
    if result["verification"]["verdict"] == "supported" {
        result["interpretation_draft"] = json!({"answer":result["answer"],"entities":result["entities"],"timeline":result["timeline"],"state":"unreviewed","human_reviewed":false});
    }
    let notice = match request.locale.as_str() {
        "es" => {
            "Los pasajes seleccionados se muestran para revisión humana; no se pudo verificar la interpretación del modelo."
        }
        "de" | "de-CH" => {
            "Die ausgewählten Quellenpassagen werden zur menschlichen Prüfung angezeigt. Die Interpretation des Modells konnte nicht verifiziert werden."
        }
        "fr" => {
            "Les passages sélectionnés sont présentés pour examen humain ; l’interprétation du modèle n’a pas pu être vérifiée."
        }
        "it" => {
            "I passaggi selezionati sono mostrati per la revisione umana; l’interpretazione del modello non è stata verificata."
        }
        "rm" => {
            "Las passaschas tschernidas vegnan mussadas per ina controlla umana; l’interpretaziun dal model n’ha betg pudì vegnir verifitgada."
        }
        _ => {
            "The selected source passages are shown for human review; the model interpretation could not be verified."
        }
    };
    let passages = result["citations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|citation| citation["quote"].as_str().map(|quote| format!("“{quote}”")))
        .collect::<Vec<_>>()
        .join("\n\n");
    result["answer"] = json!(format!("{notice}\n\n{passages}"));
    result["uncertainty"] = json!([notice]);
    result["entities"] = json!([]);
    result["timeline"] = json!([]);
    result["proposals"] = json!([]);
    result["verification"] = review(result["verification"].clone());
    result["interpretation_status"] = json!("needs_review");
    result["evidence_only"] = json!(true);
    result["human_review_required"] = json!(true);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(locale: &str) -> RunRequest {
        serde_json::from_value(json!({"question":"Subjects?","locale":locale,"model":"apertus-v1-5-8b","sources":[],"session_id":"fixture","user_id":"fixture"})).unwrap()
    }
    #[test]
    fn unverified_summary_is_removed_while_exact_evidence_survives() {
        let citations = json!([{"source_id":"stapfer-2027","passage_id":"p048","quote":"diese sollen im schreiben läsen und rechnen unterrichtet werden","sha256":"original-record-hash","text_sha256":"original-text-hash"}]);
        let result = bound(
            json!({"answer":"Latin and catechesis were taught","citations":citations,"entities":[{"name":"Latin"}],"timeline":[{"description":"catechesis"}],"uncertainty":["Latin uncertain"],"proposals":[{"description":"catechesis"}],"abstained":false,"verification":{"verdict":"needs_revision","findings":["Latin"],"contradictions":["catechesis"]}}),
            &request("en"),
        );
        assert_eq!(result["citations"], citations);
        assert_eq!(result["evidence_only"], true);
        assert_eq!(result["interpretation_status"], "needs_review");
        assert_eq!(result["abstained"], false);
        assert_eq!(result["proposals"], json!([]));
        assert!(!result.to_string().contains("Latin"));
        assert!(!result.to_string().contains("catechesis"));
    }
    #[test]
    fn abstaining_and_invalid_verdicts_cannot_publish_free_text() {
        for verdict in [json!("abstain"), json!("Latin was taught"), Value::Null] {
            let result = review(json!({"verdict":verdict,"findings":["Latin"]}));
            assert_ne!(result["verdict"], "supported");
            assert!(!result.to_string().contains("Latin"));
        }
    }
    #[test]
    fn canonical_abstention_remains_distinct_from_evidence_only() {
        let result = json!({"answer":"Cannot verify","citations":[],"abstained":true,"verification":{"verdict":"abstain"}});
        assert_eq!(bound(result.clone(), &request("en")), result);
    }
    #[test]
    fn evidence_notice_uses_requested_locale() {
        let result = bound(
            json!({"citations":[],"verification":{"verdict":"needs_revision"}}),
            &request("es"),
        );
        assert!(
            result["answer"]
                .as_str()
                .unwrap()
                .starts_with("Los pasajes")
        );
    }
    #[test]
    fn same_model_support_cannot_release_unsupported_historical_interpretation() {
        let quote = "diese sollen im schreiben läsen und rechnen unterrichtet werden";
        let result = bound(
            json!({"answer":"Latin was successfully taught by an invented author","citations":[{"source_id":"stapfer-2027","quote":quote}],"entities":[{"name":"invented author"}],"timeline":[{"description":"Latin success"}],"abstained":false,"verification":{"verdict":"supported"}}),
            &request("en"),
        );
        assert_eq!(result["human_review_required"], true);
        assert_eq!(result["evidence_only"], true);
        assert!(result["answer"].as_str().unwrap().contains(quote));
        assert!(!result["answer"].as_str().unwrap().contains("Latin"));
        assert_eq!(result["entities"], json!([]));
        assert_eq!(result["timeline"], json!([]));
        assert_eq!(result["interpretation_status"], "needs_review");
    }
}
