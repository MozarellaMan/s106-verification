#!/usr/bin/env python3
"""Find the legal agreements for a list of planning applications on an Idox Public Access register.

For each reference in a sample CSV (column `lpa_app_no`), this searches the register, opens the
application's Documents tab, and records every document listed there. Documents whose type or
description looks like a legal agreement (S106, unilateral undertaking, deed of variation) are
marked as candidates. With --download it also fetches the candidates and records their SHA-256.

Outputs, in --out (default: the sample's directory):
  documents.csv   every document listed for every application (the full listing, for audit)
  retrieval.csv   one row per application: found / no agreement listed / not found / error, and why
  corpus.tsv      downloaded candidates, in fixtures/corpus.tsv's format (id, path, sha256, url, session)

Council registers usually refuse requests from cloud machines, so run this on your own computer:
  python3 scripts/idox-deeds.py <sample.csv> [--base URL] [--download DIR] [--only REF ...]

Southwark's server omits its Sectigo intermediate certificate. If you see CERTIFICATE_VERIFY_FAILED,
download http://crt.sectigo.com/SectigoPublicServerAuthenticationCAOVR36.crt, convert it with
`openssl x509 -inform DER -in X.crt -out X.pem`, append it to a copy of your CA bundle and pass that
file with --cafile. Standard library only; no third-party packages.
"""

import argparse
import csv
import hashlib
import html
import http.cookiejar
import os
import re
import ssl
import sys
import time
import urllib.parse
import urllib.request
from html.parser import HTMLParser

DEFAULT_BASE = "https://planning.southwark.gov.uk/online-applications"
UA = "s106-verification research (deed discovery; contact via github.com/MozarellaMan/s106-verification)"

# A document is a candidate when its type or description matches this. Ordered so the label says
# what kind of instrument it most likely is.
KINDS = [
    ("variation", re.compile(r"(?i)deed of variation|\bDoV\b|variation (agreement|deed)|s\.?106A|section 106A")),
    ("unilateral", re.compile(r"(?i)unilateral undertaking|\bUU\b")),
    ("s106", re.compile(r"(?i)s\.?\s?106|section 106|legal agreement|planning obligation|\bagreement\b")),
]
# Drafts, heads of terms and correspondence about an agreement are not the executed deed.
NOT_DEED = re.compile(r"(?i)\bdraft\b|heads of terms|\bHoTs?\b|letter|e-?mail|correspondence|report|"
                      r"statement|viability|consultation|comments?|memo|monitoring|invoice|discharge")


class Session:
    def __init__(self, base, cafile, delay):
        self.base = base.rstrip("/")
        self.delay = delay
        ctx = ssl.create_default_context(cafile=cafile) if cafile else ssl.create_default_context()
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(self.jar), urllib.request.HTTPSHandler(context=ctx))
        self.last = 0.0

    def open(self, url, data=None):
        """Return (final_url, body bytes). Spaced out by --delay so the register isn't hammered."""
        wait = self.delay - (time.monotonic() - self.last)
        if wait > 0:
            time.sleep(wait)
        if not url.startswith("http"):
            url = urllib.parse.urljoin(self.base + "/", url)
        body = urllib.parse.urlencode(data).encode() if data is not None else None
        req = urllib.request.Request(url, data=body, headers={"User-Agent": UA})
        with self.opener.open(req, timeout=90) as resp:
            out = (resp.geturl(), resp.read())
        self.last = time.monotonic()
        return out


def text(b):
    return b.decode("utf-8", "replace")


def find_key_val(sess, ref):
    """Search by reference; return (keyVal, note). Idox redirects straight to the application when
    exactly one matches, otherwise lists results."""
    url, page = sess.open("search.do?action=simple&searchType=Application")
    m = re.search(r'name="_csrf"\s+value="([^"]+)"', text(page))
    form = {"searchType": "Application", "searchCriteria.simpleSearchString": ref,
            "searchCriteria.simpleSearch": "true"}
    if m:
        form["_csrf"] = m.group(1)
    url, page = sess.open("simpleSearchResults.do?action=firstPage", form)
    m = re.search(r"keyVal=([A-Z0-9]+)", url)
    if m:
        return m.group(1), ""
    body = text(page)
    # Results list: keep entries whose shown reference is exactly ours.
    hits = []
    for li in re.findall(r'(?s)<li class="searchresult">.*?</li>', body):
        k = re.search(r"keyVal=([A-Z0-9]+)", li)
        if k and re.search(re.escape(ref) + r"(?![0-9])", html.unescape(re.sub(r"<[^>]+>", " ", li))):
            hits.append(k.group(1))
    hits = list(dict.fromkeys(hits))
    if len(hits) == 1:
        return hits[0], ""
    if not hits:
        if re.search(r"(?i)no results found", body):
            return None, "no results"
        return None, "search returned no recognisable results"
    return None, f"{len(hits)} results"


class TableParser(HTMLParser):
    """Collect the Documents table: header cells, and per row its cell texts and file links."""

    def __init__(self):
        super().__init__()
        self.in_table = 0
        self.headers, self.rows = [], []
        self.row = None
        self.cell = None
        self.links = None

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag == "table" and (a.get("id") == "Documents" or self.in_table):
            self.in_table += 1
        if not self.in_table:
            return
        if tag == "tr":
            self.row, self.links = [], []
        elif tag in ("td", "th") and self.row is not None:
            self.cell = []
        elif tag == "a" and self.links is not None and a.get("href"):
            if "/files/" in a["href"] or "viewDocument" in a["href"]:
                self.links.append(a["href"])

    def handle_endtag(self, tag):
        if not self.in_table:
            return
        if tag in ("td", "th") and self.cell is not None and self.row is not None:
            self.row.append((tag, " ".join("".join(self.cell).split())))
            self.cell = None
        elif tag == "tr" and self.row is not None:
            if self.row and all(t == "th" for t, _ in self.row):
                self.headers = [c for _, c in self.row]
            elif self.row:
                self.rows.append(([c for _, c in self.row], self.links))
            self.row = self.links = None
        elif tag == "table":
            self.in_table -= 1

    def handle_data(self, data):
        if self.cell is not None:
            self.cell.append(data)


