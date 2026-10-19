use crate::api::dto::RunRequest;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    kind: String,
    subject: String,
    predicate: String,
    value: String,
    source_id: String,
    passage_id: String,
}

/// Typed, attributed proposals remain separate from accepted historical claims.
pub fn ground(candidate: &Value, request: &RunRequest) -> (Vec<Value>, Value) {
    let mut accepted = Vec::new();
    let mut reasons = Vec::new();
    let mut ids = HashSet::new();
    if let Some(input) = candidate.get("assertions") {
        if let Some(items) = input.as_array().filter(|items| items.len() <= 30) {
            for item in items {
                match validate(item, request) {
                    Ok(value) if ids.insert(value["assertion_id"].as_str().unwrap().to_owned()) => {
                        accepted.push(value)
                    }
                    Ok(_) => reasons.push("duplicate_assertion"),
                    Err(reason) => reasons.push(reason),
                }
            }
        } else {
            reasons.push("assertions_must_be_an_array_of_at_most_30");
        }
    }
    flag_conflicts(&mut accepted);
    let receipt = json!({"accepted":accepted.len(),"rejected":reasons.len(),"reasons":reasons,"policy":"exact_span_proposals_not_historical_truth"});
    (accepted, receipt)
}

fn validate(input: &Value, request: &RunRequest) -> Result<Value, &'static str> {
    let p: Proposal =
        serde_json::from_value(input.clone()).map_err(|_| "invalid_assertion_shape")?;
    validate_type(&p)?;
    let source = request
        .sources
        .iter()
        .find(|s| s.id == p.source_id)
        .ok_or("unknown_assertion_source")?;
    let passage = super::passages::resolve(source, &json!({"passage_id":p.passage_id}))
        .map_err(|_| "unknown_assertion_passage")?
        .ok_or("missing_assertion_passage")?;
    let byte_start = passage
        .text
        .find(&p.value)
        .ok_or("assertion_value_absent_from_passage")?;
    if p.subject != source.id && !passage.text.contains(&p.subject) {
        return Err("assertion_subject_absent_from_passage");
    }
    let start = passage.char_start + passage.text[..byte_start].chars().count();
    let mut evidence = json!({"source_id":source.id,"passage_id":passage.passage_id,"quote":p.value,"char_start":start,"char_end":start+p.value.chars().count(),"offset_unit":"unicode_scalar_value","sha256":source.sha256,"text_sha256":super::validation::source_text_hash(source),"title":source.title,"url":source.url,"page":source.page,"licence":source.licence,"record_hash_scope":source.record_hash_scope.as_deref().unwrap_or("unspecified_input_provenance")});
    super::page_locator::apply(&mut evidence, source);
    if let Some(method) = &source.transcription_method {
        evidence["transcription_method"] = json!(method);
        evidence["transcription_review_status"] = json!(
            source
                .transcription_review_status
                .as_deref()
                .unwrap_or("unreviewed")
        );
    }
    let identity = json!([
        p.kind,
        p.subject,
        p.predicate,
        p.value,
        source.id,
        evidence["text_sha256"],
        start
    ]);
    Ok(
        json!({"assertion_id":format!("{:x}",Sha256::digest(identity.to_string().as_bytes())),"kind":p.kind,"subject":p.subject,"predicate":p.predicate,"value":p.value,"source_id":source.id,"state":"proposed","human_reviewed":false,"evidence":evidence,"potential_conflict":false,"conflict_group":null}),
    )
}

fn validate_type(p: &Proposal) -> Result<(), &'static str> {
    if [&p.subject, &p.predicate, &p.value]
        .iter()
        .any(|s| s.trim().is_empty() || s.len() > 800 || s.chars().any(char::is_control))
    {
        return Err("invalid_assertion_text");
    }
    let allowed = match p.kind.as_str() {
        "title" => p.predicate == "document_title",
        "date" => {
            matches!(p.predicate.as_str(), "date_mentioned" | "event_date") && valid_date(&p.value)
        }
        "entity" => matches!(
            p.predicate.as_str(),
            "person" | "place" | "organization" | "concept"
        ),
        "relation" => matches!(
            p.predicate.as_str(),
            "related_to" | "located_in" | "part_of" | "reported_by"
        ),
        _ => false,
    };
    allowed
        .then_some(())
        .ok_or("invalid_assertion_type_or_predicate")
}

fn valid_date(value: &str) -> bool {
    (value.len() == 4 && value.bytes().all(|c| c.is_ascii_digit()) && value != "0000")
        || chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
}

fn flag_conflicts(assertions: &mut [Value]) {
    for i in 0..assertions.len() {
        let conflict = assertions.iter().enumerate().any(|(j, other)| {
            i != j
                && assertions[i]["kind"] == other["kind"]
                && assertions[i]["subject"] == other["subject"]
                && assertions[i]["predicate"] == other["predicate"]
                && assertions[i]["source_id"] != other["source_id"]
                && assertions[i]["value"] != other["value"]
        });
        if conflict {
            let key = json!([
                assertions[i]["kind"],
                assertions[i]["subject"],
                assertions[i]["predicate"]
            ]);
            assertions[i]["potential_conflict"] = json!(true);
            assertions[i]["conflict_group"] =
                json!(format!("{:x}", Sha256::digest(key.to_string().as_bytes())));
        }
    }
}
