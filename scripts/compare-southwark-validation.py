#!/usr/bin/env python3
"""Compare frozen agent-labelled claims to a Rust audit, without LLM calls.

Use for this milestone's schema fields, not legal completeness certification.
Checked existing eval.rs/ocrbench.rs: neither compares claim-level legal structures or
keeps human subsets separate; this wrapper adds that comparison without replacing them.
Output: field agreements/differences/missing, plus citation mismatches. Structured fields
count as one comparison. Human metrics require confirmed status, reviewer and review date.
Believe the plumbing only after tests/python/test_validation_comparison.py catches planted errors.
"""
import argparse, hashlib, json, re
from decimal import Decimal
from pathlib import Path

MISSING = '<missing>'
def norm_name(s):
    return re.sub(r'[^a-z0-9]', '', s.lower().replace('’', "'"))
def lookup(agreement, path):
    bits = path.split('.')
    if bits[0] == 'contribution':
        hits = [c for c in agreement['contributions'] if norm_name(c['name']) == norm_name(bits[1])]
        if len(hits) != 1:
            return MISSING
        node = hits[0]; bits = bits[2:]
    elif bits[0] == 'tenure':
        hits = [t for t in agreement['housing']['tenures'] if t['tenure'] == bits[1]]
        if len(hits) != 1:
            return MISSING
        node = hits[0]; bits = bits[2:]
    else:
        node = agreement
    for bit in bits:
        if not isinstance(node, dict) or bit not in node:
            return MISSING
        node = node[bit]
    return node

def equal(a, b):
    if a == MISSING or b == MISSING:
        return False
    # Decimal money formatting differs; units and categorical labels remain exact.
    if isinstance(a, str) and isinstance(b, str):
        try: return Decimal(a) == Decimal(b)
        except Exception: pass
    return a == b

def compare(reference, audit):
    agreement = audit['agreement']
    if agreement['provenance']['sha256'] != reference['source_sha256']:
        raise ValueError('source hash mismatch')
    rows=[]; confirmed=[]
    for c in reference['claims']:
        human = c['human_review_status'] == 'confirmed'
        if human and not (c.get('human_reviewer') and c.get('human_review_date')):
            raise ValueError('confirmed claim lacks human reviewer/date')
        for field, want in c['expected_fields'].items():
            got = lookup(agreement, field)
            row=dict(claim=c['id'], field=field, expected=want, actual=got,
                     outcome='missing' if got == MISSING else 'agree' if equal(want,got) else 'different')
            rows.append(row)
            if human: confirmed.append(row)
    if not rows:
        raise ValueError('no reference fields: cannot report validation')
    def count(items):
        return {'compared_fields':len(items), **{s:sum(x['outcome']==s for x in items) for s in ['agree','different','missing']}}
    locators=[]
    # Allowed pages from image-read claims; include actual duplicate restatement PDF45,
    # not Schedule1 title PDF44. These are source-based locators, not model quotations.
    for field, pages in {
        'housing.total_units_evidence.page':[14,45],
        'housing.market_units_evidence.page':[22],
        'housing.affordable_units_evidence.page':[5,91],
        'housing.claimed_affordable_evidence.page':[14,45],
        'housing.mix_table.evidence.page':[91],
        'tenure.Social Rent.evidence.page':[30,91],
        'tenure.Intermediate.evidence.page':[21,91],
    }.items():
        got=lookup(agreement,field)
        locators.append(dict(field=field,allowed_pdf_pages=pages,actual=got,outcome='agree' if got in pages else 'different'))
    names={f.split('.')[1] for c in reference['claims'] for f in c['expected_fields'] if f.startswith('contribution.')}
    present=[n for n in sorted(names) if lookup(agreement,f'contribution.{n}.name') != MISSING]
    return dict(reference_status=reference['status'],agent_labelled=count(rows),
                human_confirmed=count(confirmed) if confirmed else None,
                human_metric_note='No human-confirmed reference fields; accuracy unavailable.' if not confirmed else 'Reviewed subset only; not full-deed completeness.',
                contribution_coverage={'inventory_names':len(names),'represented':len(present),'missing':sorted(names-set(present)),'scope':'Only structured F01-F14; ancillary obligations and unrepresented qualifiers excluded. Not full-deed recall.'},
                rows=rows,locator_checks=locators,
                verification_counts={s:sum(x['status']==s for x in audit['verification']) for s in ['PASS','REVIEW','FAIL','SKIP']})

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--case',type=Path,default=Path('validation/southwark-19-AP-1773'));p.add_argument('--audit',type=Path);p.add_argument('--out',type=Path);a=p.parse_args()
    reference=json.loads((a.case/'reference.json').read_text())
    expected=(a.case/'reference-freeze.sha256').read_text().split()[0]
    if hashlib.sha256((a.case/'reference.json').read_bytes()).hexdigest()!=expected:
        raise ValueError('reference changed: retain original and document/version a new human-reviewed reference')
    audit=json.loads((a.audit or a.case/'results/rust-audit.json').read_text())
    result=compare(reference,audit)
    dest=a.out or a.case/'results/comparison.json';dest.parent.mkdir(parents=True, exist_ok=True);dest.write_text(json.dumps(result,indent=2,ensure_ascii=False)+'\n')
    print(json.dumps({k:v for k,v in result.items() if k not in ['rows','locator_checks']},indent=2))
if __name__=='__main__': main()
