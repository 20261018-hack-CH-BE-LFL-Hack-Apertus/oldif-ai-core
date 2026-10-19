"""Evaluate exact Bern source selection and abstention; never infer human gold approval."""
import argparse
import concurrent.futures
import datetime
import hashlib
import json
import time
import urllib.error
import urllib.request
from pathlib import Path


def score(result, source, expected_quote, negative):
    citations = result.get('citations') or []
    exact = all(c.get('source_id') == source['id'] and c.get('quote') and c['quote'] in source['text'] and c.get('sha256') == source['sha256'] and c.get('text_sha256') == source['text_sha256'] for c in citations)
    target = any(expected_quote in c.get('quote', '') for c in citations)
    abstained = result.get('abstained') is True
    answer = result.get('answer', '')
    remainder = answer.split('\n\n', 1)[1] if '\n\n' in answer else ''
    expected_remainder = '\n\n'.join('“' + c.get('quote', '') + '”' for c in citations)
    boundary = abstained or (result.get('evidence_only') is True and remainder == expected_remainder and not result.get('entities') and not result.get('timeline'))
    return {'exact_source_integrity': bool(exact), 'expected_passage_found': target, 'expected_abstention': negative, 'abstained': abstained, 'case_pass': (abstained if negative else target and not abstained) and exact and boundary, 'extractive_boundary': boundary, 'human_review_required': result.get('human_review_required') is True, 'human_historical_accuracy': 'NOT_SCORED_no_independent_human_QA_gold'}


def request_client(base_url, env_file):
    env = dict(line.split('=', 1) for line in Path(env_file).read_text().splitlines() if '=' in line and not line.lstrip().startswith('#'))
    token = env['INTERNAL_SERVICE_TOKEN'].strip().strip('\"').strip("'")
    def request(path, body=None):
        req = urllib.request.Request(base_url.rstrip('/') + path, data=None if body is None else json.dumps(body).encode(), headers={'Authorization':'Bearer ' + token,'Content-Type':'application/json'})
        with urllib.request.urlopen(req, timeout=190) as response:
            return json.load(response)
    return request


def run_case(request, fixture, model, locale, index, batch, output):
    start = time.monotonic()
    case = {'model':model,'locale':locale,'case':'missing_outcome' if index else 'intended_subjects','source_id':fixture['source']['id']}
    try:
        created = request('/v1/runs', {'question':fixture['questions'][locale][index],'locale':locale,'model':model,'sources':[fixture['source']],'session_id':batch,'user_id':'reference-evaluation','idempotency_key':f'{batch}:{model}:{locale}:{index}'})
        case['run_id'] = created['run_id']
        while time.monotonic()-start < 360:
            run = request('/v1/runs/' + created['run_id'])
            if run['status'] in ['needs_review','failed','cancelled']:
                break
            time.sleep(1)
        else:
            request('/v1/runs/'+created['run_id']+'/cancel', {})
            case.update(status='TIMEOUT',case_pass=False)
            return case
        (output/'runs'/f'{created["run_id"]}.json').write_text(json.dumps(run,ensure_ascii=False,indent=2)+'\n')
        case['status']=run['status']
        calls=[e['data'] for e in run.get('events',[]) if e['kind']=='model.completed']
        case.update(provider_calls=len(calls),reported_tokens=sum((c.get('usage') or {}).get('total_tokens',0) for c in calls),provider_ms=sum(c.get('duration_ms',0) for c in calls),served_models=sorted({c.get('served_model','unknown') for c in calls}))
        if run['status']=='needs_review':
            case.update(score(run.get('result') or {},fixture['source'],fixture['expected_quote'],bool(index)))
        else:
            case.update(case_pass=False,error=(run.get('result') or {}).get('error','Run did not complete'))
    except (OSError,ValueError,KeyError) as error:
        case.update(status='FAILED',case_pass=False,error=type(error).__name__)
    finally:
        case['duration_seconds']=round(time.monotonic()-start,3)
    return case


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--base-url',required=True)
    parser.add_argument('--env-file',required=True)
    parser.add_argument('--fixture',required=True)
    parser.add_argument('--output',required=True)
    parser.add_argument('--allow-astra',action='store_true')
    parser.add_argument('--models',default='apertus-v1-5-70b')
    parser.add_argument('--locales',default='en,es,de,de-CH,fr,it,rm')
    args=parser.parse_args()
    fixture= json.loads(Path(args.fixture).read_text())
    assert hashlib.sha256(fixture['source']['text'].encode()).hexdigest()==fixture['source']['text_sha256']
    output=Path(args.output);(output/'runs').mkdir(parents=True,exist_ok=True)
    request=request_client(args.base_url,args.env_file)
    models=args.models.split(',')
    allowed=['apertus-v1-5-8b','apertus-v1-5-8b-thinking','apertus-v1-5-70b','apertus-v1-5-70b-thinking']
    if any(model not in allowed for model in models):
        parser.error('Select only known Apertus variants with --models; Astra requires --allow-astra')
    if args.allow_astra:
        models.append('gpt-6-astra')
    batch='bern-reference-'+datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S')
    receipt={'batch_id':batch,'fixture_sha256':hashlib.sha256(Path(args.fixture).read_bytes()).hexdigest(),'reference_status':fixture['reference_status'],'independent_human_gold':False,'distinct_source_documents':1,'confidence_interval':None,'confidence_interval_reason':'Repeated questions and translations of one source are not independent document samples.','cases':[]}
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        futures=[pool.submit(run_case,request,fixture,model,locale,index,batch,output) for model in models for locale in args.locales.split(',') for index in [0,1]]
        for future in concurrent.futures.as_completed(futures):
            case=future.result();receipt['cases'].append(case)
            (output/'summary.json').write_text(json.dumps(receipt,ensure_ascii=False,indent=2)+'\n')
            print(json.dumps({key:case.get(key) for key in ['model','locale','case','status','case_pass','duration_seconds']}),flush=True)
    receipt['aggregates']=[{'model':model,'cases':len(selected),'completed':sum(c['status']=='needs_review' for c in selected),'reference_case_passes':sum(c.get('case_pass',False) for c in selected),'abstentions':sum(c.get('abstained',False) for c in selected),'tokens':sum(c.get('reported_tokens',0) for c in selected)} for model in models if (selected:=[c for c in receipt['cases'] if c['model']==model])]
    (output/'summary.json').write_text(json.dumps(receipt,ensure_ascii=False,indent=2)+'\n')

if __name__=='__main__':
    main()
