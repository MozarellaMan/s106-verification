#!/usr/bin/env python3
"""Generate an offline claim/image review site from any compatible case directory.
Existing report.rs renders audit outcomes, not a blank human review workflow;
this wrapper renders source-first annotations and a CSV review sheet. No inference.
Output is an ignored local site with PDF page renders and a blank review CSV.
Change 2026-10-13: case/PDF/output arguments and optional loopback serving replace
committed page/site artifacts; the source PDF is hash-checked and rendered locally.
Run after changing reference.json; inspect the HTML and run file-link checks.
"""
import argparse, csv, hashlib, html, json, shutil, subprocess
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from functools import partial
from pathlib import Path
from review_cases import apply_records, comparison_layers, digest, read_json, review_summary, write_json
parser = argparse.ArgumentParser(description="Generate a local review site from a case's reference.json and provenance.json; no LLM calls.")
parser.add_argument('--case', type=Path, default=Path('validation/southwark-19-AP-1773'))
parser.add_argument('--pdf', type=Path, help='Locally retrieved original PDF; hash must match provenance')
parser.add_argument('--out', type=Path, help='Generated site directory (default data/processed/reviews/<case-name>)')
parser.add_argument('--audit', type=Path, help='Existing Rust audit JSON for dynamic comparison (never calls an LLM)')
parser.add_argument('--reviews', type=Path, help='Separate imported review ledger directory; default <case>/reviews')
parser.add_argument('--serve', action='store_true', help='Serve generated site on loopback until Ctrl-C')
parser.add_argument('--port', type=int, default=8766)
a = parser.parse_args()
case = a.case.resolve()
out = (a.out or Path('data/processed/reviews') / case.name).resolve()
if out == case or case in out.parents:
    parser.error('Output must be outside the versioned case directory')
base, r, p, active_reviews, history = apply_records(case, a.reviews)
pdf = a.pdf or case/'source/agreement.pdf'
if not pdf.is_file():
    parser.error('Original PDF missing. Retrieve the provenance source URL or supply --pdf PATH; no automatic network access.')
if hashlib.sha256(pdf.read_bytes()).hexdigest() != p['sha256'] or r['source_sha256'] != p['sha256']:
    parser.error('PDF/reference source hash mismatch; refusing to generate evidence')
expected = (case/'reference-freeze.sha256').read_text().split()[0]
if hashlib.sha256((case/'reference.json').read_bytes()).hexdigest() != expected:
    parser.error('Reference differs from its recorded freeze; version it explicitly before rendering')
pages = {e['pdf_page'] for c in r['claims'] for e in c['evidence']} | set(p.get('identity_evidence_pdf_pages', []))
if any(not isinstance(n, int) or n < 1 or n > p['pdf_pages'] for n in pages):
    parser.error('Evidence page outside source document')
if shutil.which('pdftoppm') is None:
    parser.error('pdftoppm is required to render source pages (Poppler)')
if (out/'reference.json').exists() and digest(out/'reference.json') != digest(case/'reference.json'):
    parser.error('Existing site belongs to another reference; select a new output directory to preserve reviews')
out.mkdir(parents=True, exist_ok=True)
(out/'source').mkdir(exist_ok=True)
(out/'evidence').mkdir(exist_ok=True)
shutil.copyfile(pdf, out/'source/agreement.pdf')
for name in ['reference.json', 'provenance.json', 'completeness.md', 'protocol.md']:
    if (case/name).is_file(): shutil.copyfile(case/name, out/name)
