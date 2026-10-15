#!/usr/bin/env python3
"""Build two damaged variants of data/contractnli/sample40.jsonl (stdlib only, deterministic).

  python3 -I corrupt.py            # writes the files below next to sample40.jsonl

Outputs (in data/contractnli/):
  sample40_ocr.jsonl     character-level OCR-like noise (~2% CER) over the whole document_text;
                         gold labels unchanged; evidence offsets remapped; adds field `cer`.
  sample40_seeded.jsonl  one minimal, meaning-changing edit per item (or none); adds field
                         `seed_edit` (null when the item was left unchanged); gold label and
                         evidence offsets updated.
  seeded_edits.txt       every seeded edit, old -> new, with 80 chars of context each side.

Seeded edits come from a small generic rule set applied to the gold evidence spans, plus
item-specific OVERRIDES (hand-written after reading seeded_edits.txt) where the generic rule did
not give a clean flip. Every override is listed in OVERRIDES / SKIP / NM_INSERTIONS below so the
output is reproducible. No human has reviewed these edits.
"""
import json
import random
import re
import sys
from pathlib import Path

SEED = 20261014
HERE = Path(__file__).resolve().parent
DATA = HERE / "data" / "contractnli"
SRC = DATA / "sample40.jsonl"
FLIP = {"Entailment": "Contradiction", "Contradiction": "Entailment"}

# ---------------------------------------------------------------- OCR noise
# One-character confusions (both directions where OCR confuses both ways).
SUB1 = {"l": "1I", "1": "lI", "I": "l1", "O": "0", "0": "O", "e": "c", "c": "e",
        "S": "5", "5": "S", "B": "8", "8": "B"}
# Multi-character confusions: (source, target).
MULTI = [("rn", "m"), ("m", "rn"), ("cl", "d"), ("d", "cl")]
# Edit cost (Levenshtein) of each op type, used to hit the CER budget and to report CER.
TARGET_CER_MEAN, TARGET_CER_SD = 0.02, 0.004
# Share of the edit budget per op family.
FAMILY_SHARE = {"sub": 0.70, "multi": 0.10, "space": 0.12, "hyphen": 0.08}


def ocr_sites(text):
    """All candidate noise sites as (family, start, end, replacement, cost)."""
    sites = {k: [] for k in FAMILY_SHARE}
    for i, ch in enumerate(text):
        if ch in SUB1:
            for r in SUB1[ch]:
                sites["sub"].append((i, i + 1, r, 1))
        if ch == " ":
            if 0 < i < len(text) - 1 and text[i - 1].isalnum() and text[i + 1].isalnum():
                sites["space"].append((i, i + 1, "", 1))      # dropped space: "the party" -> "theparty"
            sites["space"].append((i, i + 1, "  ", 1))        # doubled space
    for src, dst in MULTI:
        for m in re.finditer(re.escape(src), text):
            # only inside words, so "rn" -> "m" reads like a glyph merge
            sites["multi"].append((m.start(), m.end(), dst, max(len(src), len(dst))))
    for m in re.finditer(r"[A-Za-z]{9,}", text):
        w, cut = m.group(), None
        cut = len(w) // 2
        sites["hyphen"].append((m.start() + cut, m.start() + cut, "-\n", 2))  # "confi-\ndential"
    return sites


def apply_ocr(text, rng):
    rate = min(0.03, max(0.01, rng.gauss(TARGET_CER_MEAN, TARGET_CER_SD)))
    budget = round(rate * len(text))
    sites = ocr_sites(text)
    used = bytearray(len(text) + 2)          # positions already touched (with 1-char guard)
    chosen, spent = [], 0
    for fam, share in FAMILY_SHARE.items():
        fam_budget, fam_spent = round(budget * share), 0
        cand = sites[fam][:]
        rng.shuffle(cand)
        for s, e, r, c in cand:
            if fam_spent >= fam_budget:
                break
            lo, hi = max(0, s - 1), min(len(text), max(e, s + 1) + 1)
            if any(used[lo:hi]):
                continue
            for k in range(lo, hi):
                used[k] = 1
            chosen.append((s, e, r, c, fam))
            fam_spent += c
        spent += fam_spent
    chosen.sort()
    out, mapping, pos = [], [0] * (len(text) + 1), 0
    newlen = 0
    for s, e, r, c, fam in chosen:
        for k in range(pos, s):
            mapping[k] = newlen
            out.append(text[k]); newlen += 1
        for k in range(s, e):
            mapping[k] = newlen               # every source char of the op maps to op start
        out.append(r); newlen += len(r)
        pos = e
    for k in range(pos, len(text)):
        mapping[k] = newlen
        out.append(text[k]); newlen += 1
    mapping[len(text)] = newlen
    counts = {}
    for *_, fam in chosen:
        counts[fam] = counts.get(fam, 0) + 1
    return "".join(out), mapping, spent / len(text), counts


