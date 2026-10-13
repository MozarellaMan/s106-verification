#!/usr/bin/env python3
"""Register cases, append human review records and build reports locally.

Extends the existing renderer/comparator; no scraper or LLM calls. Registry inputs
and review records can be committed separately from ignored PDFs/sites/reports.
See docs/review-workflow.md. Synthetic multi-case controls certify the plumbing.
"""
import argparse
import hashlib
import html
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

from review_cases import import_review, read_json, review_summary, write_json

ROOT = Path(__file__).resolve().parents[1]


def registry(path):
    data = read_json(path) if path.exists() else {'format': 's106-case-registry/1', 'cases': []}
    if data.get('format') != 's106-case-registry/1':
        raise ValueError('unknown case registry format')
    ids = [e['id'] for e in data['cases']]
    if len(ids) != len(set(ids)):
        raise ValueError('duplicate case IDs')
    for e in data['cases']:
        if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9-]{0,79}', e['directory']):
            raise ValueError('unsafe case directory')
        if not re.fullmatch(r'[a-z0-9][a-z0-9-]{0,79}', e['id']):
            raise ValueError('unsafe case ID')
    return data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--registry', type=Path, default=ROOT / 'validation/cases.json')
    commands = parser.add_subparsers(dest='command', required=True)
    commands.add_parser('list')
    add = commands.add_parser('add')
    add.add_argument('--id', required=True)
    add.add_argument('--pdf', type=Path, required=True)
    add.add_argument('--planning-ref', required=True)
    add.add_argument('--site-address', required=True)
    add.add_argument('--council', required=True)
    add.add_argument('--source-url', required=True)
    add.add_argument('--register-url', required=True)
    add.add_argument('--retrieved-at', help='Actual retrieval timestamp, if known; registering a local file is not retrieval')
    add.add_argument('--audit', type=Path, help='Existing audit JSON, if available; no extraction call')
    add.add_argument('--reference', type=Path, help='Source-prepared agent annotation JSON; otherwise start with no claims')
    review = commands.add_parser('import-review')
    review.add_argument('case_id')
    review.add_argument('record', type=Path)
    build = commands.add_parser('build')
    build.add_argument('--case', action='append', help='Select cases (repeatable); default all registered cases')
    build.add_argument('--out', type=Path, default=ROOT / 'data/processed/review-hub')
    build.add_argument('--serve', action='store_true')
    build.add_argument('--port', type=int, default=8767)
    args = parser.parse_args()
    args.registry = args.registry.resolve()
    data = registry(args.registry)
    root = args.registry.parent
    if args.command == 'add':
        if not re.fullmatch(r'[a-z0-9][a-z0-9-]{0,79}', args.id) or any(e['id'] == args.id for e in data['cases']):
            raise ValueError('invalid or existing case ID')
        source = args.pdf.resolve()
        pdf_sha = hashlib.sha256(source.read_bytes()).hexdigest()
        info = subprocess.run(['pdfinfo', str(source)], capture_output=True, text=True, check=True).stdout
        match = re.search(r'^Pages:\s+(\d+)', info, re.M)
        if not match:
            raise ValueError('cannot establish PDF page count')
        pages = int(match.group(1))
        reference = read_json(args.reference) if args.reference else {'reference_version': 1, 'source_sha256': pdf_sha, 'status': 'awaiting_annotations', 'author': 'not yet annotated', 'claims': [], 'contacts': []}
        if reference['source_sha256'] != pdf_sha or reference.get('status') in {'human_review_complete', 'partially_human_reviewed'} or any(c.get('human_review_status') is not None for c in reference['claims']):
            raise ValueError('reference must match PDF and contain only agent-proposed annotations')
        from review_cases import validate_evidence, validate_fields
        for c in reference['claims']:
            validate_evidence(c['evidence'], pages)
            validate_fields(c.get('expected_fields', {}), pages)
        if len({c['id'] for c in reference['claims']}) != len(reference['claims']):
            raise ValueError('duplicate claim IDs')
        case = root / args.id
        if case.exists():
            raise ValueError('case directory already exists; refusing to replace inputs')
        pdf_dest = case / 'source/agreement.pdf'
        provenance = {'planning_reference': args.planning_ref, 'site_address': args.site_address, 'council': args.council, 'source_url': args.source_url, 'register_url': args.register_url, 'retrieved_at_utc': args.retrieved_at, 'retrieval_date_europe_london': 'not recorded', 'sha256': pdf_sha, 'bytes': source.stat().st_size, 'pdf_pages': pages, 'instrument_type': 'unknown; source identity awaiting inspection', 'apparent_execution_date': None, 'identity_evidence_pdf_pages': [], 'identity_reason': 'Local document registered; execution/type and official-register relationship require source inspection and human review.', 'human_review_status': None}
        if args.retrieved_at:
            from datetime import datetime, timezone
            from zoneinfo import ZoneInfo
            timestamp = datetime.fromisoformat(args.retrieved_at.replace('Z', '+00:00'))
            if timestamp.tzinfo is None:
                raise ValueError('retrieval timestamp must include timezone')
            provenance['retrieved_at_utc'] = timestamp.astimezone(timezone.utc).isoformat()
            provenance['retrieval_date_europe_london'] = timestamp.astimezone(ZoneInfo('Europe/London')).date().isoformat()
        if args.audit and read_json(args.audit)['agreement']['provenance']['sha256'] != pdf_sha:
            raise ValueError('audit source hash mismatch')
        case.mkdir(parents=True)
        pdf_dest.parent.mkdir()
        shutil.copyfile(source, pdf_dest)
        write_json(case / 'reference.json', reference)
        write_json(case / 'provenance.json', provenance)
        (case / 'reference-freeze.sha256').write_text(hashlib.sha256((case / 'reference.json').read_bytes()).hexdigest() + '  reference.json\n')
        (case / 'completeness.md').write_text('# Human completeness review\n\nInitially unchecked. Read every definition, operative schedule, annex and cross-reference. Record scope and unresolved exclusions in a separate @completeness review.\n')
        (case / 'protocol.md').write_text('# Annotation provenance\n\nRegistering a document is not verifying it. Record annotation author, source image-reading scope and any prior model-output exposure in reference.json. Human review records are separate.\n')
        entry = {'id': args.id, 'directory': args.id, 'pdf': str(pdf_dest.relative_to(root)), 'audit': str(args.audit.resolve()) if args.audit else None}
        data['cases'].append(entry)
        write_json(args.registry, data)
        print(f'Registered {args.id}: {pages} pages; {len(reference["claims"])} proposed claims; human review pending.')
    elif args.command == 'list':
        for e in data['cases']:
            summary = review_summary(root / e['directory'])
            summary['case_id'] = e['id']
            print(json.dumps(summary, ensure_ascii=False))
    elif args.command == 'import-review':
        entry = next((e for e in data['cases'] if e['id'] == args.case_id), None)
        if entry is None:
            raise ValueError('unknown case ID')
        print(import_review(root / entry['directory'], args.record))
    elif args.command == 'build':
        selected = data['cases'] if not args.case else [e for e in data['cases'] if e['id'] in args.case]
        if not selected or args.case and set(args.case) - {e['id'] for e in selected}:
            raise ValueError('no cases selected or unknown case ID')
        out = args.out.resolve()
        if out == root or root in out.parents:
            raise ValueError('generated hub must be outside versioned registry inputs')
        out.mkdir(parents=True, exist_ok=True)
        summaries = []
        for e in selected:
            case = root / e['directory']
            command = [sys.executable, str(ROOT / 'scripts/build-southwark-dossier.py'), '--case', str(case), '--pdf', str(root / e['pdf']), '--out', str(out / e['id'])]
            if e.get('audit') and (root / e['audit']).is_file():
                command += ['--audit', str(root / e['audit'])]
            subprocess.run(command, check=True)
            case_summary = read_json(out / e['id'] / 'summary.json')
            case_summary['case_id'] = e['id']
            summaries.append(case_summary)
        write_json(out / 'summary.json', {'format': 's106-review-hub/1', 'cases': summaries, 'note': 'Per-case diagnostic results; no pooled accuracy or legal/delivery conclusion.'})
        rows = ''.join(f'<tr><td><a href="{html.escape(s["case_id"])}/dossier.html">{html.escape(s["planning_reference"])}</a></td><td>{html.escape(s["status"])}</td><td>{s["claims"]}</td><td>{s["decisions"]["confirmed"]}</td><td>{s["decisions"]["corrected"]}</td><td>{s["decisions"]["rejected"]}</td><td>{s["decisions"]["added"]}</td><td>{s["unreviewed"]}</td><td>{html.escape(s.get("comparison_status", "Audit unavailable"))}</td></tr>' for s in summaries)
        (out / 'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Validation review hub</title><style>body{font:16px/1.5 system-ui;margin:40px;color:#173040}table{border-collapse:collapse;width:100%}td,th{padding:12px;border-bottom:1px solid #ddd;text-align:left}a{color:#096d9c}</style><h1>Validation review hub</h1><p>Cases can expand while human review proceeds. Agent annotations and human corrections remain separate. Generated reports reflect imported review records; browser drafts are not imported automatically.</p><p><a href="summary.json">Machine-readable cross-case report</a></p><table><tr><th>Application</th><th>Review status</th><th>Claims</th><th>Confirmed</th><th>Corrected</th><th>Rejected</th><th>Added by human</th><th>Unreviewed</th><th>Comparison</th></tr>' + rows + '</table>')
        print(f'Hub: {out / "index.html"}')
        if args.serve:
            from functools import partial
            from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
            server = ThreadingHTTPServer(('127.0.0.1', args.port), partial(SimpleHTTPRequestHandler, directory=str(out)))
            print(f'Open http://127.0.0.1:{args.port}/index.html', flush=True)
            try:
                server.serve_forever()
            except KeyboardInterrupt:
                pass
            finally:
                server.server_close()


if __name__ == '__main__':
    try:
        main()
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(f'Validation workflow error: {error}')
