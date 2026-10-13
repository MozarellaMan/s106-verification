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
parser = argparse.ArgumentParser(description="Generate a local review site from a case's reference.json and provenance.json; no LLM calls.")
parser.add_argument('--case', type=Path, default=Path('validation/southwark-19-AP-1773'))
parser.add_argument('--pdf', type=Path, help='Locally retrieved original PDF; hash must match provenance')
parser.add_argument('--out', type=Path, help='Generated site directory (default data/processed/reviews/<case-name>)')
parser.add_argument('--serve', action='store_true', help='Serve generated site on loopback until Ctrl-C')
parser.add_argument('--port', type=int, default=8766)
a = parser.parse_args()
case = a.case.resolve()
out = (a.out or Path('data/processed/reviews') / case.name).resolve()
if out == case or case in out.parents:
    parser.error('Output must be outside the versioned case directory')
r=json.loads((case/'reference.json').read_text());p=json.loads((case/'provenance.json').read_text())
pdf = a.pdf or case/'source/agreement.pdf'
if not pdf.is_file():
    parser.error('Original PDF missing. Retrieve the provenance source URL or supply --pdf PATH; no automatic network access.')
if hashlib.sha256(pdf.read_bytes()).hexdigest() != p['sha256'] or r['source_sha256'] != p['sha256']:
    parser.error('PDF/reference source hash mismatch; refusing to generate evidence')
expected = (case/'reference-freeze.sha256').read_text().split()[0]
if hashlib.sha256((case/'reference.json').read_bytes()).hexdigest() != expected:
    parser.error('Reference differs from its recorded freeze; version it explicitly before rendering')
for c in r['claims']:
    if c.get('human_review_status') is not None:
        parser.error('This renderer expects proposed annotations with blank human statuses; retain human reviews in separate exported records')
pages = {e['pdf_page'] for c in r['claims'] for e in c['evidence']} | set(p.get('identity_evidence_pdf_pages', []))
if any(not isinstance(n, int) or n < 1 or n > p['pdf_pages'] for n in pages):
    parser.error('Evidence page outside source document')
if shutil.which('pdftoppm') is None:
    parser.error('pdftoppm is required to render source pages (Poppler)')
out.mkdir(parents=True, exist_ok=True)
(out/'source').mkdir(exist_ok=True)
(out/'evidence').mkdir(exist_ok=True)
shutil.copyfile(pdf, out/'source/agreement.pdf')
for name in ['reference.json', 'provenance.json', 'completeness.md', 'protocol.md']:
    if (case/name).is_file(): shutil.copyfile(case/name, out/name)
