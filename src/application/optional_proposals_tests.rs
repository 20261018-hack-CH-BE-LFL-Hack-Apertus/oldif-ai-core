use super::optional_proposals_test_support::{Fixture, OptionalResponse, snapshot};
use crate::error::CoreError;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;

fn events(snapshot: &Value, kind: &str) -> Vec<Value> {
    snapshot["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["kind"] == kind)
        .cloned()
        .collect()
}

fn assert_withheld(snapshot: &Value, reason: &str) {
    let withheld = events(snapshot, "proposals.withheld");
    assert_eq!(withheld.len(), 1);
    assert_eq!(
        withheld[0]["data"],
        json!({"reason":reason,"free_form_output_published":false,"model_pass_executed":false})
    );
    assert!(events(snapshot, "proposals.created").is_empty());
    assert!(
        snapshot["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["agent"] != "dreamer")
    );
}

fn assert_evidence(result: &Value) {
    assert!(
        result["answer"]
            .as_str()
            .unwrap()
            .contains("School in 1799")
    );
    assert_eq!(result["evidence_only"], true);
    assert_eq!(result["human_review_required"], true);
    assert_eq!(result["interpretation_draft"]["human_reviewed"], false);
    assert_eq!(result["verification"]["verdict"], "supported");
    assert_eq!(result["citations"][0]["quote"], "School in 1799");
    assert_eq!(result["citations"][0]["source_id"], "s1");
    assert_eq!(result["citations"][0]["passage_id"], "p001");
    assert_eq!(result["citations"][0]["sha256"], "a".repeat(64));
    assert_eq!(result["citations"][0]["char_start"], 0);
    assert_eq!(result["citations"][0]["char_end"], 14);
    assert_eq!(result["proposals"], json!([]));
}

#[tokio::test]
async fn evidence_only_workflow_preserves_answer_without_requesting_proposals() {
    let fixture = Fixture::new(OptionalResponse::ProviderFailure).await;
    let value = fixture.workflow().await;
    assert_eq!(value["status"], "needs_review");
    assert_evidence(&value["result"]);
    assert_withheld(&value, "interpretation_unverified");
    assert_eq!(fixture.request_count().await, 4);
    assert_eq!(events(&value, "model.completed").len(), 4);
    assert_eq!(events(&value, "verification.completed").len(), 1);
}

#[tokio::test]
async fn ineligible_results_skip_inference_and_preserve_the_input() {
    for (result, reason) in [
        (json!({"abstained":true}), "candidate_abstained"),
        (json!({"evidence_only":true}), "interpretation_unverified"),
    ] {
        let fixture = Fixture::new(OptionalResponse::ProviderFailure).await;
        let run = fixture.run().await;
        let original = result.clone();
        assert_eq!(fixture.reflect(&run, &result).await.unwrap(), json!([]));
        assert_eq!(result, original);
        assert_eq!(fixture.request_count().await, 0);
        assert_withheld(&snapshot(&run).await, reason);
    }
}

#[tokio::test]
async fn cancelled_ineligible_stage_emits_nothing_and_preserves_cancellation() {
    let fixture = Fixture::new(OptionalResponse::ProviderFailure).await;
    let run = fixture.run().await;
    run.cancelled.store(true, Ordering::SeqCst);
    let result = fixture.reflect(&run, &json!({"evidence_only":true})).await;
    assert!(matches!(result, Err(CoreError::Cancelled)));
    assert_eq!(fixture.request_count().await, 0);
    assert!(
        snapshot(&run).await["events"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn withholding_storage_failure_is_propagated_without_inference() {
    let fixture = Fixture::new(OptionalResponse::ProviderFailure).await;
    let run = fixture.run().await;
    let run_id = snapshot(&run).await["run_id"].as_str().unwrap().to_owned();
    std::fs::create_dir(fixture.directory.join(format!("{run_id}.tmp"))).unwrap();
    let result = fixture.reflect(&run, &json!({"evidence_only":true})).await;
    assert!(matches!(result, Err(CoreError::Storage)));
    assert_eq!(fixture.request_count().await, 0);
    assert!(events(&snapshot(&run).await, "model.started").is_empty());
}

#[tokio::test]
async fn eligible_invalid_optional_output_remains_nonfatal_and_explicit() {
    for response in [
        OptionalResponse::Truncated,
        OptionalResponse::MissingContent,
    ] {
        let fixture = Fixture::new(response).await;
        let run = fixture.run().await;
        assert_eq!(fixture.reflect(&run, &json!({})).await.unwrap(), json!([]));
        assert_eq!(fixture.request_count().await, 1);
        let value = snapshot(&run).await;
        assert_eq!(
            events(&value, "proposals.unavailable")[0]["data"],
            json!({"reason":"invalid_or_truncated_json","proposals_published":0,"retry":false})
        );
        assert!(events(&value, "proposals.created").is_empty());
        assert!(events(&value, "proposals.withheld").is_empty());
    }
}

#[tokio::test]
async fn eligible_provider_failure_is_propagated_with_failure_evidence() {
    let fixture = Fixture::new(OptionalResponse::ProviderFailure).await;
    let run = fixture.run().await;
    assert!(matches!(
        fixture.reflect(&run, &json!({})).await,
        Err(CoreError::Provider(503))
    ));
    assert_eq!(fixture.request_count().await, 1);
    let value = snapshot(&run).await;
    assert_eq!(events(&value, "model.failed")[0]["agent"], "dreamer");
    assert!(events(&value, "proposals.unavailable").is_empty());
    assert!(events(&value, "proposals.withheld").is_empty());
}

#[tokio::test]
async fn eligible_proposals_remain_pending_review_and_unapplied() {
    let fixture = Fixture::new(OptionalResponse::Valid).await;
    let run = fixture.run().await;
    let result = fixture.reflect(&run, &json!({})).await.unwrap();
    assert_eq!(fixture.request_count().await, 1);
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result[0]["kind"], "source_review");
    assert_eq!(result[0]["status"], "pending_human_review");
    assert_eq!(result[0]["applied"], false);
    let value = snapshot(&run).await;
    assert_eq!(events(&value, "proposals.created").len(), 1);
    assert!(events(&value, "proposals.withheld").is_empty());
}