def make_ocr(items):
    out = []
    for it in items:
        rng = random.Random(f"{SEED}|ocr|{it['item_id']}")
        new, mp, cer, counts = apply_ocr(it["document_text"], rng)
        r = dict(it)
        r["document_text"] = new
        r["gold_evidence_spans"] = [[mp[s], mp[e], new[mp[s]:mp[e]]] for s, e, _ in it["gold_evidence_spans"]]
        r["cer"] = round(cer, 5)
        r["ocr_ops"] = counts
        out.append(r)
    return out


# ---------------------------------------------------------------- seeded edits
# Generic rules, tried in order on each gold evidence span (first match wins).
# (kind, regex, replacement, label_effect) ; label_effect "flip" or "keep".
GENERIC = [
    ("negation_delete", r"\bmay not\b", "may", "flip"),
    ("negation_delete", r"\bshall not\b", "shall", "flip"),
    ("negation_delete", r"\bwill not\b", "will", "flip"),
    ("exception_change", r"\bregardless of whether\b", "provided that", "flip"),
    ("modal_change", r"\bshall continue to\b", "shall cease to", "flip"),
    ("negation_insert", r"\bmay be\b", "may not be", "flip"),
    ("negation_insert", r"\bmay (?!not\b)", "may not ", "flip"),
    ("negation_insert", r"\bshall (also )?include\b", "shall not include", "flip"),
]
NUM_WORDS = {"one": 1, "two": 2, "three": 3, "four": 4, "five": 5, "six": 6, "seven": 7, "eight": 8,
             "nine": 9, "ten": 10, "twelve": 12, "fifteen": 15, "thirty": 30, "sixty": 60, "ninety": 90}
NUM_SWAP = {1: 3, 2: 5, 3: 6, 4: 7, 5: 2, 6: 3, 7: 4, 8: 4, 9: 4, 10: 4, 12: 24, 15: 45, 30: 90, 60: 10, 90: 30}
WORD_OF = {v: k for k, v in NUM_WORDS.items()}
WORD_OF.update({24: "twenty-four", 45: "forty-five"})
NUM_RE = re.compile(r"\b(" + "|".join(NUM_WORDS) + r") \((\d+)\) (days?|months?|years?)\b", re.I)
NUM_BARE = re.compile(r"\b(" + "|".join(NUM_WORDS) + r") (days?|months?|years?)\b", re.I)

# Items where the generic rule fires but does not give a clean flip, and no hand rule was
# written: left unchanged (seed_edit = null). Reason recorded in the output file.
SKIP = {
    "test-01": "definition lists business info in two spans ('which may include', 'may include human resources data'); one edit cannot make it technical-only; generic 'may not include' would not yield Entailment",
    "test-04": "long definition listing business, financial and marketing items; no single minimal edit makes it technical-only",
    "test-05": "long definition listing financial, sales, marketing items; no single minimal edit makes it technical-only",
    "test-09": "definition by reference ('all such information'); no single minimal edit makes it technical-only",
    "test-16": "three independent spans permit disclosure (Representatives, lenders and investors); one edit cannot flip",
    "test-20": "two independent spans (1.1.2 oral information, 1.2 'orally'); one edit cannot flip",
    "test-26": "definition lists marketing materials and finances alongside technical items; no clean single edit",
    "test-30": "definition lists business, financial, customer plans; no clean single edit",
    "test-37": "5.2 exceptions and 5.3 'retained ... pursuant to clause 5.2' both imply retention; one edit leaves the other",
}