def list_documents(sess, key):
    url = f"applicationDetails.do?activeTab=documents&keyVal={key}"
    _, page = sess.open(url)
    p = TableParser()
    p.feed(text(page))
    docs = []
    for cells, links in p.rows:
        if not links:
            continue
        named = dict(zip((h.lower() for h in p.headers), cells)) if p.headers else {}
        docs.append({
            "date_published": named.get("date published", ""),
            "document_type": named.get("document type", ""),
            "description": named.get("description", "") or " ".join(cells),
            "url": urllib.parse.urljoin(sess.base + "/", links[0]),
        })
    return sess.base + "/" + url, docs


def classify(doc):
    hay = f"{doc['document_type']} {doc['description']} {urllib.parse.unquote(doc['url'].rsplit('/', 1)[-1])}"
    for kind, rx in KINDS:
        if rx.search(hay):
            return kind, bool(NOT_DEED.search(hay))
    return "", False


def slug(ref):
    return re.sub(r"[^A-Za-z0-9]+", "_", ref).strip("_")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("sample", help="CSV with an lpa_app_no column")
    ap.add_argument("--base", default=DEFAULT_BASE, help="Idox online-applications root")
    ap.add_argument("--prefix", default="southwark", help="id prefix for corpus.tsv rows")
    ap.add_argument("--out", help="where to write the CSVs (default: the sample's directory)")
    ap.add_argument("--download", metavar="DIR", help="also download candidate deeds into DIR")
    ap.add_argument("--cafile", help="CA bundle to trust (for registers missing an intermediate)")
    ap.add_argument("--delay", type=float, default=2.0, help="seconds between requests (default 2)")
    ap.add_argument("--only", nargs="*", default=[], help="limit to these references")
    a = ap.parse_args()

    out = a.out or os.path.dirname(os.path.abspath(a.sample))
    os.makedirs(out, exist_ok=True)
    refs = [r["lpa_app_no"] for r in csv.DictReader(open(a.sample, newline=""))]
    if a.only:
        refs = [r for r in refs if r in a.only]
    sess = Session(a.base, a.cafile, a.delay)
    stamp = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())

    doc_rows, ret_rows, corpus_rows = [], [], []
    for ref in refs:
        row = {"lpa_app_no": ref, "retrieved_at": stamp, "key_val": "", "documents_page": "",
               "n_documents": 0, "n_candidates": 0, "status": "", "reason": ""}
        try:
            key, note = find_key_val(sess, ref)
            if not key:
                row.update(status="not found", reason=note)
            else:
                row["key_val"] = key
                page, docs = list_documents(sess, key)
                row["documents_page"] = page
                row["n_documents"] = len(docs)
                for d in docs:
                    kind, maybe_not = classify(d)
                    d.update(lpa_app_no=ref, kind=kind, candidate=bool(kind) and not maybe_not,
                             sha256="", path="")
                    doc_rows.append(d)
                cands = [d for d in docs if d["candidate"]]
                row["n_candidates"] = len(cands)
                if not docs:
                    row.update(status="no documents", reason="Documents tab empty or unreadable")
                elif not cands:
                    row.update(status="no agreement listed",
                               reason="no document type/description matched an agreement")
                else:
                    row["status"] = "found"
                    row["reason"] = ", ".join(sorted({d["kind"] for d in cands}))
                if a.download:
                    os.makedirs(a.download, exist_ok=True)
                    for i, d in enumerate(cands, 1):
                        name = f"{a.prefix}-{slug(ref)}-{i}.pdf"
                        path = os.path.join(a.download, name)
                        _, body = sess.open(d["url"])
                        if not body.startswith(b"%PDF"):
                            d["path"] = "(not a PDF)"
                            continue
                        with open(path, "wb") as f:
                            f.write(body)
                        d["sha256"] = hashlib.sha256(body).hexdigest()
                        d["path"] = path
                        corpus_rows.append([f"{a.prefix}-{slug(ref)}-{i}", path, d["sha256"], d["url"], page])
        except Exception as e:  # noqa: BLE001 - every failure is recorded, not fatal
            row.update(status="error", reason=f"{type(e).__name__}: {e}"[:300])
        ret_rows.append(row)
        print(f"{ref}\t{row['status']}\t{row['n_candidates']}/{row['n_documents']}\t{row['reason']}",
              file=sys.stderr)

    with open(os.path.join(out, "retrieval.csv"), "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(ret_rows[0]) if ret_rows else ["lpa_app_no"])
        w.writeheader()
        w.writerows(ret_rows)
    cols = ["lpa_app_no", "date_published", "document_type", "description", "kind", "candidate",
            "url", "sha256", "path"]
    with open(os.path.join(out, "documents.csv"), "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=cols, extrasaction="ignore")
        w.writeheader()
        w.writerows(doc_rows)
    if corpus_rows:
        with open(os.path.join(out, "corpus.tsv"), "w", newline="") as f:
            f.write("id\tpath\tsha256\turl\tsession\n")
            for r in corpus_rows:
                f.write("\t".join(r) + "\n")
    found = sum(r["status"] == "found" for r in ret_rows)
    print(f"{found}/{len(ret_rows)} applications list an agreement; see {out}/retrieval.csv", file=sys.stderr)


if __name__ == "__main__":
    main()
