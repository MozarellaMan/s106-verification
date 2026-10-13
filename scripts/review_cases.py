"""Case/review data layer shared by renderer, comparator and registry CLI.

The existing renderer only exported decisions and the comparator assumed Southwark
locators. Extend those callers here: immutable agent reference + append-only human
records + a derived view. Imports check binding/conflicts, not reviewer identity or
legal accuracy. Synthetic controls in tests/python/test_review_cases.py must pass.
"""
import copy
import hashlib
import json
import re
from datetime import date, datetime, timezone
from pathlib import Path

FORMAT = 's106-human-review/1'
DECISIONS = {'confirmed', 'corrected', 'rejected', 'needs clarification', 'added'}
CLAIM_KEYS = {'interpretation', 'expected_fields', 'evidence'}


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read_json(path):
    return json.loads(Path(path).read_text())


def write_json(path, value, exclusive=False):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(value, indent=2, ensure_ascii=False, allow_nan=False) + '\n'
    with path.open('x' if exclusive else 'w') as file:
        file.write(text)


def load_case(case):
    case = Path(case)
    reference = read_json(case / 'reference.json')
    provenance = read_json(case / 'provenance.json')
    if digest(case / 'reference.json') != (case / 'reference-freeze.sha256').read_text().split()[0]:
        raise ValueError('reference freeze mismatch; create a new reference version explicitly')
    if reference['source_sha256'] != provenance['sha256']:
        raise ValueError('reference/source hash mismatch')
    ids = [c['id'] for c in reference['claims']]
    if len(ids) != len(set(ids)):
        raise ValueError('duplicate reference claim IDs')
    if reference.get('status') in {'human_review_complete', 'partially_human_reviewed'} or any(c.get('human_review_status') is not None for c in reference['claims']):
        raise ValueError('agent reference contains human statuses; use separate review records')
    return reference, provenance


def validate_fields(fields, page_count=None):
    if not isinstance(fields, dict):
        raise ValueError('expected_fields must be an object')
    for key, value in fields.items():
        if not isinstance(key, str) or not re.fullmatch(r'(housing\.[a-z_]+(?:\.[a-z_]+){0,2}|tenure\.[^.]+\.(units|habitable_rooms|evidence\.(page|snippet))|contribution\.[^.]+\.(amount\.(structure|value_gbp|index_linked|words|evidence\.(page|snippet))|conditionality\.(kind|condition|evidence\.(page|snippet))|payment\.(trigger_event|instalments|trigger_phrase\.(page|snippet)|covenant\.(text|evidence\.(page|snippet)))))', key):
            raise ValueError(f'unsupported comparison field: {key}')
        if value is None:
            continue
        if key.endswith(('.units', '.habitable_rooms')) or key in {'housing.total_units', 'housing.market_units', 'housing.affordable_units', 'housing.affordable_habitable_rooms', 'housing.total_habitable_rooms', 'housing.mix_table.total_units', 'housing.mix_table.total_habitable_rooms'}:
            if type(value) is not int or value < 0:
                raise ValueError(f'{key}: expected a nonnegative integer')
        if key.endswith('.page') and (type(value) is not int or value < 1 or page_count is not None and value > page_count):
            raise ValueError('expected citation page outside source')
        if key.endswith(('.snippet', '.text', '.condition', '.words')) and not isinstance(value, str):
            raise ValueError('quoted correction must be text')
        if key.endswith('trigger_event') and value not in {'prior to Implementation', 'prior to Commencement', 'prior to Occupation', 'on completion of agreement', 'on demand / review', 'other'}:
            raise ValueError('invalid trigger classification')
        if key.endswith('.structure') and value not in {'fixed', 'cap', 'formula', 'rate', 'none'}:
            raise ValueError('invalid amount structure')
        if key.endswith('conditionality.kind') and value not in {'conditional', 'unconditional', 'unknown'}:
            raise ValueError('invalid conditionality')
        if key.endswith('claimed_affordable_basis') and value not in {'habitable_rooms', 'units', 'floorspace'}:
            raise ValueError('invalid housing basis')
        if key.endswith('index_linked') and type(value) is not bool:
            raise ValueError('index_linked must be boolean')
        if key.endswith(('value_gbp', 'claimed_affordable_pct')):
            from decimal import Decimal, InvalidOperation
            try:
                number = Decimal(str(value))
            except InvalidOperation as error:
                raise ValueError('invalid numeric correction') from error
            if type(value) is bool or not number.is_finite() or number < 0 or (key.endswith('claimed_affordable_pct') and number > 100):
                raise ValueError('invalid numeric correction')
        if key.endswith(('rows', 'subtotals', 'instalments')) and not isinstance(value, list):
            raise ValueError('table/instalment correction must be a list')


