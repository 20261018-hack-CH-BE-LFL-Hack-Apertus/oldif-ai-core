use crate::{api::dto::Source, error::CoreError};
use serde_json::{Value, json};
pub fn definitions() -> Value {
    json!([
     {"type":"function","function":{"name":"search_sources","description":"Search only the approved source texts in this run. Returns exact passages with provenance.","parameters":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false}}},
     {"type":"function","function":{"name":"read_source","description":"Read one approved archival source by its supplied ID.","parameters":{"type":"object","properties":{"source_id":{"type":"string"}},"required":["source_id"],"additionalProperties":false}}}
    ])
}
pub fn for_sources(sources: &[Source], read_only: bool) -> Value {
    let mut functions = definitions().as_array().unwrap().clone();
    functions[1]["function"]["parameters"]["properties"]["source_id"]["enum"] =
        json!(sources.iter().map(|source| &source.id).collect::<Vec<_>>());
    if read_only {
        functions.remove(0);
    }
    json!(functions)
}
pub fn execute(name: &str, args: &Value, sources: &[Source]) -> Result<Value, CoreError> {
    match name {
        "read_source" => {
            check_argument_keys(args, "source_id")?;
            let id = args["source_id"]
                .as_str()
                .ok_or_else(|| CoreError::Invalid("Tool source_id must be a string".into()))?;
            let s = sources.iter().find(|s| s.id == id).ok_or_else(|| {
                CoreError::Invalid("Tool source is outside this run's scope".into())
            })?;
            Ok(super::passages::source_view(s))
        }
        "search_sources" => search(args, sources),
        _ => Err(CoreError::Invalid(
            "Tool denied: only search_sources and read_source are permitted".into(),
        )),
    }
}
fn check_argument_keys(args: &Value, required: &str) -> Result<(), CoreError> {
    let object = args
        .as_object()
        .ok_or_else(|| CoreError::Invalid("Tool arguments must be an object".into()))?;
    if object.len() != 1 || !object.contains_key(required) {
        return Err(CoreError::Invalid(format!(
            "Tool arguments must contain only the required {required} field"
        )));
    }
    Ok(())
}
fn search(args: &Value, sources: &[Source]) -> Result<Value, CoreError> {
    check_argument_keys(args, "query")?;
    let query = args["query"]
        .as_str()
        .filter(|q| !q.is_empty() && q.len() <= 500)
        .ok_or_else(|| CoreError::Invalid("Tool query requires 1–500 bytes".into()))?;
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut ranked: Vec<_> = sources
        .iter()
        .map(|s| {
            let text = format!("{} {}", s.title, s.text).to_lowercase();
            let score = terms.iter().filter(|t| text.contains(t.as_str())).count();
            (score, s)
        })
        .filter(|(score, _)| *score > 0)
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
    Ok(
        json!({"query":query,"retrieval":"lexical","matches":ranked.into_iter().take(8).map(|(score,s)|json!({"score":score,"source":s})).collect::<Vec<_>>()}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    fn sources() -> Vec<Source> {
        serde_json::from_value(json!([{
            "id":"fixture", "title":"Synthetic source", "text":"Arithmetic is proposed.",
            "url":"urn:oldif:synthetic:tool-arguments", "page":1,
            "sha256":"0000000000000000000000000000000000000000000000000000000000000000",
            "licence":"CC0-1.0"
        }]))
        .unwrap()
    }
    #[test]
    fn arbitrary_tools_are_denied() {
        assert!(execute("shell", &json!({"command":"echo unsafe"}), &[]).is_err());
    }
    #[test]
    fn cross_scope_source_denied() {
        assert!(execute("read_source", &json!({"source_id":"other-tenant"}), &[]).is_err());
    }
    #[test]
    fn read_source_rejects_additional_arguments() {
        let args = json!({"source_id":"fixture","unexpected":"unused"});
        assert!(matches!(
            execute("read_source", &args, &sources()),
            Err(CoreError::Invalid(_))
        ));
    }
    #[test]
    fn search_sources_rejects_additional_arguments() {
        let args = json!({"query":"Arithmetic","unexpected":{"nested":true}});
        assert!(matches!(
            execute("search_sources", &args, &sources()),
            Err(CoreError::Invalid(_))
        ));
    }
    #[test]
    fn non_object_arguments_are_rejected() {
        for args in [
            json!(null),
            json!([]),
            json!(true),
            json!(7),
            json!("fixture"),
        ] {
            for name in ["read_source", "search_sources"] {
                assert!(matches!(
                    execute(name, &args, &sources()),
                    Err(CoreError::Invalid(_))
                ));
            }
        }
    }
    #[test]
    fn missing_or_wrongly_typed_arguments_are_rejected() {
        for (name, key) in [("read_source", "source_id"), ("search_sources", "query")] {
            assert!(execute(name, &json!({}), &sources()).is_err());
            for value in [json!(null), json!(7), json!(false), json!([]), json!({})] {
                let args = Value::Object([(key.to_owned(), value)].into_iter().collect());
                assert!(matches!(
                    execute(name, &args, &sources()),
                    Err(CoreError::Invalid(_))
                ));
            }
        }
    }
    #[test]
    fn supported_arguments_preserve_exact_source_output() {
        let sources = sources();
        let read = execute("read_source", &json!({"source_id":"fixture"}), &sources).unwrap();
        assert_eq!(read, super::super::passages::source_view(&sources[0]));
        let search = execute("search_sources", &json!({"query":"Arithmetic"}), &sources).unwrap();
        assert_eq!(search["retrieval"], "lexical");
        assert_eq!(search["matches"][0]["source"], json!(sources[0]));
    }
    #[test]
    fn search_query_byte_limit_is_preserved() {
        assert!(
            execute(
                "search_sources",
                &json!({"query":"x".repeat(500)}),
                &sources()
            )
            .is_ok()
        );
        for query in [String::new(), "x".repeat(501), "é".repeat(251)] {
            assert!(execute("search_sources", &json!({"query":query}), &sources()).is_err());
        }
    }
}
