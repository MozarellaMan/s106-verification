#!/usr/bin/env python3
"""Find the legal agreements for a list of planning applications on an Idox Public Access register.

For each reference in a sample CSV (column `lpa_app_no`), this searches the register, opens the
application's Documents tab, and records every document listed there. Documents whose type or
description looks like a legal agreement (S106, unilateral undertaking, deed of variation) are
marked as candidates. With --download it also fetches the candidates into the data checkout's
raw/ folder and records their SHA-256.

Registers rate-limit scrapers. The script waits --delay seconds between requests and stops at the
first sign of a block (HTTP 429/403, or a page it can't recognise), saving that page next to the
CSVs and marking the remaining applications "not attempted". Re-run with --only for those later.

Outputs, in --out (default: the sample's directory). Each run appends; nothing is overwritten, so
a re-run with --only adds new attempts and keeps the earlier ones (read the latest `retrieved_at`
per application for its current state):
  documents.csv   every document listed for every application (the full listing, for audit)
  retrieval.csv   one row per application per attempt, with an outcome and why:
                    found                   agreement listed (and downloaded, with --download)
                    found, not downloaded   agreement listed but a download failed or was blocked
                    no agreement listed     documents listed, none looks like an agreement
                    no documents            the register says the application has no documents
                    not found               the search found no such application
                    error                   anything else, including a page that couldn't be read
                    not attempted           the register blocked the run before this application
  corpus.tsv      downloaded candidates as corpus/manifest.tsv rows (id, path, sha256, url, session),
                  paths relative to the data checkout

Council registers usually refuse requests from cloud machines, so run this on your own computer:
  python3 scripts/idox-deeds.py <sample.csv> [--base URL] [--download] [--only REF ...]

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
import urllib.error
import urllib.parse
import urllib.request
from html.parser import HTMLParser

DEFAULT_BASE = "https://planning.southwark.gov.uk/online-applications"
class Blocked(Exception):
    """The register is refusing us; stop rather than keep knocking."""


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
                      r"statement|viability|consultation|comments?|memo|monitoring|invoice|discharge|"
                      r"enquiry|inqui?e?ry|confirmation|notice|decision")


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
        try:
            with self.opener.open(req, timeout=90) as resp:
                out = (resp.geturl(), resp.read())
        except urllib.error.HTTPError as e:
            if e.code in (403, 429):
                raise Blocked(f"HTTP {e.code} from {url}", e.read()) from e
            raise
        finally:
            self.last = time.monotonic()
        if re.search(rb"(?i)unusual traffic|too many requests", out[1][:20000]):
            raise Blocked(f"block page from {url}", out[1])
        return out


def text(b):
    return b.decode("utf-8", "replace")


def find_key_val(sess, ref):
    """Search by reference; return (keyVal, note). Idox redirects straight to the application when
    exactly one matches, otherwise lists results."""
    # The advanced search matches the reference field exactly; the simple search is full-text and
    # can return dozens of applications that merely mention the reference.
    url, page = sess.open("search.do?action=advanced&searchType=Application")
    m = re.search(r'name="_csrf"\s+value="([^"]+)"', text(page))
    form = {"searchType": "Application", "searchCriteria.reference": ref}
    if m:
        form["_csrf"] = m.group(1)
    url, page = sess.open("advancedSearchResults.do?action=firstPage", form)
    m = re.search(r"keyVal=([A-Z0-9]+)", url)
    if m:
        return m.group(1), ""
    body = text(page)
    # Southwark shows the application itself (no redirect) when exactly one matches.
    m = re.search(r"activeTab=summary&(?:amp;)?keyVal=([A-Z0-9]+)", body)
    if m and 'id="searchresults"' not in body:
        return m.group(1), ""
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
        raise Blocked(f"unrecognised search page for {ref}", page)
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
        self.saw_table = False

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag == "table" and (a.get("id") == "Documents" or self.in_table):
            self.in_table += 1
            self.saw_table = True
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
    if not p.saw_table:
        # An empty tab says so; anything else is a page we can't read, which is not the same as
        # an application with no documents.
        if re.search(r"(?i)no documents", text(page)):
            return sess.base + "/" + url, []
        raise ValueError("Documents table not found on the documents tab")
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


RETRIEVAL_COLS = ["lpa_app_no", "retrieved_at", "key_val", "documents_page", "n_documents",
                  "n_candidates", "status", "reason"]
DOCUMENT_COLS = ["lpa_app_no", "retrieved_at", "date_published", "document_type", "description",
                 "kind", "candidate", "url", "sha256", "path"]


def append_csv(path, cols, rows):
    """Append rows, writing the header only for a new file. Earlier runs are never rewritten."""
    new = not os.path.exists(path)
    if not new:
        with open(path, newline="") as f:
            if next(csv.reader(f), None) != cols:
                sys.exit(f"{path} has different columns; move it aside before appending")
    with open(path, "a", newline="") as f:
        w = csv.DictWriter(f, fieldnames=cols, extrasaction="ignore")
        if new:
            w.writeheader()
        w.writerows(rows)


def slug(ref):
    return re.sub(r"[^A-Za-z0-9]+", "_", ref).strip("_")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("sample", help="CSV with an lpa_app_no column")
    ap.add_argument("--base", default=DEFAULT_BASE, help="Idox online-applications root")
    ap.add_argument("--prefix", default="southwark", help="id prefix for corpus.tsv rows")
    ap.add_argument("--out", help="where to write the CSVs (default: the sample's directory)")
    ap.add_argument("--download", action="store_true",
                    help="also download candidate deeds into the data checkout's raw/")
    ap.add_argument("--data", default=os.environ.get("S106_DATA", "data"),
                    help="data checkout (default: $S106_DATA, else data/)")
    ap.add_argument("--cafile", help="CA bundle to trust (for registers missing an intermediate)")
    ap.add_argument("--delay", type=float, default=8.0, help="seconds between requests (default 8)")
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
                    row.update(status="no documents", reason="the register lists no documents")
                elif not cands:
                    row.update(status="no agreement listed",
                               reason="no document type/description matched an agreement")
                else:
                    row["status"] = "found"
                    row["reason"] = ", ".join(sorted({d["kind"] for d in cands}))
                if a.download and cands:
                    os.makedirs(os.path.join(a.data, "raw"), exist_ok=True)
                    failed = []
                    try:
                        for i, d in enumerate(cands, 1):
                            path = f"raw/{a.prefix}-{slug(ref)}-{i}.pdf"
                            try:
                                _, body = sess.open(d["url"])
                            except Blocked:
                                failed.append(f"{i}: blocked")
                                raise
                            except Exception as e:  # noqa: BLE001 - recorded per document
                                failed.append(f"{i}: {type(e).__name__}: {e}")
                                continue
                            if not body.startswith(b"%PDF"):
                                failed.append(f"{i}: not a PDF")
                                continue
                            with open(os.path.join(a.data, path), "wb") as f:
                                f.write(body)
                            d["sha256"] = hashlib.sha256(body).hexdigest()
                            d["path"] = path
                            corpus_rows.append([f"{a.prefix}-{slug(ref)}-{i}", path, d["sha256"], d["url"], page])
                    finally:
                        if failed:
                            row.update(status="found, not downloaded",
                                       reason=f"{len(failed)} of {len(cands)} failed: " + "; ".join(failed)[:250])
        except Blocked as e:
            msg, page = e.args
            with open(os.path.join(out, "blocked.html"), "wb") as f:
                f.write(page or b"")
            if row["status"] != "found, not downloaded":
                row.update(status="error", reason=f"blocked: {msg}")
            ret_rows.append(row)
            print(f"{ref}\tblocked\t{msg}; page saved to blocked.html; stopping", file=sys.stderr)
            for rest in refs[refs.index(ref) + 1:]:
                ret_rows.append({**row, "lpa_app_no": rest, "key_val": "", "documents_page": "",
                                 "n_documents": 0, "n_candidates": 0, "status": "not attempted",
                                 "reason": "register blocked earlier in this run"})
            break
        except Exception as e:  # noqa: BLE001 - every failure is recorded, not fatal
            row.update(status="error", reason=f"{type(e).__name__}: {e}"[:300])
        ret_rows.append(row)
        print(f"{ref}\t{row['status']}\t{row['n_candidates']}/{row['n_documents']}\t{row['reason']}",
              file=sys.stderr)

    append_csv(os.path.join(out, "retrieval.csv"), RETRIEVAL_COLS, ret_rows)
    for d in doc_rows:
        d["retrieved_at"] = stamp
    append_csv(os.path.join(out, "documents.csv"), DOCUMENT_COLS, doc_rows)
    corpus = os.path.join(out, "corpus.tsv")
    have = set()
    if os.path.exists(corpus):
        have = {line.split("\t", 1)[0] for line in open(corpus)}
    new = [r for r in corpus_rows if r[0] not in have]
    if new:
        with open(corpus, "a", newline="") as f:
            if not have:
                f.write("id\tpath\tsha256\turl\tsession\n")
            for r in new:
                f.write("\t".join(r) + "\n")
    found = sum(r["status"] == "found" for r in ret_rows)
    print(f"{found}/{len(ret_rows)} applications list an agreement; see {out}/retrieval.csv", file=sys.stderr)


if __name__ == "__main__":
    main()