def validate_evidence(evidence, page_count):
    if not isinstance(evidence, list) or not evidence:
        raise ValueError('corrected claims require source evidence')
    for e in evidence:
        if not isinstance(e, dict) or type(e.get('pdf_page')) is not int or not 1 <= e['pdf_page'] <= page_count:
            raise ValueError('corrected evidence page outside source')
        if not isinstance(e.get('quote'), str) or not e['quote'].strip():
            raise ValueError('corrected evidence needs a source quotation')
        if not isinstance(e.get('clause'), str) or not isinstance(e.get('printed_page'), str):
            raise ValueError('corrected evidence needs clause and printed page (use unknown when unavailable)')


def added_claim(entry, page_count):
    claim = copy.deepcopy(entry.get('new_claim'))
    if entry.get('human_review_status') != 'added' or not isinstance(claim, dict) or claim.get('id') != entry.get('claim_id'):
        raise ValueError('new obligations require added status and a matching new_claim object')
    if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_-]{0,79}', claim['id']):
        raise ValueError('invalid new claim ID')
    if not isinstance(claim.get('title'), str) or not claim['title'].strip() or not isinstance(claim.get('interpretation'), str) or not claim['interpretation'].strip():
        raise ValueError('new claim requires title and interpretation')
    if claim.get('priority') not in {'high', 'medium', 'low'}:
        raise ValueError('new claim requires a priority')
    validate_evidence(claim.get('evidence'), page_count)
    validate_fields(claim.get('expected_fields', {}), page_count)
    claim['human_review_status'] = None
    claim['annotation_origin'] = 'human_added'
    claim.setdefault('ambiguity', None)
    claim.setdefault('expected_fields', {})
    return claim


def validate_decision(entry, previous, base_claim, page_count):
    status = entry.get('human_review_status')
    if status not in DECISIONS:
        raise ValueError('invalid human decision')
    if not isinstance(entry.get('human_reviewer'), str) or not entry['human_reviewer'].strip():
        raise ValueError('human decision requires a reviewer')
    try:
        date.fromisoformat(entry['human_review_date'])
    except (KeyError, ValueError, TypeError) as error:
        raise ValueError('human decision requires an ISO review date') from error
    if entry.get('human_notes') is not None and not isinstance(entry['human_notes'], str):
        raise ValueError('human notes must be text')
    if entry.get('previous_review_sha256') != previous:
        raise ValueError('stale/conflicting review; rebuild the dossier before superseding this decision')
    if status == 'added':
        added_claim(entry, page_count)
        if previous is not None or entry.get('corrections'):
            raise ValueError('addition must introduce a new claim, not supersede one')
    corrections = entry.get('corrections') or {}
    if not isinstance(corrections, dict) or set(corrections) - CLAIM_KEYS:
        raise ValueError('unsupported correction keys')
    if base_claim is None and corrections:
        raise ValueError('provenance/completeness decisions cannot mutate claim fields')
    if corrections and status != 'corrected':
        raise ValueError('changes require corrected status, not confirmation of unchanged wording')
    if status == 'corrected':
        if base_claim is None or not corrections or not any(base_claim.get(k) != v for k, v in corrections.items()):
            raise ValueError('corrected status requires an actual claim change')
        validate_evidence(corrections.get('evidence'), page_count)
        if 'expected_fields' in corrections:
            validate_fields(corrections['expected_fields'], page_count)
            # Keep fields explicit: removals are supported but recorded as full replacement.
        if 'interpretation' in corrections and not isinstance(corrections['interpretation'], str):
            raise ValueError('interpretation correction must be text')
    if base_claim is None and status == 'confirmed' and (not isinstance(entry.get('scope'), str) or not entry['scope'].strip()):
        raise ValueError('provenance/completeness confirmation needs an explicit review scope')
    return corrections