render_key = p['sha256'] + ':poppler-1400-v1'
cache_path = out/'render-cache.json'
cache_info = read_json(cache_path) if cache_path.exists() else {}
cache_valid = cache_info.get('key') == render_key
for n in sorted(pages):
    dest=out/f'evidence/page-{n:03}.png'
    if cache_valid and dest.is_file() and cache_info.get('pages', {}).get(str(n)) == digest(dest):
        continue
    # pdftoppm -singlefile avoids embedding source page numbering in its output name.
    subprocess.run(['pdftoppm','-f',str(n),'-l',str(n),'-singlefile','-scale-to','1400','-png',str(pdf),str(dest.with_suffix(''))], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    print(f'Rendered evidence page {n}', flush=True)
write_json(cache_path, {'key': render_key, 'pages': {str(n): digest(out/f'evidence/page-{n:03}.png') for n in sorted(pages)}})
summary = review_summary(case, a.reviews)
comparison = comparison_layers(case, read_json(a.audit), a.reviews) if a.audit else {'agent_labelled': None, 'human_confirmed': None, 'comparison_status': 'Audit unavailable; no extraction accuracy result.'}
summary.update({k: comparison.get(k) for k in ['agent_labelled', 'human_confirmed', 'comparison_status']})
write_json(out/'summary.json', summary)
write_json(out/'comparison.json', comparison)
write_json(out/'review-history.json', history)
write_json(out/'reviewed-reference.json', r)
report_lines = [f"# {p['planning_reference']} validation report", '', f"Source SHA-256: {p['sha256']}", f"Reference SHA-256: {digest(case/'reference.json')}", f"Review status: {summary['status']}", '', 'Agent-labelled and human-confirmed subsets are distinct. Imported decisions are reviewer attestations; identity and legal accuracy are not authenticated by this tool.', '', '## Review progress', json.dumps(summary['decisions'], sort_keys=True), f"Unreviewed claims: {summary['unreviewed']}", f"Provenance review: {summary['provenance_review']}", f"Completeness review: {summary['completeness_review']}", '', '## Comparison', f"Agent-labelled fields: {json.dumps(comparison.get('agent_labelled'))}", f"Human-confirmed fields: {json.dumps(comparison.get('human_confirmed'))}", comparison['comparison_status'], '', '## Differences and missing fields']
for layer, rows in [('Agent-labelled', comparison.get('rows', [])), ('Human-confirmed', comparison.get('human_rows', []))]:
    report_lines += ['', f'### {layer} subset']
    for row in rows:
        if row['outcome'] != 'agree': report_lines.append(f"- {row['claim']} / {row['field']}: expected {row['expected']!r}; extracted {row['actual']!r}; {row['outcome']}")
report_lines += ['', '## Citation differences']
for row in comparison.get('locator_checks', []):
    if row['outcome'] != 'agree': report_lines.append(f"- {row['field']}: expected PDF pages {row['allowed_pdf_pages']}; cited {row['actual']}")
report_lines += ['', '## Human correction history']
for event in history:
    report_lines.append(f"- {event['claim_id']}: {event['human_review_status']} by {event['human_reviewer']} on {event['human_review_date']}; record {event['record_sha256']}")
report_lines += ['', 'No actual-delivery, compliance or wrongdoing conclusion. Completeness requires its own explicit human decision.']
(out/'report.md').write_text('\n'.join(report_lines)+'\n')
label = p['planning_reference'] + ' — ' + p['site_address']

h=html.escape;cards=[]
review_originals = {c['id']: {**c, 'expected_fields': c.get('expected_fields', {})} for c in base['claims']}
for event in history:
    if event.get('new_claim'): review_originals[event['claim_id']] = {**event['new_claim'], 'expected_fields': event['new_claim'].get('expected_fields', {})}
for c in r['claims']:
    ev=[]
    for e in c['evidence']:
        coords=e.get('coordinates')
        image_path=f"evidence/page-{e['pdf_page']:03}.png"
        where=f"Approximate OCR region: {coords['bbox']} (normalised top-left; navigation only)" if coords else 'No reliable coordinates supplied; use full page.'
        ev.append(f'''<details><summary>PDF {e['pdf_page']} · printed {h(e['printed_page'])} · {h(e['clause'])}</summary>
        <p><a href="source/agreement.pdf#page={e['pdf_page']}">Open original PDF at page {e['pdf_page']}</a> · <a href="{image_path}">Full-resolution page image</a></p>
        <p class="small">{h(where)}</p><a href="{image_path}"><img loading="lazy" src="{image_path}" alt="Original scan PDF page {e['pdf_page']}"></a></details>
        <blockquote>{h(e['quote'])}</blockquote>''')
    amb=f'<p class="amb">Review ambiguity: {h(c["ambiguity"])}</p>' if c['ambiguity'] else ''
    status = c.get('human_review_status') or ''
    options = ''.join(f'<option value="{option}"'+(' selected' if status == option else '')+'>'+option+'</option>' for option in ['', 'confirmed', 'corrected', 'rejected', 'needs clarification'] + (['added'] if status == 'added' else []))
    badge = 'AGENT-PROPOSED · HUMAN CONFIRMATION PENDING' if not status else 'IMPORTED HUMAN DECISION: ' + status.upper()
    record_sha = active_reviews.get(c['id'], {}).get('record_sha256') or ''
    cards.append(f'''<article id="{h(c['id'])}" data-priority="{h(c['priority'])}" data-previous-review="{h(record_sha)}"><h2>{c['id']} · {h(c['title'])}</h2>
    <p class="tag">{h(badge)} · {h(c['priority'].upper())} PRIORITY</p>
    <p><b>Proposed interpretation:</b> {h(c['interpretation'])}</p>{amb}
    <div class="evidence">{''.join(ev)}</div>
    <fieldset><legend>Human review decision</legend><label>Decision <select class="decision">{options}</select></label>
    <label>Reviewer <input class="reviewer" value="{h(c.get('human_reviewer') or '')}"></label><label>Date <input class="date" type="date" value="{h(c.get('human_review_date') or '')}"></label><label>Notes / corrections <textarea class="notes">{h(c.get('human_notes') or '')}</textarea></label></fieldset><details><summary>Correct interpretation, structured values or source evidence</summary><p>Use corrected status for changes. Source evidence is required. Structured fields replace this claim’s field map; removed fields are recorded explicitly in history.</p><label>Interpretation<textarea class="interpretation-edit">{h(c['interpretation'])}</textarea></label><label>Expected fields (JSON)<textarea class="fields-edit" rows="8">{h(json.dumps(c.get('expected_fields', {}), indent=2, ensure_ascii=False))}</textarea></label><label>Evidence (JSON)<textarea class="evidence-edit" rows="10">{h(json.dumps(c['evidence'], indent=2, ensure_ascii=False))}</textarea></label></details></article>''')
page='''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>CASE_TITLE — human review dossier</title>
<style>body{margin:0;background:#f3f5f7;color:#172333;font:16px/1.55 system-ui,sans-serif}main{max-width:980px;margin:auto;padding:30px}h1{font-size:32px;line-height:1.2}h2{font-size:23px}a{color:#075f9f}article{background:white;margin:24px 0;padding:24px;border:1px solid #d3dae2;border-radius:8px}blockquote{margin:12px 0;padding:12px 16px;background:#f0f5f9;border-left:4px solid #256b97}.tag,.small{font-size:12px;color:#536273}.amb{background:#fff3d7;padding:12px}summary{cursor:pointer;font-weight:650}details img{display:block;max-width:100%;width:620px;margin:14px auto}fieldset{border:1px solid #cad3dc;margin-top:20px;display:grid;grid-template-columns:repeat(3,1fr);gap:12px}label{display:block;font-size:13px}input,select,textarea{display:block;box-sizing:border-box;width:100%;padding:8px;font:inherit}label:last-child{grid-column:1/-1}button{padding:10px 16px;background:#125a7e;color:white;border:0;border-radius:4px;cursor:pointer}.notice{padding:16px;background:#fff1cb}.tools{position:sticky;top:0;background:#f3f5f7;padding:10px;border-bottom:1px solid #ccd5df;z-index:1}@media(max-width:600px){main{padding:14px}fieldset{grid-template-columns:1fr}article{padding:16px}}@media print{.tools{display:none}article{break-inside:avoid}details img{width:100%}}</style><main>
<h1>CASE_TITLE</h1><p class="notice">REVIEW_NOTICE</p>
'''
page+=f'''<p><a href="source/agreement.pdf">Original agreement scan · {p['pdf_pages']} PDF pages</a> · <a href="provenance.json">Source provenance</a> · <a href="completeness.md">Separate completeness checklist</a> · <a href="human-review.csv">Review CSV</a> · <a href="report.md">Dynamic report</a> · <a href="summary.json">Review summary</a> · <a href="comparison.json">Extraction comparison</a> · <a href="review-history.json">Correction history</a></p><p class="small">Retrieved {h(p['retrieval_date_europe_london'])}. SHA-256: {p['sha256']}<br><a href="{h(p['source_url'])}">Official agreement download</a> · <a href="{h(p['register_url'])}">Official application register</a></p>
<p>Start with the high-priority claims. Expand a source-page panel to inspect the scan; quotes stay readable beside the proposed meaning. Imported decisions appear on the cards. New edits are browser drafts until exported and imported. Confirmed/corrected decisions require a human name and date. Use corrected status and edit the fields/evidence for corrections. Export preserves the source/reference binding and previous review identifiers. Quotation typography and line breaks are normalised; source images govern. Approximate OCR coordinates are navigation aids.</p>
<p>Independence limits: {h(r.get('blindness', 'Agent-proposed extraction; human verification is pending.'))} See <a href="protocol.md">protocol</a> and reference.json for the recorded extraction/contact history.</p>
<p>Provenance evidence pages: {', '.join(f'<a href="evidence/page-{n:03}.png">PDF {n}</a>' for n in p.get('identity_evidence_pdf_pages', []))}. {h(p.get('identity_reason', 'Confirm source identity directly against the document.'))}</p>
<div class="tools"><button id="filter">Show high-priority claims only</button> <button id="export">Export human review JSON</button> <span id="message" role="status"></span></div>
'''+''.join(cards)
# Provenance and completeness are separate review decisions; neither is inferred from claim counts.
for cid, title in [('@provenance', 'Source provenance review'), ('@completeness', 'Completeness review')]:
    e = active_reviews.get(cid, {})
    options = ''.join(f'<option value="{option}"'+(' selected' if e.get('human_review_status', '') == option else '')+'>'+option+'</option>' for option in ['', 'confirmed', 'needs clarification'])
    page += f'<fieldset class="scope-review" data-review-id="{cid}" data-previous-review="{h(e.get("record_sha256", ""))}"><legend>{title}</legend><label>Decision<select class="decision">{options}</select></label><label>Reviewer<input class="reviewer" value="{h(e.get("human_reviewer") or "")}"></label><label>Date<input class="date" type="date" value="{h(e.get("human_review_date") or "")}"></label><label>Scope reviewed<textarea class="scope">{h(e.get("scope") or "")}</textarea></label><label>Notes<textarea class="notes">{h(e.get("human_notes") or "")}</textarea></label></fieldset>'
page+='''</main><script>
const originalClaims = ORIGINAL_CLAIMS;
const binding = REVIEW_BINDING;
const activeReviews = ACTIVE_REVIEWS;
let highOnly=false;document.querySelector('#filter').onclick=()=>{highOnly=!highOnly;document.querySelectorAll('article').forEach(a=>a.hidden=highOnly&&a.dataset.priority!=='high');document.querySelector('#filter').textContent=highOnly?'Show all claims':'Show high-priority claims only';};
const canonical = v => Array.isArray(v)?v.map(canonical):v&&typeof v==='object'?Object.fromEntries(Object.keys(v).sort().map(k=>[k,canonical(v[k])])):v;
const same = (a,b)=>JSON.stringify(canonical(a))===JSON.stringify(canonical(b));
document.querySelector('#export').onclick=()=>{try{
let rows=Array.from(document.querySelectorAll('article,.scope-review')).map(a=>{
let row={claim_id:a.id||a.dataset.reviewId,human_review_status:a.querySelector('.decision').value||null,human_reviewer:a.querySelector('.reviewer').value||null,human_review_date:a.querySelector('.date').value||null,human_notes:a.querySelector('.notes').value||null,previous_review_sha256:a.dataset.previousReview||null};
if(a.classList.contains('scope-review')){row.scope=a.querySelector('.scope').value;}else{
let original=originalClaims[row.claim_id];let values={interpretation:a.querySelector('.interpretation-edit').value,expected_fields:JSON.parse(a.querySelector('.fields-edit').value),evidence:JSON.parse(a.querySelector('.evidence-edit').value)};
row.corrections=Object.fromEntries(Object.entries(values).filter(([key,val])=>!same(val,original[key])));
if(row.human_review_status==='corrected' && Object.keys(row.corrections).length){row.corrections.evidence=values.evidence;}
if(Object.keys(row.corrections).length && row.human_review_status!=='corrected'){throw Error(row.claim_id+': choose corrected for changed fields.');}
}
const previous=activeReviews[row.claim_id];
if(previous && previous.human_review_status==='added' && row.human_review_status==='added'){return null;}
if(previous && ['human_review_status','human_reviewer','human_review_date','human_notes','corrections','scope'].every(k=>same(row[k]??(k==='corrections'?{}:null),previous[k]??(k==='corrections'?{}:null)))){return null;}
return row;}).filter(r=>r&&r.human_review_status);
if(!rows.length){throw Error('No submitted decisions. Keep your edits as a draft.');}
if(rows.some(r=>!r.human_reviewer||!r.human_review_date)){throw Error('Add reviewer and date for each submitted decision.');}
let data={...binding,format:'s106-human-review/1',review_origin:'human_entered',claims:rows};let url=URL.createObjectURL(new Blob([JSON.stringify(data,null,2)],{type:'application/json'}));let link=document.createElement('a');link.href=url;link.download=REVIEW_FILENAME;link.click();URL.revokeObjectURL(url);document.querySelector('#message').textContent='Exported. Import this record and rebuild to update reports; original annotations stay unchanged.';
}catch(error){document.querySelector('#message').textContent=error.message;}};
</script></html>'''
script_json = lambda value: json.dumps(value, ensure_ascii=False).replace('<', '\\u003c')
page = page.replace('ORIGINAL_CLAIMS', script_json(review_originals))
page = page.replace('ACTIVE_REVIEWS', script_json(active_reviews))
page = page.replace('REVIEW_BINDING', script_json({'case_id': case.name, 'provenance_sha256': digest(case/'provenance.json'), 'source_sha256': p['sha256'], 'reference_version': base['reference_version'], 'reference_sha256': digest(case/'reference.json')}))
page = page.replace('REVIEW_FILENAME', script_json(case.name+'-human-review.json'))
page = page.replace('REVIEW_NOTICE', h(f"Review status: {summary['status']}. {summary['unreviewed']} of {summary['claims']} claims await human decisions. Provenance and completeness require separate confirmation. No delivery/compliance conclusion."))
page=page.replace('CASE_TITLE', h(label))
page=page.replace('SOURCE_HASH',json.dumps(p['sha256'])).replace('REFERENCE_VERSION',str(r['reference_version']))
(out/'dossier.html').write_text(page)
# Do not overwrite a CSV the human may have started filling in.
if not (out/'human-review.csv').exists():
 with (out/'human-review.csv').open('w',newline='') as f:
    writer=csv.writer(f, lineterminator="\n");writer.writerow(['claim_id','claim','pdf_pages','human_review_status','human_reviewer','human_review_date','human_notes'])
    for c in r['claims']:writer.writerow([c['id'],c['title'],','.join(str(e['pdf_page']) for e in c['evidence']),'','','',''])
print(f"Generated {out/'dossier.html'} with {len(cards)} claims and {len(history)} imported human review events.")
if a.serve:
    server = ThreadingHTTPServer(('127.0.0.1', a.port), partial(SimpleHTTPRequestHandler, directory=str(out)))
    print(f'Open http://127.0.0.1:{a.port}/dossier.html (Ctrl-C to stop)', flush=True)
    try: server.serve_forever()
    except KeyboardInterrupt: pass
    finally: server.server_close()
