"""Run bounded, live workflow checks; unavailable providers never become benchmark scores."""
import argparse
import concurrent.futures
import datetime
import hashlib
import json
import time
import urllib.error
import urllib.request
from pathlib import Path

LOCALES = ["en", "es", "de", "de-CH", "fr", "it", "rm"]
MODELS = ["apertus-v1-5-8b", "apertus-v1-5-8b-thinking", "apertus-v1-5-70b", "apertus-v1-5-70b-thinking", "gpt-6-astra"]
QUESTION = "Summarize what the source says about schooling. Use one short exact quotation from source.text and note source bias. Read the supplied source once; do not search other records."


def client(base_url, token):
    def request(path, body=None):
        req = urllib.request.Request(base_url.rstrip("/") + path, data=None if body is None else json.dumps(body).encode(), headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=180) as response:
            return json.load(response)
    return request


def run_case(request, source, model, locale, batch_id, deadline_seconds):
    started = time.monotonic()
    case = {"model": model, "locale": locale, "source_ids": [source["id"]], "status": "NOT_RUN"}
    try:
        run = request("/v1/runs", {"question": QUESTION, "locale": locale, "model": model, "sources": [source], "session_id": batch_id, "user_id": "evaluation-client", "idempotency_key": f"{batch_id}:{model}:{locale}"})
        case["run_id"] = run["run_id"]
        while time.monotonic() - started < deadline_seconds:
            run = request("/v1/runs/" + run["run_id"])
            if run["status"] in ["needs_review", "failed", "cancelled"]:
                return summarize(case, run, source, started)
            time.sleep(1)
        request("/v1/runs/" + run["run_id"] + "/cancel", {})
        case.update(status="TIMEOUT", reason="Evaluation deadline reached; cancellation requested")
    except urllib.error.HTTPError as error:
        case.update(status="FAILED", http_status=error.code, reason="Service rejected evaluation request")
    except (OSError, ValueError, KeyError) as error:
        case.update(status="FAILED", reason=type(error).__name__)
    case["duration_seconds"] = round(time.monotonic() - started, 3)
    return case


def summarize(case, run, source, started):
    result = run.get("result") or {}
    events = run.get("events", [])
    calls = [event["data"] for event in events if event["kind"] == "model.completed"]
    citations = result.get("citations", [])
    exact = sum(c.get("source_id") == source["id"] and bool(c.get("quote")) and c["quote"] in source["text"] for c in citations)
    return {**case, "status": run["status"], "duration_seconds": round(time.monotonic() - started, 3), "answer": result.get("answer"), "error": result.get("error"), "abstained": result.get("abstained"), "citations_checked": len(citations), "exact_quotes": exact, "quote_integrity": exact / len(citations) if citations else None, "events": len(events), "tool_calls": sum(e["kind"] == "tool.completed" for e in events), "served_models": sorted(set(c.get("served_model", "unknown") for c in calls)), "provider_calls": len(calls), "failed_provider_calls": sum(e["kind"] == "model.failed" for e in events), "provider_duration_ms": sum(c.get("duration_ms", 0) for c in calls), "usage": [c.get("usage") for c in calls], "verification": result.get("verification"), "human_language_review": "NOT_RUN", "human_historical_accuracy": "NOT_RUN"}


