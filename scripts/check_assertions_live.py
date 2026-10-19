"""Real-provider synthetic conflict acceptance; never a historical-accuracy score."""
import argparse
import datetime
import hashlib
import json
import time
import urllib.error
from pathlib import Path
from reference_evaluate import request_client


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--base-url', required=True)
    parser.add_argument('--env-file', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    request = request_client(args.base_url, args.env_file)
    sources = []
    for identifier, year in [('synthetic-a', '1799'), ('synthetic-b', '1800')]:
        text = f'École opened in {year}.'
        digest = hashlib.sha256(text.encode()).hexdigest()
        sources.append({'id': identifier, 'title': 'Synthetic contradictory fixture', 'text': text,
                        'url': 'https://example.org/' + identifier, 'page': None,
                        'sha256': digest, 'text_sha256': digest, 'licence': 'synthetic-test-only'})
    key = 'assertions-' + str(time.time_ns())
    body = {'question': 'These two explicitly synthetic sources disagree. Return two separate date assertions for subject École with predicate event_date, copying each exact year and its own source_id and passage_id. Cite both sources. Do not select a consensus date.',
            'locale': 'en', 'model': 'apertus-v1-5-70b', 'sources': sources,
            'session_id': key, 'user_id': 'synthetic-acceptance', 'idempotency_key': key}
    checks = []
    try:
        request('/v1/runs', dict(body, unauthorized_override=True))
        checks.append({'name': 'unknown request key rejected', 'pass': False})
    except urllib.error.HTTPError as error:
        checks.append({'name': 'unknown request key rejected', 'pass': error.code == 422, 'http_status': error.code})
    started = time.monotonic()
    created = request('/v1/runs', body)
    repeated = request('/v1/runs', body)
    checks.append({'name': 'identical request reuses run', 'pass': repeated['run_id'] == created['run_id']})
    run = None
    while time.monotonic() - started < 240:
        run = request('/v1/runs/' + created['run_id'])
        if run['status'] in ['needs_review', 'failed', 'cancelled']:
            break
        time.sleep(1)
    else:
        request('/v1/runs/' + created['run_id'] + '/cancel', {})
    output = Path(args.output)
    output.mkdir(parents=True, exist_ok=True)
    raw = json.dumps(run, ensure_ascii=False, indent=2) + '\n'
    (output / 'run.json').write_text(raw)
    result = run.get('result') or {}
    assertions = result.get('assertions') or []
    events = run.get('events') or []
    checks.extend([
        {'name': 'run completed', 'pass': run['status'] == 'needs_review'},
        {'name': 'both attributed dates retained', 'pass': {(a['source_id'], a['value']) for a in assertions} == {('synthetic-a', '1799'), ('synthetic-b', '1800')}},
        {'name': 'potential conflict retains shared group', 'pass': len(assertions) == 2 and all(a['potential_conflict'] and a['conflict_group'] for a in assertions) and assertions[0]['conflict_group'] == assertions[1]['conflict_group']},
        {'name': 'no automatic historical approval', 'pass': len(assertions) == 2 and all(a['state'] == 'proposed' and a['human_reviewed'] is False for a in assertions) and result.get('evidence_only') is True},
        {'name': 'stable event identities', 'pass': bool(events) and all(e['event_id'] == f'{e["run_id"]}:{e["sequence"]}' for e in events) and len({e['event_id'] for e in events}) == len(events)},
    ])
    for assertion in assertions:
        source = next(s for s in sources if s['id'] == assertion['source_id'])
        evidence = assertion['evidence']
        checks.append({'name': 'exact span ' + source['id'], 'pass': source['text'][evidence['char_start']:evidence['char_end']] == evidence['quote'] == assertion['value'] and evidence['text_sha256'] == source['text_sha256'] and evidence['sha256'] == source['sha256']})
    receipt = {'captured_at': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'model': body['model'],
               'provider_calls': sum(e['kind'] == 'model.completed' for e in events), 'run_id': created['run_id'],
               'elapsed_seconds': round(time.monotonic() - started, 3), 'checks': checks,
               'passed': sum(c['pass'] for c in checks), 'failed': sum(not c['pass'] for c in checks),
               'run_sha256': hashlib.sha256(raw.encode()).hexdigest(),
               'scope': 'Real Apertus provider, synthetic contradictory input, local native core. Not human historical gold or production load.'}
    (output / 'summary.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps(receipt, indent=2))
    raise SystemExit(bool(receipt['failed']))


if __name__ == '__main__':
    main()
