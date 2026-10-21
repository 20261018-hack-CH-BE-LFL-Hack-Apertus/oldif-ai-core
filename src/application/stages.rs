use super::{context, interpretation, passages, prompts, tools, validation, workflow::Engine};
use crate::{api::dto::RunRequest, domain, error::CoreError, infrastructure::store::Run};
use serde_json::{Value, json};
use std::collections::HashSet;
fn content(message: &Value) -> Result<&str, CoreError> {
    message["content"]
        .as_str()
        .ok_or_else(|| CoreError::Invalid("Model did not return textual JSON".into()))
}
pub async fn plan(e: &Engine, run: &Run, r: &RunRequest) -> Result<Value, CoreError> {
    let input = e.context(run, r).await?;
    let input = if domain::supports_tools(&r.model) && e.config.max_tool_calls > 0 {
        context::catalog(input, &r.sources)
    } else {
        input
    };
    let msg = e
        .call(
            run,
            "planner",
            &r.model,
            prompts::messages(&e.config, &prompts::plan(&e.config), input),
            None,
        )
        .await?;
    let plan = validation::parse_json(content(&msg)?)?;
    e.store
        .emit(run, "plan.created", "planner", plan.clone())
        .await?;
    Ok(plan)
}
pub async fn research(
    e: &Engine,
    run: &Run,
    r: &RunRequest,
    plan: &Value,
) -> Result<Value, CoreError> {
    let mut context = e.context(run, r).await?;
    context["plan"] = plan.clone();
    let events = run.snapshot.lock().await.events.clone();
    // A steering revision shares the same budget and already-read evidence.
    let mut calls = events
        .iter()
        .filter(|event| event.kind == "tool.started")
        .count();
    let mut read_ids: HashSet<String> = events
        .iter()
        .filter(|event| event.kind == "tool.completed" && event.data["tool"] == "read_source")
        .filter_map(|event| event.data["result"]["id"].as_str().map(str::to_owned))
        .collect();
    if domain::supports_tools(&r.model) && e.config.max_tool_calls > 0 {
        context = context::catalog(context, &r.sources);
        context["previously_read_sources"] = json!(
            r.sources
                .iter()
                .filter(|source| read_ids.contains(&source.id))
                .map(passages::source_view)
                .collect::<Vec<_>>()
        );
    } else {
        context["sources"] = json!(
            r.sources
                .iter()
                .map(passages::source_view)
                .collect::<Vec<_>>()
        );
    }
    let instruction = prompts::research(&e.config);
    let mut conversation = prompts::messages(&e.config, &instruction, context);
    loop {
        let allowed = domain::supports_tools(&r.model)
            && calls < e.config.max_tool_calls
            && read_ids.len() < r.sources.len();
        let msg = e
            .call(
                run,
                "researcher",
                &r.model,
                conversation.clone(),
                allowed.then(|| tools::for_sources(&r.sources, calls == 0)),
            )
            .await?;
        let requested = msg["tool_calls"].as_array().cloned().unwrap_or_default();
        if requested.is_empty() {
            let text = content(&msg)?;
            match validation::parse_json(text) {
                Ok(value) => return Ok(value),
                Err(error) => {
                    e.store
                        .emit(
                            run,
                            "output.rejected",
                            "researcher",
                            json!({"error":error.to_string(),"repair_budget":1}),
                        )
                        .await?;
                    conversation.push(msg);
                    conversation
                        .push(json!({"role":"user","content":prompts::format_repair(&e.config)}));
                    let repaired = e
                        .call(
                            run,
                            "researcher-format-repair",
                            &r.model,
                            conversation,
                            None,
                        )
                        .await?;
                    return validation::parse_json(content(&repaired)?);
                }
            }
        }
        if !allowed {
            return Err(CoreError::Invalid(
                "Provider requested tools after bounded finalization disabled them".into(),
            ));
        }
        conversation.push(msg);
        for call in requested {
            if calls >= e.config.max_tool_calls {
                let denied = json!({"error":"tool_budget_exhausted","executed":false});
                e.store.emit(run,"tool.denied","researcher",json!({"call_id":call["id"],"tool":call["function"]["name"],"reason":"tool_budget_exhausted"})).await?;
                conversation.push(
                    json!({"role":"tool","tool_call_id":call["id"],"content":denied.to_string()}),
                );
                continue;
            }
            calls += 1;
            let name = call["function"]["name"].as_str().unwrap_or_default();
            if calls == 1 && name != "read_source" {
                return Err(CoreError::Invalid(
                    "The first evidence tool must read an approved source".into(),
                ));
            }
            let args: Value =
                serde_json::from_str(call["function"]["arguments"].as_str().unwrap_or("{}"))
                    .map_err(|_| CoreError::Invalid("Invalid tool arguments".into()))?;
            e.store
                .emit(
                    run,
                    "tool.started",
                    "researcher",
                    json!({"tool":name,"arguments":args,"call_id":call["id"]}),
                )
                .await?;
            let result = tools::execute(name, &args, &r.sources)?;
            if name == "read_source" {
                if let Some(id) = result["id"].as_str() {
                    read_ids.insert(id.to_owned());
                }
            }
            e.store
                .emit(
                    run,
                    "tool.completed",
                    "researcher",
                    json!({"tool":name,"call_id":call["id"],"result":result}),
                )
                .await?;
            conversation.push(
                json!({"role":"tool","tool_call_id":call["id"],"content":result.to_string()}),
            );
        }
        if read_ids.len() == r.sources.len() {
            e.store
                .emit(
                    run,
                    "tools.evidence_complete",
                    "manager",
                    json!({"sources_read":read_ids.len(),"finalization":"json_without_tools"}),
                )
                .await?;
        }
        if calls >= e.config.max_tool_calls {
            e.store
                .emit(
                    run,
                    "tools.budget_reached",
                    "manager",
                    json!({"executed":calls,"finalization":"tools_disabled"}),
                )
                .await?;
            conversation.push(json!({"role":"user","content":"The tool budget is exhausted. Do not request more tools. Use the available source texts and completed tool outputs to return the required JSON now. Explicitly abstain if the evidence is insufficient."}));
        }
    }
}
pub async fn verify(
    e: &Engine,
    run: &Run,
    r: &RunRequest,
    result: &Value,
) -> Result<Value, CoreError> {
    let mut ctx = context::reviewed(e.context(run, r).await?, &r.sources, result);
    ctx["candidate"] = result.clone();
    let msg = e
        .call(
            run,
            "verifier",
            &r.model,
            prompts::messages(&e.config, &prompts::verify(&e.config), ctx),
            None,
        )
        .await?;
    let mut review = if result["abstained"].as_bool() == Some(true) {
        e.store
            .emit(
                run,
                "review.output_bounded",
                "outcome-owner",
                json!({"reason":"candidate_abstained","free_form_output_published":false}),
            )
            .await?;
        json!({"verdict":"abstain","findings":[result["answer"]],"contradictions":[],"historical_bias":[],"output_policy":"deterministic_abstention"})
    } else {
        let raw = content(&msg)
            .and_then(validation::parse_json)
            .unwrap_or(json!({"verdict":null}));
        interpretation::review(raw)
    };
    review["independent_pass"] = json!(false);
    review["separate_pass"] = json!(true);
    review["same_model"] = json!(true);
    review["human_review_required"] = json!(true);
    e.store
        .emit(run, "verification.completed", "verifier", review.clone())
        .await?;
    Ok(review)
}
pub async fn reflect(
    e: &Engine,
    run: &Run,
    r: &RunRequest,
    result: &Value,
) -> Result<Value, CoreError> {
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
    let v = match content(&msg).and_then(validation::parse_json) {
        Ok(value) => value,
        Err(CoreError::Invalid(_)) => {
            e.store.emit(run,"proposals.unavailable","dreamer",json!({"reason":"invalid_or_truncated_json","proposals_published":0,"retry":false})).await?;
            return Ok(json!([]));
        }
        Err(error) => return Err(error),
    };
    if result["abstained"] == true || result["evidence_only"] == true {
        let reason = if result["abstained"] == true {
            "candidate_abstained"
        } else {
            "interpretation_unverified"
        };
        e.store.emit(run,"proposals.withheld","outcome-owner",json!({"reason":reason,"free_form_output_published":false,"model_pass_executed":true})).await?;
        return Ok(json!([]));
    }
    let proposals:Vec<Value>=v["proposals"].as_array().into_iter().flatten().take(2).map(|p|json!({"id":uuid::Uuid::new_v4().to_string(),"kind":p["kind"],"description":p["description"],"rationale":p["rationale"],"status":"pending_human_review","applied":false})).collect();
    e.store
        .emit(run, "proposals.created", "dreamer", json!(proposals))
        .await?;
    Ok(json!(proposals))
}

pub async fn repair(
    e: &Engine,
    run: &Run,
    r: &RunRequest,
    candidate: Value,
    error: &str,
) -> Result<Value, CoreError> {
    let context = context::for_repair(
        e.context(run, r).await?,
        &r.sources,
        &run.snapshot.lock().await.events.clone(),
        candidate,
        error,
    );
    let instruction = prompts::repair(&e.config);
    let message = e
        .call(
            run,
            "researcher-repair",
            &r.model,
            prompts::messages(&e.config, &instruction, context),
            None,
        )
        .await?;
    validation::parse_json(content(&message)?)
}
