use super::{assertions, interpretation, validation};
use crate::api::dto::RunRequest;
use serde_json::{Value, json};

fn request() -> RunRequest {
    serde_json::from_value(json!({"question":"Compare the school dates","locale":"en","model":"apertus-v1-5-70b","session_id":"fixture","user_id":"fixture","sources":[
        {"id":"a","title":"A","text":"École opened in 1799.","url":"https://example.org/a","page":null,"sha256":"a".repeat(64),"licence":"CC0"},
        {"id":"b","title":"B","text":"École opened in 1800.","url":"https://example.org/b","page":null,"sha256":"b".repeat(64),"licence":"CC0"}
    ]})).unwrap()
}
fn proposal(source: &str, value: &str) -> Value {
    json!({"kind":"date","subject":"École","predicate":"event_date","value":value,"source_id":source,"passage_id":"p001"})
}

#[test]
fn exact_unicode_spans_and_canonical_provenance_are_preserved() {
    let request = request();
    let (items, receipt) =
        assertions::ground(&json!({"assertions":[proposal("a","1799")]}), &request);
    assert_eq!(receipt["accepted"], 1);
    let evidence = &items[0]["evidence"];
    assert_eq!(evidence["char_start"], 16);
    assert_eq!(evidence["char_end"], 20);
    assert_eq!(evidence["offset_unit"], "unicode_scalar_value");
    assert_eq!(evidence["sha256"], "a".repeat(64));
    assert_eq!(
        evidence["text_sha256"],
        validation::source_text_hash(&request.sources[0])
    );
    assert_eq!(items[0]["state"], "proposed");
    assert_eq!(items[0]["human_reviewed"], false);
}

#[test]
fn invented_values_sources_subjects_and_authority_are_rejected() {
    let original = proposal("a", "1799");
    let mut bad = Vec::new();
    for (key, value) in [
        ("source_id", json!("private")),
        ("passage_id", json!("p999")),
        ("value", json!("1700")),
        ("subject", json!("Invented person")),
        ("human_reviewed", json!(true)),
        ("kind", json!("approved")),
        ("predicate", json!("guessed")),
    ] {
        let mut item = original.clone();
        item[key] = value;
        bad.push(item);
    }
    let (items, receipt) = assertions::ground(&json!({"assertions":bad}), &request());
    assert!(items.is_empty());
    assert_eq!(receipt["rejected"], 7);
    assert!(!receipt.to_string().contains("Invented person"));
}

#[test]
fn conflicts_retain_both_assertions_and_stable_identifiers() {
    let input = vec![proposal("a", "1799"), proposal("b", "1800")];
    let (items, _) = assertions::ground(&json!({"assertions":input}), &request());
    let (reversed, _) = assertions::ground(&json!({"assertions":[input[1],input[0]]}), &request());
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["potential_conflict"], true);
    assert_eq!(items[1]["potential_conflict"], true);
    assert_eq!(items[0]["conflict_group"], items[1]["conflict_group"]);
    assert_eq!(items[0]["assertion_id"], reversed[1]["assertion_id"]);
    assert_eq!(items[1]["assertion_id"], reversed[0]["assertion_id"]);
    assert_ne!(
        items[0]["evidence"]["sha256"],
        items[1]["evidence"]["sha256"]
    );
}

#[test]
fn duplicate_assertions_do_not_create_false_conflicts() {
    let (items, receipt) = assertions::ground(
        &json!({"assertions":[proposal("a","1799"),proposal("a","1799")]}),
        &request(),
    );
    assert_eq!(items.len(), 1);
    assert_eq!(receipt["reasons"], json!(["duplicate_assertion"]));
    assert_eq!(items[0]["potential_conflict"], false);
    assert_eq!(items[0]["conflict_group"], Value::Null);
}

#[test]
fn types_and_bounds_are_checked_before_proposal_publication() {
    let mut request = request();
    request.sources[0].text = "École 1799-02-30".into();
    let (_, receipt) = assertions::ground(
        &json!({"assertions":[proposal("a","1799-02-30")]}),
        &request,
    );
    assert_eq!(receipt["rejected"], 1);
    for input in [json!(true), json!(vec![proposal("a", "1799"); 31])] {
        let (items, receipt) = assertions::ground(&json!({"assertions":input}), &request);
        assert!(items.is_empty());
        assert_eq!(receipt["rejected"], 1);
    }
}

#[test]
fn legacy_untyped_items_never_enter_the_review_draft() {
    let request = request();
    let mut result = validation::ground(json!({"answer":"Unsupported consensus","citations":[{"source_id":"a","passage_id":"p001"}],"entities":[{"source_id":"a","name":"Invented person"}],"timeline":[{"source_id":"a","date":"1700"}],"assertions":[proposal("a","1799"),proposal("b","1800")]}), &request).unwrap();
    result["verification"] = json!({"verdict":"supported"});
    let bounded = interpretation::bound(result, &request);
    assert_eq!(bounded["entities"], json!([]));
    assert_eq!(bounded["interpretation_draft"]["entities"], json!([]));
    assert_eq!(bounded["assertions"].as_array().unwrap().len(), 2);
    assert_eq!(bounded["evidence_only"], true);
    assert!(!bounded["answer"].as_str().unwrap().contains("consensus"));
    assert!(!bounded.to_string().contains("Invented person"));
}

#[test]
fn explicit_abstention_cannot_smuggle_assertions() {
    let value = validation::ground(
        json!({"abstained":true,"assertions":[proposal("a","1799")]}),
        &request(),
    )
    .unwrap();
    assert!(value.get("assertions").is_none());
    assert_eq!(value["abstained"], true);
}