# Hand-written edits for E/C items, used instead of the generic rule:
# item_id -> (kind, exact original substring inside a gold span, replacement, label_effect, note)
OVERRIDES = {
    "test-06": ("exception_change", "technologies and business topics", "technologies but not business topics", "flip",
                "Invention restricted to technical topics; CI is defined as information with respect to the Invention"),
    "test-10": ("number_change", "twelve (12) months", "twenty-four (24) months", "keep",
                "both spans independently prohibit solicitation, so no negation flips the label; the duration fact changes"),
    "test-11": ("scope_restriction", "shall mean all trade secrets and confidential or proprietary information",
                "shall mean only technical trade secrets and other technical information", "flip",
                "definition narrowed to technical information"),
    "test-12": ("negation_insert", "must seek the highest level of protection available and, when possible, give the other enough prior notice",
                "must seek the highest level of protection available but is not required to give the other any prior notice", "flip",
                "generic rule would hit 'may disclose' in span 1, which does not remove the notice duty"),
    "test-13": ("number_change", "thirty (30) days", "ninety (90) days", "keep",
                "'only if ... designated' cannot be negated without leaving (i)/(ii) dangling; the confirmation window changes"),
    "test-15": ("negation_delete", "shall not give Recipient or its Representatives any licence",
                "shall give Recipient or its Representatives a licence", "flip",
                "generic 'shall not'->'shall' alone leaves 'any licence', which reads poorly"),
    "test-17": ("number_change", "five years", "two years", "keep",
                "removing the third-party exclusion would make the hypothesis NotMentioned rather than Contradiction"),
    "test-18": ("exception_change", "regardless of the manner in which it is so furnished",
                "provided that it is expressly identified as confidential when so furnished", "flip",
                "only identified information is protected"),
    "test-19": ("negation_insert", "Information shall also include the identity", "Information shall not include the identity", "flip",
                "the fact of the agreement is carved out of Information"),
    "test-25": ("exception_change",
                "which at the time of disclosure is designated as confidential (or like designation), is disclosed in circumstances of confidence, or would be understood by the parties, exercising reasonable business judgment, to be confidential",
                "but only if at the time of disclosure it is designated as confidential (or like designation)", "flip",
                "circumstances-of-confidence and reasonable-judgment limbs removed"),
    "test-27": ("scope_restriction", "shall include all information or material that has",
                "shall include only technical information or material that has", "flip",
                "definition narrowed to technical information"),
    "test-29": ("scope_restriction", "is that information described in", "is only that technical information described in", "flip",
                "definition narrowed to technical information"),
    "test-36": ("exception_change", "whether in writing, in electronic form or pursuant to discussions",
                "whether in writing or in electronic form (but excluding any information conveyed orally or pursuant to discussions)", "flip",
                "verbal information excluded"),
    "test-39": ("negation_delete", "shall not be required to keep the Information confidential to the extent",
                "shall be required to keep the Information confidential even to the extent", "flip",
                "the carve-out list, including (c) independently developed, becomes a list of still-confidential information"),
}

