"""Verify persisted real-run receipts and fail-closed submission policy without new inference."""
import argparse
import datetime
import hashlib
import json
import urllib.error
from pathlib import Path
from reference_evaluate import request_client


def main():
    p = argparse.ArgumentParser()
    for key in ['base-url', 'env-file', 'run', 'receipt-directory', 'output']:
        p.add_argument('--' + key, required=True)
    a = p.parse_args()
    call = request_client(a.base_url, a.env_file)
    run = json.loads(Path(a.run).read_text())
    checks = []
    def check(name, passed):
        checks.append({'name':name, 'pass':bool(passed)})
    models = call('/v1/models')['data']
    check('submission model catalog disables all proprietary routes', all(not m['available'] for m in models if not m['challenge_model']))
    for m in [m['id'] for m in models if not m['challenge_model']]:
        try:
            call('/v1/chat/completions', {'model':m, 'messages':[{'role':'user','content':'Forbidden submission-profile probe'}]})
            check('policy rejects ' + m, False)
        except urllib.error.HTTPError as error:
            body = json.loads(error.read())
            check('policy rejects ' + m, error.code == 403 and body.get('code') == 'POLICY_RESTRICTED')
    completed = [e for e in run['events'] if e['kind'] == 'model.completed']
    check('real workflow has provider completions', len(completed) > 0)
    for event in completed:
        record = event['data']['provider_receipt']
        path = Path(a.receipt_directory) / (record['receipt_id'] + '.json')
        persisted = json.loads(path.read_text())
        check('persisted receipt matches event ' + str(event['sequence']), record == persisted)
        check('submission Apertus outcome/configuration ' + str(event['sequence']),
              record['execution_profile'] == 'submission' and record['requested_model'] == 'apertus-v1-5-70b'
              and record['http_status'] == 200 and record['raw_response_bytes'] > 0
              and len(record['raw_response_sha256']) == 64 and len(record['request_sha256']) == 64
              and record['configuration']['model'] == 'apertus-v1-5-70b'
              and record['credentials_retained'] is False and record['raw_content_retained'] is False)
    result = {'captured_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'run_id':run['run_id'], 'run_sha256':hashlib.sha256(Path(a.run).read_bytes()).hexdigest(),
              'additional_inference_calls':0, 'observed_provider_completions':len(completed),
              'checks':checks, 'passed':sum(c['pass'] for c in checks), 'failed':sum(not c['pass'] for c in checks),
              'limits':['No provider weight revision is invented when the gateway does not expose it.',
                        'Raw response hashes establish byte identity, not historical truth.',
                        'Realtime SDP/session hashes do not represent received speech or listening verification.']}
    Path(a.output).write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result,indent=2))
    raise SystemExit(bool(result['failed']))


if __name__ == '__main__':
    main()