for n in sorted(pages):
    dest=out/f'evidence/page-{n:03}.png'
    # pdftoppm -singlefile avoids embedding source page numbering in its output name.
    subprocess.run(['pdftoppm','-f',str(n),'-l',str(n),'-singlefile','-scale-to','1400','-png',str(pdf),str(dest.with_suffix(''))], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    print(f'Rendered evidence page {n}', flush=True)
label = p['planning_reference'] + ' — ' + p['site_address']

h=html.escape;cards=[]
for c in r['claims']:
    ev=[]
    for e in c['evidence']:
        coords=e['coordinates']
        image_path=f"evidence/page-{e['pdf_page']:03}.png"
        where=f"Approximate OCR region: {coords['bbox']} (normalised top-left; navigation only)" if coords else 'No reliable coordinates supplied; use full page.'
        ev.append(f'''<details><summary>PDF {e['pdf_page']} · printed {h(e['printed_page'])} · {h(e['clause'])}</summary>
        <p><a href="source/agreement.pdf#page={e['pdf_page']}">Open original PDF at page {e['pdf_page']}</a> · <a href="{image_path}">Full-resolution page image</a></p>
        <p class="small">{h(where)}</p><a href="{image_path}"><img loading="lazy" src="{image_path}" alt="Original scan PDF page {e['pdf_page']}"></a></details>
        <blockquote>{h(e['quote'])}</blockquote>''')
    amb=f'<p class="amb">Review ambiguity: {h(c["ambiguity"])}</p>' if c['ambiguity'] else ''
    cards.append(f'''<article id="{h(c['id'])}" data-priority="{h(c['priority'])}"><h2>{c['id']} · {h(c['title'])}</h2>
    <p class="tag">AGENT-PROPOSED · {c['priority'].upper()} PRIORITY · HUMAN CONFIRMATION PENDING</p>
    <p><b>Proposed interpretation:</b> {h(c['interpretation'])}</p>{amb}
    <div class="evidence">{''.join(ev)}</div>
    <fieldset><legend>Human review (initially blank)</legend><label>Decision <select class="decision"><option value=""></option><option>confirmed</option><option>rejected</option><option>needs clarification</option></select></label>
    <label>Reviewer <input class="reviewer"></label><label>Date <input class="date" type="date"></label><label>Notes / corrections <textarea class="notes"></textarea></label></fieldset></article>''')
page='''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>CASE_TITLE — human review dossier</title>
<style>body{margin:0;background:#f3f5f7;color:#172333;font:16px/1.55 system-ui,sans-serif}main{max-width:980px;margin:auto;padding:30px}h1{font-size:32px;line-height:1.2}h2{font-size:23px}a{color:#075f9f}article{background:white;margin:24px 0;padding:24px;border:1px solid #d3dae2;border-radius:8px}blockquote{margin:12px 0;padding:12px 16px;background:#f0f5f9;border-left:4px solid #256b97}.tag,.small{font-size:12px;color:#536273}.amb{background:#fff3d7;padding:12px}summary{cursor:pointer;font-weight:650}details img{display:block;max-width:100%;width:620px;margin:14px auto}fieldset{border:1px solid #cad3dc;margin-top:20px;display:grid;grid-template-columns:repeat(3,1fr);gap:12px}label{display:block;font-size:13px}input,select,textarea{display:block;box-sizing:border-box;width:100%;padding:8px;font:inherit}label:last-child{grid-column:1/-1}button{padding:10px 16px;background:#125a7e;color:white;border:0;border-radius:4px;cursor:pointer}.notice{padding:16px;background:#fff1cb}.tools{position:sticky;top:0;background:#f3f5f7;padding:10px;border-bottom:1px solid #ccd5df;z-index:1}@media(max-width:600px){main{padding:14px}fieldset{grid-template-columns:1fr}article{padding:16px}}@media print{.tools{display:none}article{break-inside:avoid}details img{width:100%}}</style><main>
<h1>CASE_TITLE</h1><p class="notice"><b>Awaiting human confirmation.</b> Every annotation below was proposed by an agent. No claim or completeness decision has been human-reviewed. Verify the original images, then record your decisions. This dossier makes no finding about delivery or compliance.</p>
'''
page+=f'''<p><a href="source/agreement.pdf">Original agreement scan · {p['pdf_pages']} PDF pages</a> · <a href="provenance.json">Source provenance</a> · <a href="completeness.md">Separate completeness checklist</a> · <a href="human-review.csv">Blank review CSV</a></p><p class="small">Retrieved {h(p['retrieval_date_europe_london'])}. SHA-256: {p['sha256']}<br><a href="{h(p['source_url'])}">Official agreement download</a> · <a href="{h(p['register_url'])}">Official application register</a></p>
<p>Start with the high-priority claims. Expand a source-page panel to inspect the scan; quotes stay readable beside the proposed meaning. Blank decisions are intentional. “Confirmed” requires a human name and date. Export downloads a review record; it does not modify the repository or promote annotations automatically. Quotation typography and line breaks are normalised; source images govern. Approximate OCR coordinates are navigation aids.</p>
<p>Independence limits: {h(r.get('blindness', 'Agent-proposed extraction; human verification is pending.'))} See <a href="protocol.md">protocol</a> and reference.json for the recorded extraction/contact history.</p>
<p>Provenance evidence pages: {', '.join(f'<a href="evidence/page-{n:03}.png">PDF {n}</a>' for n in p.get('identity_evidence_pdf_pages', []))}. {h(p.get('identity_reason', 'Confirm source identity directly against the document.'))}</p>
<div class="tools"><button id="filter">Show high-priority claims only</button> <button id="export">Export human review JSON</button> <span id="message" role="status"></span></div>
'''+''.join(cards)
page+='''</main><script>
let highOnly=false;document.querySelector('#filter').onclick=()=>{highOnly=!highOnly;document.querySelectorAll('article').forEach(a=>a.hidden=highOnly&&a.dataset.priority!=='high');document.querySelector('#filter').textContent=highOnly?'Show all claims':'Show high-priority claims only';};
document.querySelector('#export').onclick=()=>{let rows=Array.from(document.querySelectorAll('article')).map(a=>({claim_id:a.id,human_review_status:a.querySelector('.decision').value||null,human_reviewer:a.querySelector('.reviewer').value||null,human_review_date:a.querySelector('.date').value||null,human_notes:a.querySelector('.notes').value||null}));if(rows.some(r=>r.human_review_status&&(!r.human_reviewer||!r.human_review_date))){document.querySelector('#message').textContent='Add reviewer and date for each decision.';return;}let data={source_sha256:SOURCE_HASH,reference_version:REFERENCE_VERSION,review_origin:'human-entered; awaiting ingestion and audit',claims:rows};let url=URL.createObjectURL(new Blob([JSON.stringify(data,null,2)],{type:'application/json'}));let a=document.createElement('a');a.href=url;a.download='southwark-human-review.json';a.click();URL.revokeObjectURL(url);document.querySelector('#message').textContent='Review record exported; repository annotations remain unchanged.';};
</script></html>'''
page=page.replace('CASE_TITLE', h(label))
page=page.replace('SOURCE_HASH',json.dumps(p['sha256'])).replace('REFERENCE_VERSION',str(r['reference_version']))
(out/'dossier.html').write_text(page)
# Do not overwrite a CSV the human may have started filling in.
if not (out/'human-review.csv').exists():
 with (out/'human-review.csv').open('w',newline='') as f:
    writer=csv.writer(f, lineterminator="\n");writer.writerow(['claim_id','claim','pdf_pages','human_review_status','human_reviewer','human_review_date','human_notes'])
    for c in r['claims']:writer.writerow([c['id'],c['title'],','.join(str(e['pdf_page']) for e in c['evidence']),'','','',''])
print(f"Generated {out/'dossier.html'} with {len(cards)} proposed claims; human fields remain blank.")
if a.serve:
    server = ThreadingHTTPServer(('127.0.0.1', a.port), partial(SimpleHTTPRequestHandler, directory=str(out)))
    print(f'Open http://127.0.0.1:{a.port}/dossier.html (Ctrl-C to stop)', flush=True)
    try: server.serve_forever()
    except KeyboardInterrupt: pass
    finally: server.server_close()
