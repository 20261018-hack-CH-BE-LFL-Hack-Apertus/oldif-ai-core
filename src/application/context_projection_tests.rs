use super::{context, passages};
use crate::api::dto::{Event, Source};
use serde_json::{Value, json};

fn source() -> Source {
    let original: Value =
        serde_json::from_str(include_str!("../../evals/fixtures/stapfer-2027.json")).unwrap();
    serde_json::from_value(json!({"id":original["id"],"title":original["title"],"text":original["text"],"url":original["url"],"page":original["page"],"sha256":original["sha256"],"licence":original["licence"]})).unwrap()
}

#[test]
fn compact_projection_keeps_every_exact_passage_without_mutating_originals() {
    let source = source();
    let original = serde_json::to_value(&source).unwrap();
    let view = passages::source_view(&source);
    let preserved = view.clone();
    let packed = context::model_source(&source, true);
    assert_eq!(packed["id"], source.id);
    assert_eq!(packed["title"], source.title);
    assert_eq!(packed.as_object().unwrap().len(), 3);
    let full = view["passages"].as_array().unwrap();
    let compact = packed["passages"].as_array().unwrap();
    assert_eq!(compact.len(), 50);
    assert_eq!(compact.len(), full.len());
    for (compact, full) in compact.iter().zip(full) {
        assert_eq!(compact["passage_id"], full["passage_id"]);
        assert_eq!(compact["text"], full["text"]);
        assert_eq!(compact.as_object().unwrap().len(), 2);
        assert!(full["char_start"].is_number());
        assert!(full["char_end"].is_number());
    }
    assert_eq!(view, preserved);
    assert_eq!(serde_json::to_value(&source).unwrap(), original);
    assert_eq!(view["sha256"], source.sha256);
    assert_eq!(view["text_sha256"].as_str().unwrap().len(), 64);
}

#[test]
fn standard_source_views_and_other_tool_results_are_unchanged() {
    let source = source();
    let view = passages::source_view(&source);
    assert_eq!(context::model_source(&source, false), view);
    assert_eq!(context::model_view(&view, false), view);
    for result in [
        json!({"error":"tool_budget_exhausted","executed":false}),
        json!({"results":[{"id":"s1","excerpt":"exact text"}]}),
    ] {
        assert_eq!(context::model_view(&result, true), result);
        assert_eq!(context::model_view(&result, false), result);
    }
}

fn read_event(id: &str) -> Event {
    serde_json::from_value(json!({"run_id":"fixture","sequence":1,"kind":"tool.completed","agent":"researcher","data":{"tool":"read_source","result":{"id":id}},"created_at":"2026-10-21T00:00:00Z"})).unwrap()
}

#[test]
fn repair_context_keeps_read_scope_and_projects_only_model_source_views() {
    let source = source();
    let mut other = source.clone();
    other.id = "unread-source".into();
    let sources = [source, other];
    let event = read_event(&sources[0].id);
    let context = json!({"question":"Find evidence","prior_conversation_data":["old answer"]});
    let candidate = json!({"citations":[{"source_id":"stapfer-2027","passage_id":"p999"}]});
    for compact in [true, false] {
        let result = context::for_repair(
            context.clone(),
            &sources,
            std::slice::from_ref(&event),
            candidate.clone(),
            "unknown passage",
            compact,
        );
        assert_eq!(result["sources"].as_array().unwrap().len(), 1);
        assert_eq!(
            result["sources"][0],
            context::model_source(&sources[0], compact)
        );
        assert_eq!(result["rejected_candidate"], candidate);
        assert_eq!(result["validation_error"], "unknown passage");
        assert_eq!(result["prior_conversation_data"], json!([]));
    }
}
