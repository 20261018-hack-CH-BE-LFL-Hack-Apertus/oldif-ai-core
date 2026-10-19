use crate::api::dto::Source;
use serde_json::{Value, json};
use std::collections::HashSet;

pub const ANSWER_SCHEMA: &str = r#"{"answer":string,"citations":[{"source_id":string,"passage_id":string}],"entities":[{"name":string,"type":string,"source_id":string}],"timeline":[{"date":string,"description":string,"source_id":string}],"uncertainty":[string],"abstained":boolean}"#;

pub fn catalog(mut context: Value, sources: &[Source]) -> Value {
    context["sources"] = json!(sources.iter().map(|source|json!({"id":source.id,"title":source.title,"url":source.url,"page":source.page,"licence":source.licence,"sha256":source.sha256})).collect::<Vec<_>>());
    context["source_text_available_via"] = json!("read_source");
    context
}

pub fn reviewed(mut context: Value, sources: &[Source], candidate: &Value) -> Value {
    let ids: HashSet<&str> = candidate["citations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["source_id"].as_str())
        .collect();
    let abstained = candidate["abstained"].as_bool().unwrap_or(false);
    context["sources"] = json!(
        sources
            .iter()
            .filter(|source| !abstained && ids.contains(source.id.as_str()))
            .collect::<Vec<_>>()
    );
    // The candidate already carries the conversational intent. Previous answers are not evidence.
    context["prior_conversation_data"] = json!([]);
    context["candidate_abstained"] = json!(abstained);
    context
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(id: &str, text: &str) -> Source {
        serde_json::from_value(json!({"id":id,"title":id,"text":text,"url":"https://example.org","page":null,"sha256":"a".repeat(64),"licence":"CC0"})).unwrap()
    }
    #[test]
    fn catalog_never_preloads_other_records_text() {
        let context = catalog(
            json!({"question":"Read s1","sources":[]}),
            &[source("s1", "private source text")],
        );
        assert_eq!(context["sources"][0]["id"], "s1");
        assert!(context["sources"][0].get("text").is_none());
    }
    #[test]
    fn review_uses_only_cited_sources_and_no_prior_answers() {
        let context = reviewed(
            json!({"prior_conversation_data":[{"role":"assistant","content":"false earlier answer"}]}),
            &[source("s1", "one"), source("s2", "two")],
            &json!({"citations":[{"source_id":"s2"}],"abstained":false}),
        );
        assert_eq!(context["sources"].as_array().unwrap().len(), 1);
        assert_eq!(context["sources"][0]["id"], "s2");
        assert_eq!(context["prior_conversation_data"], json!([]));
    }
    #[test]
    fn abstention_has_no_historical_source_context() {
        let context = reviewed(
            json!({}),
            &[source("s1", "one")],
            &json!({"citations":[{"source_id":"s1"}],"abstained":true}),
        );
        assert_eq!(context["sources"], json!([]));
    }
}
