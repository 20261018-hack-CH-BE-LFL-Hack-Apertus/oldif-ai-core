use crate::{error::CoreError, infrastructure::provider::Provider};
use serde_json::{Value, json};
pub async fn complete(
    p: &Provider,
    model: &str,
    messages: Vec<Value>,
    tools: Option<Value>,
) -> Result<Value, CoreError> {
    let (base, key) = p.route(model)?;
    let mut body = json!({"model":model,"input":input_items(messages),"max_output_tokens":p.config.output_tokens,"store":false,"reasoning":{"effort":"low"},"include":["reasoning.encrypted_content"]});
    if let Some(tools) = tools {
        body["tools"]=json!(tools.as_array().into_iter().flatten().map(|t|json!({"type":"function","name":t["function"]["name"],"description":t["function"]["description"],"parameters":t["function"]["parameters"],"strict":true})).collect::<Vec<_>>());
    }
    let response = p
        .client
        .post(format!("{}/responses", base.trim_end_matches('/')))
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|_| CoreError::Transport)?;
    p.record_access(model, response.status().as_u16(), "inference");
    if !response.status().is_success() {
        return Err(CoreError::Provider(response.status().as_u16()));
    }
    let value: Value = response.json().await.map_err(|_| CoreError::Transport)?;
    let output = value["output"].as_array().ok_or(CoreError::Transport)?;
    let text = output
        .iter()
        .filter(|v| v["type"] == "message")
        .flat_map(|v| v["content"].as_array().into_iter().flatten())
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("");
    let calls:Vec<Value>=output.iter().filter(|v|v["type"]=="function_call").map(|v|json!({"id":v["call_id"],"type":"function","function":{"name":v["name"],"arguments":v["arguments"]}})).collect();
    let message =
        json!({"role":"assistant","content":text,"tool_calls":calls,"_responses_output":output});
    Ok(
        json!({"id":value["id"],"model":value["model"],"usage":value["usage"],"api":"responses","choices":[{"message":message}]}),
    )
}
fn input_items(messages: Vec<Value>) -> Vec<Value> {
    let mut input = vec![];
    for message in messages {
        if let Some(items) = message["_responses_output"].as_array() {
            input.extend(items.iter().cloned());
            continue;
        }
        if message["role"] == "tool" {
            input.push(json!({"type":"function_call_output","call_id":message["tool_call_id"],"output":message["content"]}));
        } else {
            input.push(json!({"role":message["role"],"content":message["content"]}));
        }
    }
    input
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_reasoning_and_tool_continuation() {
        let v = input_items(vec![
            json!({"role":"assistant","_responses_output":[{"type":"reasoning","id":"r"},{"type":"function_call","call_id":"c"}]}),
            json!({"role":"tool","tool_call_id":"c","content":"result"}),
        ]);
        assert_eq!(v.len(), 3);
        assert_eq!(v[2]["call_id"], "c");
    }
}