def analytics(cases, models):
    aggregates = []
    for model in models:
        selected = [c for c in cases if c["model"] == model]
        usage = [u for c in selected for u in c.get("usage", []) if isinstance(u, dict)]
        aggregates.append({"model": model, "role": "default_and_primary" if model == "apertus-v1-5-8b" else "apertus_variant" if model.startswith("apertus-") else "explicit_backup_comparison", "cases": len(selected), "completed": sum(c["status"] == "needs_review" for c in selected), "answered": sum(c["status"] == "needs_review" and c.get("abstained") is False for c in selected), "abstained": sum(c["status"] == "needs_review" and c.get("abstained") is True for c in selected), "failed": sum(c["status"] in ["FAILED", "failed", "TIMEOUT", "cancelled"] for c in selected), "not_run": sum(c["status"] == "NOT_RUN" for c in selected), "provider_calls": sum(c.get("provider_calls", 0) for c in selected), "tool_calls": sum(c.get("tool_calls", 0) for c in selected), "failed_provider_calls": sum(c.get("failed_provider_calls", 0) for c in selected), "provider_duration_ms": sum(c.get("provider_duration_ms", 0) for c in selected), "total_reported_tokens": sum(u.get("total_tokens", 0) for u in usage), "token_total_excludes_unreported_failed_calls": True, "billing_cost_verified": False, "sum_case_seconds_including_queue": round(sum(c.get("duration_seconds", 0) for c in selected), 3), "quality": "NOT_RUN_human_gold_unavailable"})
    return aggregates


def evaluate(base_url, env_file, source_file, output, models=MODELS, locales=LOCALES, workers=2, deadline_seconds=480):
    env = dict(line.split("=", 1) for line in Path(env_file).read_text().splitlines() if "=" in line and not line.startswith("#"))
    request = client(base_url, env["INTERNAL_SERVICE_TOKEN"])
    source = json.loads(Path(source_file).read_text())
    catalog = {m["id"]: m for m in request("/v1/models")["data"]}
    batch_id = "eval-" + datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S")
    cases = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
        futures = []
        for model in models:
            for locale in locales:
                info = catalog.get(model, {})
                if not info.get("available", False):
                    cases.append({"model": model, "locale": locale, "status": "NOT_RUN", "reason": info.get("unavailable_reason", "Model unavailable"), "access_observation": info.get("observation")})
                else:
                    futures.append(pool.submit(run_case, request, source, model, locale, batch_id, deadline_seconds))
        for future in concurrent.futures.as_completed(futures):
            cases.append(future.result())
            Path(output).parent.mkdir(parents=True, exist_ok=True)
            Path(output).with_suffix(".progress.json").write_text(json.dumps({"batch_id":batch_id,"status":"RUNNING","cases":cases},ensure_ascii=False,indent=2)+"\n")
    report = {"batch_id": batch_id, "source_sha256": source["sha256"], "source_hash_scope": "declared_input_provenance_hash", "source_text_sha256": hashlib.sha256(source["text"].encode()).hexdigest(), "source_artifact_sha256": source.get("source_sha256"), "source_record_locator": source.get("record_locator"), "fixture_sha256": hashlib.sha256(Path(source_file).read_bytes()).hexdigest(), "question": QUESTION, "test_class": "live_workflow_smoke_not_held_out_quality_benchmark", "expected_cases": len(models) * len(locales), "completed_cases": sum(c["status"] == "needs_review" for c in cases), "not_run_cases": sum(c["status"] == "NOT_RUN" for c in cases), "failed_cases": sum(c["status"] in ["FAILED", "failed", "TIMEOUT", "cancelled"] for c in cases), "human_gold_documents": 0, "challenge_1000_page_throughput": "NOT_RUN", "challenge_80_percent_usability": "NOT_RUN", "challenge_under_5_percent_structural_errors": "NOT_RUN", "reason": "One public transcribed source record; no frozen human gold, manuscript page ground truth, or declared standard-machine 1000-page corpus", "cases": sorted(cases, key=lambda c: (c["model"], c["locale"]))}
    report["analytics"] = analytics(cases, models)
    Path(output).parent.mkdir(parents=True, exist_ok=True)
    Path(output).write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--env-file", required=True)
    parser.add_argument("--source-file", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--workers", type=int, choices=[1, 2], default=2)
    args = parser.parse_args()
    report = evaluate(**vars(args))
    print(json.dumps({key: report[key] for key in ["batch_id", "expected_cases", "completed_cases", "not_run_cases", "failed_cases"]}))
