use super::compact_prompt_test_support::{Candidate, Fixture};
use serde_json::{Value, json};

fn instruction(request: &Value) -> &str {
    request["messages"][0]["content"].as_str().unwrap()
}

fn assert_model_source(source: &Value) {
    assert_eq!(source.as_object().unwrap().len(), 3);
    assert_eq!(source["id"], "s1");
    assert_eq!(
        source["passages"],
        json!([{"passage_id":"p001","text":"School in 1799"}])
    );
}

fn assert_compact_source_request(request: &Value) {
    let mut checked = 0;
    for message in request["messages"].as_array().unwrap() {
        let Ok(content) = serde_json::from_str::<Value>(message["content"].as_str().unwrap_or(""))
        else {
            continue;
        };
        if content["passages"].is_array() {
            assert_model_source(&content);
            checked += 1;
        }
        for source in content["sources"].as_array().into_iter().flatten() {
            if source["passages"].is_array() {
                assert_model_source(source);
                checked += 1;
            }
        }
    }
    assert!(checked > 0);
}

fn assert_evidence(snapshot: &Value) {
    assert_eq!(snapshot["status"], "needs_review", "{snapshot}");
    let result = &snapshot["result"];
    assert_eq!(result["citations"][0]["quote"], "School in 1799");
    assert_eq!(result["citations"][0]["source_id"], "s1");
    assert_eq!(result["citations"][0]["passage_id"], "p001");
    assert_eq!(result["citations"][0]["sha256"], "a".repeat(64));
    assert_eq!(result["citations"][0]["char_start"], 0);
    assert_eq!(result["citations"][0]["char_end"], 14);
    assert_eq!(result["assertions"][0]["value"], "1799");
    assert_eq!(result["assertion_validation"]["accepted"], 1);
    assert_eq!(result["evidence_only"], true);
    assert_eq!(result["human_review_required"], true);
    assert_eq!(result["proposals"], json!([]));
    let event = snapshot["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["kind"] == "tool.completed")
        .unwrap();
    let original = &event["data"]["result"];
    assert_eq!(original["sha256"], "a".repeat(64));
    assert_eq!(original["passages"][0]["char_start"], 0);
    assert_eq!(original["passages"][0]["char_end"], 14);
}

fn assert_compact_requests(requests: &[Value], budget: u32) {
    for request in requests {
        let prompt = instruction(request);
        assert!(!prompt.contains("below 300 words"));
        assert!(prompt.contains(&format!("configured {budget}-token limit")));
        assert!(prompt.contains("untrusted DATA"));
        assert_eq!(request["model"], "apertus-v1-5-8b");
        assert_eq!(request["max_tokens"], budget);
    }
    assert!(instruction(&requests[1]).contains("without any prefacing text"));
    assert_eq!(requests[1]["tool_choice"], "required");
    assert!(requests[2].get("tools").is_none());
    assert_compact_source_request(&requests[2]);
    assert!(!instruction(requests.last().unwrap()).contains("\"findings\""));
}

#[tokio::test]
async fn compact_workflow_retains_required_calls_exact_evidence_and_requested_assertions() {
    for budget in [128, 256] {
        let fixture = Fixture::new(budget, Candidate::Valid).await;
        let snapshot = fixture.run().await;
        assert_evidence(&snapshot);
        let requests = fixture.requests().await;
        assert_eq!(requests.len(), 4);
        assert_compact_requests(&requests, budget);
        let agents: Vec<_> = snapshot["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["kind"] == "model.completed")
            .map(|event| event["agent"].as_str().unwrap())
            .collect();
        assert_eq!(
            agents,
            ["planner", "researcher", "researcher", "verifier"]
        );
    }
}

#[tokio::test]
async fn compact_repairs_keep_small_contract_without_weakening_passage_validation() {
    for candidate in [Candidate::InvalidJson, Candidate::UnknownPassage] {
        let fixture = Fixture::new(256, candidate).await;
        let snapshot = fixture.run().await;
        assert_evidence(&snapshot);
        let requests = fixture.requests().await;
        assert_eq!(requests.len(), 5);
        assert_compact_requests(&requests, 256);
        assert_compact_source_request(&requests[3]);
        let repair = requests[3]["messages"].to_string();
        assert!(!repair.contains("Keep every required key"));
        assert!(repair.contains("Omit optional empty collections"));
        let event = match candidate {
            Candidate::InvalidJson => "output.rejected",
            Candidate::UnknownPassage => "validation.failed",
            Candidate::Valid => unreachable!(),
        };
        assert!(
            snapshot["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["kind"] == event)
        );
    }
}

#[tokio::test]
async fn compact_research_without_tools_keeps_projected_evidence() {
    let fixture = Fixture::new(256, Candidate::Valid)
        .await
        .without_tools()
        .await;
    let snapshot = fixture.run().await;
    assert_eq!(snapshot["status"], "needs_review");
    assert_eq!(
        snapshot["result"]["citations"][0]["quote"],
        "School in 1799"
    );
    let requests = fixture.requests().await;
    assert_eq!(requests.len(), 3);
    assert!(
        requests
            .iter()
            .all(|request| request.get("tools").is_none())
    );
    assert_compact_source_request(&requests[1]);
}

#[tokio::test]
async fn larger_output_budgets_preserve_existing_stage_instructions() {
    for budget in [257, 2048] {
        let fixture = Fixture::new(budget, Candidate::Valid).await;
        let snapshot = fixture.run().await;
        assert_evidence(&snapshot);
        let requests = fixture.requests().await;
        assert!(
            requests
                .iter()
                .all(|request| instruction(request).contains("below 300 words"))
        );
        assert!(instruction(&requests[0]).contains("Plan at most four"));
        assert!(instruction(&requests[2]).contains("\"entities\":[]"));
        assert!(instruction(&requests[3]).contains("\"findings\""));
        assert_eq!(requests.len(), 4);
    }
}
