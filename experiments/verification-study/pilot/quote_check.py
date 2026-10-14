"""Deterministic quote check: is each quote in the document, and does it overlap the gold evidence?

A quote is `exact` when it appears in the document after normalisation (whitespace runs -> one
space; curly quotes -> straight; en/em/figure dashes and minus -> '-'; ligatures -> letters; '…' -> '...'), `near` when some window of
the document is within Levenshtein distance 3 of it, else `missing`. Offsets are mapped back to the
original document so they can be compared with the gold evidence spans.
"""
MAX_EDITS = 3
TRANS = str.maketrans({"‘": "'", "’": "'", "‚": "'", "‛": "'", "′": "'",
                       "“": '"', "”": '"', "„": '"', "″": '"',
                       "‐": "-", "‑": "-", "‒": "-", "–": "-", "—": "-",
                       "―": "-", "−": "-", " ": " ",
                       # one character to several: PDF ligatures and the ellipsis character occur in
                       # the ContractNLI documents, and a model copying them writes the plain letters
                       "ﬀ": "ff", "ﬁ": "fi", "ﬂ": "fl", "ﬃ": "ffi", "ﬄ": "ffl", "…": "..."})


def normalise(text):
    """The normalised text and, for each of its characters, the index in `text` it came from."""
    out, where = [], []
    for i, orig in enumerate(text):
        for ch in orig.translate(TRANS):  # per character, so `where` stays on the original offsets
            if ch.isspace():
                if not out or out[-1] == " ":
                    continue
                ch = " "
            out.append(ch)
            where.append(i)
    if out and out[-1] == " ":
        out.pop(), where.pop()
    return "".join(out), where


def best_window(q, t):
    """(distance, start, end) of the substring of t closest to q (free start and end in t)."""
    prev = [(0, j) for j in range(len(t) + 1)]  # (cost, start) per end position
    for i in range(1, len(q) + 1):
        cur = [(i, 0)]
        for j in range(1, len(t) + 1):
            sub = prev[j - 1][0] + (q[i - 1] != t[j - 1])
            cur.append(min((sub, prev[j - 1][1]), (prev[j][0] + 1, prev[j][1]),
                           (cur[j - 1][0] + 1, cur[j - 1][1])))
        prev = cur
    end = min(range(len(t) + 1), key=lambda j: prev[j][0])
    return prev[end][0], prev[end][1], end


def near(q, doc):
    """Best window within MAX_EDITS, or None. With at most 3 edits, one of 4 disjoint chunks of the
    quote survives intact, so only the neighbourhoods of exact chunk hits need a full comparison."""
    m, k = len(q), MAX_EDITS + 1
    if m < 4 * k:
        return None if m <= MAX_EDITS else _scan(q, doc, [], whole=True)
    size = m // k
    anchors = set()
    for c in range(k):
        chunk = q[c * size:(c + 1) * size]
        pos = doc.find(chunk)
        while pos != -1:
            anchors.add(pos - c * size)
            pos = doc.find(chunk, pos + 1)
    return _scan(q, doc, sorted(anchors))


def _scan(q, doc, anchors, whole=False):
    best = None
    spans = [(0, len(doc))] if whole else [(max(0, a - MAX_EDITS), a + len(q) + MAX_EDITS) for a in anchors]
    for lo, hi in spans:
        d, s, e = best_window(q, doc[lo:hi])
        if d <= MAX_EDITS and (best is None or d < best[0]):
            best = (d, lo + s, lo + e)
    return best


def check_quote(quote, doc_norm, where):
    q, _ = normalise(quote)
    if not q:
        return {"quote": quote, "status": "missing", "dist": None, "start": None, "end": None}
    pos = doc_norm.find(q)
    if pos != -1:
        status, dist, s, e = "exact", 0, pos, pos + len(q)
    else:
        hit = near(q, doc_norm)
        if hit is None:
            return {"quote": quote, "status": "missing", "dist": None, "start": None, "end": None}
        status, (dist, s, e) = "near", hit
    return {"quote": quote, "status": status, "dist": dist, "start": where[s], "end": where[e - 1] + 1}


def overlap(checked, gold_spans):
    """Character-level precision and recall of the located quotes against the gold spans."""
    quoted = set()
    for c in checked:
        if c["start"] is not None:
            quoted.update(range(c["start"], c["end"]))
    gold = set()
    for s, e, *_ in gold_spans:
        gold.update(range(s, e))
    both = len(quoted & gold)
    return (both / len(quoted) if quoted else 0.0), (both / len(gold) if gold else 0.0)


def check_reading(doc, label, quotes, gold_label, gold_spans):
    """status: pass / fail / n/a (NotMentioned with no quotes). A labelled reading with no quote fails."""
    doc_norm, where = normalise(doc)
    checked = [check_quote(q, doc_norm, where) for q in quotes]
    if label == "NotMentioned" and not quotes:
        status = "n/a"
    elif not quotes or any(c["status"] != "exact" for c in checked):
        status = "fail"
    else:
        status = "pass"
    out = {"status": status, "quotes": checked, "precision": None, "recall": None}
    if gold_label != "NotMentioned" and label != "NotMentioned":
        out["precision"], out["recall"] = overlap(checked, gold_spans)
    return out


if __name__ == "__main__":
    doc = "The Receiving Party shall not\n  disclose “Confidential Information” — ever."
    for q in ['shall not disclose "Confidential Information" - ever.', "shall nut disclose",
              "shall not disclose Confidental Informaton", "reverse engineer"]:
        print(check_reading(doc, "Entailment", [q], "Entailment", [[4, 40, ""]]))