def apply_records(case, reviews_dir=None):
    base, provenance = load_case(case)
    view = copy.deepcopy(base)
    by_id = {c['id']: c for c in view['claims']}
    original = {c['id']: c for c in base['claims']}
    active, history = {}, []
    previous_record = None
    directory = Path(reviews_dir) if reviews_dir else Path(case) / 'reviews'
    files = sorted(directory.glob('*.json')) if directory.exists() else []
    binding = {'case_id': Path(case).name, 'provenance_sha256': digest(Path(case) / 'provenance.json'), 'source_sha256': base['source_sha256'], 'reference_version': base['reference_version'], 'reference_sha256': digest(Path(case) / 'reference.json')}
    for file in files:
        record = read_json(file)
        for key, value in binding.items():
            if record.get(key) != value:
                raise ValueError(f'stale review binding: {file.name} ({key})')
        if record.get('format') != FORMAT or record.get('previous_record_sha256') != previous_record:
            raise ValueError('review ledger hash chain invalid')
        record_sha = digest(file)
        seen = set()
        for entry in record['claims']:
            cid = entry['claim_id']
            if cid in seen:
                raise ValueError('duplicate review claim')
            if cid not in original and cid not in {'@provenance', '@completeness'}:
                new = added_claim(entry, provenance['pdf_pages'])
                original[cid] = new
                by_id[cid] = copy.deepcopy(new)
                view['claims'].append(by_id[cid])
            elif entry.get('human_review_status') == 'added':
                raise ValueError('addition collides with an existing claim')
            seen.add(cid)
            prior = active.get(cid, {}).get('record_sha256')
            # Corrected events describe a complete replacement against the agent original;
            # later confirmations can explicitly keep or reject that original view.
            corrections = validate_decision(entry, prior, original.get(cid), provenance['pdf_pages'])
            event = {**copy.deepcopy(entry), 'record_sha256': record_sha, 'record_file': file.name}
            active[cid] = event
            history.append(event)
            if cid in by_id:
                c = copy.deepcopy(original[cid])
                c.update(corrections)
                for key in ['human_review_status', 'human_reviewer', 'human_review_date', 'human_notes']:
                    c[key] = entry.get(key)
                c['review_record_sha256'] = record_sha
                if corrections:
                    c['annotation_origin'] = 'human_corrected'
                by_id[cid].clear()
                by_id[cid].update(c)
        previous_record = record_sha
    dispositioned = sum(c.get('human_review_status') in {'confirmed', 'corrected', 'rejected', 'added'} for c in view['claims'])
    complete = bool(view['claims']) and dispositioned == len(view['claims']) and all(active.get(k, {}).get('human_review_status') == 'confirmed' for k in ['@provenance', '@completeness'])
    view['status'] = 'human_review_complete' if complete else 'partially_human_reviewed' if active else base.get('status', 'awaiting_human_confirmation')
    return base, view, provenance, active, history


def _import_review(case, file, reviews_dir=None):
    base, view, provenance, active, history = apply_records(case, reviews_dir)
    payload = read_json(file)
    if payload.get('format') != FORMAT or payload.get('review_origin') != 'human_entered':
        raise ValueError('requires an exported human-entered review record; agent annotations are not human review')
    for key, want in {'case_id': Path(case).name, 'provenance_sha256': digest(Path(case) / 'provenance.json'), 'source_sha256': base['source_sha256'], 'reference_version': base['reference_version'], 'reference_sha256': digest(Path(case) / 'reference.json')}.items():
        if payload.get(key) != want:
            raise ValueError(f'review source/reference binding mismatch: {key}')
    directory = Path(reviews_dir) if reviews_dir else Path(case) / 'reviews'
    # Byte-identical replay is harmless; it must not append a duplicate decision.
    import_sha = digest(file)
    for old in sorted(directory.glob('*.json')) if directory.exists() else []:
        if read_json(old).get('import_sha256') == import_sha:
            return old
    if not isinstance(payload.get('claims'), list) or any(not isinstance(e, dict) for e in payload['claims']):
        raise ValueError('review claims must be a list of decision objects')
    entries = []
    seen = set()
    for e in payload.get('claims', []):
        if e.get('human_review_status') is None:
            continue
        cid = e.get('claim_id')
        originals = {c['id']: c for c in base['claims']}
        originals.update({event['claim_id']: event['new_claim'] for event in history if event.get('new_claim')})
        if cid in seen:
            raise ValueError('duplicate claim ID')
        if cid not in originals and cid not in {'@provenance', '@completeness'}:
            originals[cid] = added_claim(e, provenance['pdf_pages'])
        elif e.get('human_review_status') == 'added':
            raise ValueError('addition collides with an existing claim')
        seen.add(cid)
        validate_decision(e, active.get(cid, {}).get('record_sha256'), originals.get(cid), provenance['pdf_pages'])
        entries.append(copy.deepcopy(e))
    if not entries:
        raise ValueError('no submitted human decisions; keep this file as a draft')
    files = sorted(directory.glob('*.json')) if directory.exists() else []
    record = {**{k: payload[k] for k in ['format', 'case_id', 'provenance_sha256', 'source_sha256', 'reference_version', 'reference_sha256', 'review_origin']}, 'import_sha256': import_sha, 'imported_at_utc': datetime.now(timezone.utc).isoformat(), 'previous_record_sha256': digest(files[-1]) if files else None, 'claims': entries}
    # Sequence + exclusive creation detects concurrent writers rather than overwriting them.
    dest = directory / f'{len(files) + 1:06}-{import_sha[:12]}.json'
    if any(directory.glob(f'{len(files) + 1:06}-*.json')):
        raise ValueError('concurrent ledger write; retry after rebuilding')
    write_json(dest, record, exclusive=True)
    return dest