# NotMentioned items: insert one sentence after the sentence containing `anchor` (first match).
NM_INSERTIONS = {
    "test-00": ("shall cause its Representatives not to, use any Proprietary Information",
                "The Receiving Party shall not reverse engineer, decompile or disassemble any prototypes, software, samples or other objects which embody the Disclosing Party’s Proprietary Information."),
    # review fix: the first anchor ('reveal, divulge, disclose ...') hit 'Inc.'/'T.I.P.' mid-sentence;
    # anchor now on the last clause of (b), so the sentence lands at the end of paragraph (b).
    "test-02": ("the Undersigned can demonstrate that such information was already known",
                "Notwithstanding the foregoing, Samples may make copies of such confidential information to the extent reasonably necessary to perform his obligations under the Merger Agreement."),
    "test-03": ("shall be returned to it or destroyed promptly at its request",
                "The Receiving Party may make copies of the Confidential Information only to the extent reasonably necessary for the Scope."),
    "test-08": ("The parties agree to protect each other's Conﬁdential Information",
                "For the purposes of this Agreement, Conﬁdential Information includes information disclosed in writing or orally."),
    "test-14": ("7. Upon providing the Receiving Party with a specific list",
                "The Receiving Party may make copies of the Confidential Information solely to the extent necessary to evaluate a Transaction."),
    "test-21": ("At the request of Disclosing Party, Recipient shall return",
                "Upon the termination of this Agreement, Recipient shall promptly return to Disclosing Party or destroy all Confidential Information of Disclosing Party."),
    "test-22": ("6. The Contractor shall, at its own expense, return",
                "The Contractor’s obligations under this Agreement shall survive the termination of this Agreement and of the Contract."),
    "test-23": ("Confidential Information includes, by way of example only",
                "Confidential Information includes information disclosed to the Contractor orally as well as in writing."),
    "test-28": ("4. The obligations and limitations set forth herein regarding Confidential Information shall not apply",
                "Nothing in this Agreement shall prevent Recipient from independently developing information similar to Disclosing Party’s Confidential Information without use of such Confidential Information."),
    "test-31": ("I agree that I shall return all information to which I have had access",
                "I may make copies of such information only to the extent required to perform my official duties and in accordance with the applicable handling requirements."),
    "test-32": ("does not include information which is or becomes generally available to the public",
                "Nothing in this Agreement shall prevent the Seller Parties from independently developing information similar to the Confidential Information without use of the Confidential Information."),
    "test-35": ("(c) shall not use Confidential Information for any purpose other than in connection with the Evaluation",
                "The Receiving Party shall not reverse engineer, decompile or disassemble any prototypes, software, samples or other objects which embody the Disclosing Party’s Confidential Information."),
    "test-38": ("Receiving Party shall return all information to the Disclosing Party without retaining any copies",
                "The obligations of the Receiving Party under this Agreement shall survive the termination of this Agreement."),
}


ABBREV = {"inc", "ltd", "co", "corp", "no", "e.g", "i.e", "u.s", "mr", "ms", "dr", "st", "sec", "art"}


def sentence_end(text, start):
    """Index just after the first sentence-final period at/after start.

    Review fix (seeded_edits.txt, first pass): a period counts only if the next non-space
    character is not lower-case (test-35 had '...Evaluation. and (d) ...') and the word before it
    is not an abbreviation (test-02 had 'Innco Hospitality, Inc. and any entity ...')."""
    for m in re.finditer(r"\.(?=\s|$)", text[start:]):
        p = start + m.end()
        nxt = text[p:].lstrip()[:1]
        word = re.search(r"([A-Za-z.]+)\.$", text[max(0, p - 12):p])
        if nxt and nxt.islower():
            continue
        if word and word.group(1).lower() in ABBREV:
            continue
        return p
    raise ValueError("no sentence end")


def generic_edit(spans):
    for kind, pat, rep, eff in GENERIC:
        for si, (s, e, t) in enumerate(spans):
            m = re.search(pat, t)
            if m:
                return kind, s + m.start(), m.group(), rep, eff, si
    for si, (s, e, t) in enumerate(spans):
        m = NUM_RE.search(t)
        if m:
            n = int(m.group(2)); n2 = NUM_SWAP.get(n, n + 3)
            rep = f"{WORD_OF.get(n2, str(n2))} ({n2}) {m.group(3)}"
            return "number_change", s + m.start(), m.group(), rep, "keep", si
    return None


def shift_spans(spans, off, old, new):
    d, end = len(new) - len(old), off + len(old)
    out = []
    for s, e, t in spans:
        if e <= off:
            out.append([s, e])
        elif s >= end:
            out.append([s + d, e + d])
        else:
            assert s <= off and end <= e, "edit crosses a span boundary"
            out.append([s, e + d])
    return out


