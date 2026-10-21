use super::{context, prompts, stages::content, validation, workflow::Engine};
use crate::{api::dto::RunRequest, error::CoreError, infrastructure::store::Run};
use serde_json::{Value, json};

pub async fn reflect(
    e: &Engine,
    run: &Run,
    r: &RunRequest,
    result: &Value,
) -> Result<Value, CoreError> {
    Engine::check_cancel(run)?;
    if result["abstained"] == true || result["evidence_only"] == true {
        return withhold(e, run, result).await;
    }
    let msg = e
        .call(
            run,
            "dreamer",
            &r.model,
            prompts::messages(&e.config, &prompts::reflect(&e.config), {
                let mut context = context::reviewed(e.context(run, r).await?, &r.sources, result);
                context["result"] = result.clone();
                context
            }),
            None,
        )
        .await?;
    publish(e, run, content(&msg).and_then(validation::parse_json)).await
}

async fn withhold(e: &Engine, run: &Run, result: &Value) -> Result<Value, CoreError> {
    let reason = if result["abstained"] == true {
        "candidate_abstained"
    } else {
        "interpretation_unverified"
    };
    e.store
        .emit(
            run,
            "proposals.withheld",
            "outcome-owner",
            json!({"reason":reason,"free_form_output_published":false,"model_pass_executed":false}),
        )
        .await?;
    Ok(json!([]))
}

async fn publish(
    e: &Engine,
    run: &Run,
    parsed: Result<Value, CoreError>,
) -> Result<Value, CoreError> {
    let v = match parsed {
        Ok(value) => value,
        Err(CoreError::Invalid(_)) => {
            e.store.emit(run,"proposals.unavailable","dreamer",json!({"reason":"invalid_or_truncated_json","proposals_published":0,"retry":false})).await?;
            return Ok(json!([]));
        }
        Err(error) => return Err(error),
    };
    let proposals:Vec<Value>=v["proposals"].as_array().into_iter().flatten().take(2).map(|p|json!({"id":uuid::Uuid::new_v4().to_string(),"kind":p["kind"],"description":p["description"],"rationale":p["rationale"],"status":"pending_human_review","applied":false})).collect();
    e.store
        .emit(run, "proposals.created", "dreamer", json!(proposals))
        .await?;
    Ok(json!(proposals))
}