def review_summary(case, reviews_dir=None):
    base, view, provenance, active, history = apply_records(case, reviews_dir)
    counts = {s: sum(c.get('human_review_status') == s for c in view['claims']) for s in sorted(DECISIONS)}
    return {'case_id': Path(case).name, 'planning_reference': provenance['planning_reference'], 'source_sha256': provenance['sha256'], 'reference_sha256': digest(Path(case) / 'reference.json'), 'reference_version': base['reference_version'], 'status': view['status'], 'claims': len(view['claims']), 'agent_claims': len(base['claims']), 'human_added_claims': len(view['claims']) - len(base['claims']), 'decisions': counts, 'unreviewed': sum(c.get('human_review_status') is None for c in view['claims']), 'provenance_review': active.get('@provenance', {}).get('human_review_status'), 'completeness_review': active.get('@completeness', {}).get('human_review_status'), 'history_events': len(history), 'human_identity_authenticated': False}


def comparison_layers(case, audit, reviews_dir=None):
    import importlib.util
    module_spec = importlib.util.spec_from_file_location('validation_comparator', Path(__file__).with_name('compare-southwark-validation.py'))
    comparator = importlib.util.module_from_spec(module_spec)
    module_spec.loader.exec_module(comparator)
    base, view, provenance, active, history = apply_records(case, reviews_dir)
    if audit['agreement']['provenance']['sha256'] != provenance['sha256']:
        raise ValueError('audit source hash mismatch')
    spec_path = Path(case) / 'comparison-spec.json'
    spec = read_json(spec_path) if spec_path.exists() else {}
    base_has_fields = any(c.get('expected_fields') for c in base['claims'])
    view_has_fields = any(c.get('expected_fields') for c in view['claims'])
    if not view_has_fields:
        return {'agent_labelled': None, 'human_confirmed': None, 'comparison_status': 'No structured reference fields; no accuracy result.'}
    agent = comparator.compare(base, audit, spec) if base_has_fields else {'agent_labelled': None, 'rows': []}
    reviewed = comparator.compare(view, audit, spec)
    return {**agent, 'reference_status': view['status'], 'human_confirmed': reviewed['human_confirmed'], 'human_metric_note': reviewed['human_metric_note'], 'human_rows': [row for row in reviewed['rows'] if row['human_accepted']], 'review_events': len(history), 'comparison_status': 'Diagnostic comparison; human-confirmed subset available.' if reviewed['human_confirmed'] is not None else 'Diagnostic comparison; awaiting human-confirmed fields.'}


def import_review(case, file, reviews_dir=None):
    directory = Path(reviews_dir) if reviews_dir else Path(case) / 'reviews'
    directory.mkdir(parents=True, exist_ok=True)
    lock = directory / '.import-lock'
    try:
        lock.mkdir()
    except FileExistsError as error:
        raise ValueError('another review import is active; retry after it completes') from error
    try:
        return _import_review(case, file, directory)
    finally:
        lock.rmdir()