def make_seeded(items):
    out, log = [], []
    for it in items:
        r = dict(it)
        doc, iid, gold = it["document_text"], it["item_id"], it["gold_label"]
        spans = it["gold_evidence_spans"]
        edit, rule, note = None, None, ""
        if gold == "NotMentioned":
            anchor, sent = NM_INSERTIONS[iid]
            a = doc.index(anchor)
            p = sentence_end(doc, a + len(anchor))
            new_text = " " + sent
            edit = ("sentence_insert", p, "", new_text, None, None)
            rule, note = "nm_table", f"inserted after sentence containing: {anchor!r}"
        elif iid in SKIP:
            g = generic_edit(spans)
            rule, note = "skip", SKIP[iid] + (f" [generic would have done {g[2]!r}->{g[3]!r}]" if g else "")
        elif iid in OVERRIDES:
            kind, old, new, eff, note = OVERRIDES[iid]
            for si, (s, e, t) in enumerate(spans):
                if old in t:
                    assert t.count(old) == 1, (iid, old)
                    edit = (kind, s + t.index(old), old, new, eff, si)
                    break
            else:
                raise AssertionError(f"{iid}: override text not in any gold span")
            g = generic_edit(spans)
            rule = "override"
            if g:
                note += f" [generic would have done {g[2]!r}->{g[3]!r}]"
        else:
            g = generic_edit(spans)
            if g:
                edit, rule = g, "generic"
        if edit is None:
            r["seed_edit"] = None
            log.append((iid, gold, rule or "none", note, None))
            out.append(r)
            continue
        kind, off, old, new, eff, si = edit
        assert doc[off:off + len(old)] == old
        new_doc = doc[:off] + new + doc[off + len(old):]
        if gold == "NotMentioned":
            new_gold = "Entailment"
            ns = off + 1
            new_spans = [[ns, ns + len(new) - 1, new_doc[ns:ns + len(new) - 1]]]
            # original NM items have no spans; the inserted sentence becomes the evidence
        else:
            new_gold = FLIP[gold] if eff == "flip" else gold
            new_spans = [[s, e, new_doc[s:e]] for s, e in shift_spans(spans, off, old, new)]
        r["document_text"] = new_doc
        r["gold_label"] = new_gold
        r["gold_evidence_spans"] = new_spans
        if gold == "NotMentioned":
            r["gold_evidence_span_ids"] = []  # inserted sentence has no original span id
        r["seed_edit"] = {"kind": kind, "original_text": old, "new_text": new, "char_offset": off,
                          "new_gold_label": new_gold, "old_gold_label": gold, "rule": rule,
                          "span_index": si, "note": note}
        if kind == "number_change":
            r["seed_edit"]["new_fact"] = f"document now says {new!r} where the original said {old!r}"
        log.append((iid, gold, rule, note, (doc, new_doc, off, old, new, new_gold, kind)))
        out.append(r)
    return out, log


def write_log(log, path):
    lines = []
    for iid, gold, rule, note, ed in log:
        lines.append(f"=== {iid}  gold={gold}  rule={rule}")
        if ed is None:
            lines.append(f"    UNCHANGED (seed_edit null). {note}")
        else:
            doc, new_doc, off, old, new, new_gold, kind = ed
            pre = doc[max(0, off - 80):off].replace("\n", " / ")
            post = doc[off + len(old):off + len(old) + 80].replace("\n", " / ")
            lines.append(f"    kind={kind}  new_gold={new_gold}  offset={off}")
            if note:
                lines.append(f"    note: {note}")
            lines.append(f"    OLD: ...{pre}[[{old}]]{post}...")
            lines.append(f"    NEW: ...{pre}[[{new}]]{post}...")
        lines.append("")
    path.write_text("\n".join(lines), encoding="utf-8")


def dump(rows, path):
    with open(path, "w", encoding="utf-8") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")


def main():
    items = [json.loads(l) for l in SRC.read_text(encoding="utf-8").splitlines() if l.strip()]
    ocr = make_ocr(items)
    dump(ocr, DATA / "sample40_ocr.jsonl")
    seeded, log = make_seeded(items)
    dump(seeded, DATA / "sample40_seeded.jsonl")
    write_log(log, DATA / "seeded_edits.txt")
    cers = sorted(r["cer"] for r in ocr)
    kinds = {}
    for r in seeded:
        k = r["seed_edit"]["kind"] if r["seed_edit"] else "unchanged"
        kinds[k] = kinds.get(k, 0) + 1
    rules = {}
    for _, _, rule, _, _ in log:
        rules[rule] = rules.get(rule, 0) + 1
    print(f"OCR: n={len(cers)} mean CER={sum(cers)/len(cers):.4f} min={cers[0]:.4f} "
          f"median={cers[len(cers)//2]:.4f} max={cers[-1]:.4f}")
    print("seeded kinds:", dict(sorted(kinds.items())))
    print("seeded rules:", dict(sorted(rules.items())))


if __name__ == "__main__":
    sys.exit(main())
