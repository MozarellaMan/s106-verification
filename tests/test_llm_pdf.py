"""Offline tests for the llm-pdf backend: fixture replay and the Opus cascade against a mock client.

    .venv/bin/python -m unittest discover -s tests

Needs the sample deed and its OCR cache (see README); skipped otherwise. No credentials needed.
"""
import copy
import json
import os
import subprocess
import sys
import unittest
from pathlib import Path
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "s106-audit"))

from extractor import WireExtraction, cascade_llm_pdf, extract, llm_pdf_pages, pdf_subset_b64  # noqa: E402
from verifier import ErrorClass, verify  # noqa: E402

PDF = ROOT / "data/raw/southwark_s106_sample.pdf"
FIXTURE = ROOT / "s106-audit/fixtures/southwark_s106_sample/claude-haiku-5-5.extract.json"
CYCLE = "Cycle Hire Docking Station Contribution"


def response(wire: dict, model: str):
    return SimpleNamespace(stop_reason="end_turn", stop_details=None, model=model,
                           parsed_output=WireExtraction.model_validate(wire),
                           usage=SimpleNamespace(input_tokens=1000, output_tokens=100))


class FakeClient:
    """Stands in for anthropic.Anthropic: records each call and returns canned structured output."""

    def __init__(self, plain=None, beta=None):
        self.calls = []
        self.messages = SimpleNamespace(parse=lambda **kw: self._call("messages", plain, kw))
        self.beta = SimpleNamespace(messages=SimpleNamespace(parse=lambda **kw: self._call("beta", beta, kw)))

    def _call(self, ns, wire, kw):
        self.calls.append((ns, kw))
        return response(wire, kw["model"])


def parser_errors(results):
    return [r for r in results if r.error_class == ErrorClass.PARSER_ERROR]


@unittest.skipUnless(PDF.exists(), "sample deed not fetched")
class LlmPdfTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.wire = json.loads(FIXTURE.read_text())["output"]

    def test_page_subset(self):
        _, pages = extract(PDF, backend="llm-pdf", llm_mode="replay")
        nums = llm_pdf_pages(pages)
        import base64

        import pypdfium2 as pdfium
        doc = pdfium.PdfDocument(base64.b64decode(pdf_subset_b64(PDF, nums)))
        self.assertEqual(len(doc), len(nums))
        self.assertLess(len(nums), 50)

    def test_replay_cli_exit_code(self):
        env = {k: v for k, v in os.environ.items() if not k.startswith("ANTHROPIC_")}
        out = subprocess.run([sys.executable, "s106-audit/run_audit.py", "--backend", "llm-pdf", "--llm-mode", "replay", "-q",
                              "--out", os.devnull], cwd=ROOT, env=env, capture_output=True, text=True)
        self.assertEqual(out.returncode, 0, out.stdout + out.stderr)
        self.assertIn("backend=llm-pdf:claude-haiku-5-5 (replay)", out.stdout)

    def test_live_request_shape(self):
        client = FakeClient(plain=self.wire)
        a, pages = extract(PDF, backend="llm-pdf", client=client)
        ns, kw = client.calls[0]
        self.assertEqual((ns, kw["model"]), ("messages", "claude-haiku-5-5"))
        doc, text = kw["messages"][0]["content"]
        self.assertEqual(doc["source"]["media_type"], "application/pdf")
        self.assertIs(kw["output_format"], WireExtraction)
        self.assertEqual(a.extraction_backend, "llm-pdf:claude-haiku-5-5")
        # cited attachment pages are mapped back to deed pages
        self.assertEqual(a.housing.mix_table.evidence.page, 91)
        self.assertEqual(parser_errors(verify(a, pages)), [])

    def _broken_haiku(self):
        wire = copy.deepcopy(self.wire)
        c = next(c for c in wire["contributions"] if c["name"] == CYCLE)
        c["amount_in_words"] = "twelve thousand and six hundred pounds"  # misread: words != figure
        c["evidence"]["snippet"] = "£12,650 (twelve thousand and six hundred pounds)"
        return wire

    def test_cascade_fixes_parser_error(self):
        _, pages = extract(PDF, backend="rules")
        a, _ = extract(PDF, backend="llm-pdf", client=FakeClient(plain=self._broken_haiku()))
        results = verify(a, pages)
        self.assertEqual([r.check_id for r in parser_errors(results)], ["F7.words"])

        opus = copy.deepcopy(self.wire)
        opus["contributions"] = [c for c in opus["contributions"] if c["name"] == CYCLE]
        client = FakeClient(beta=opus)
        fixed = cascade_llm_pdf(PDF, a, pages, results, client=client)
        ns, kw = client.calls[0]
        self.assertEqual((ns, kw["model"], kw["fallbacks"]), ("beta", "claude-opus-5-5", "default"))
        prompt = kw["messages"][0]["content"][1]["text"]
        self.assertIn(CYCLE, prompt)
        self.assertNotIn("Bus Contribution", prompt)
        self.assertEqual(parser_errors(verify(fixed, pages)), [])
        self.assertIn("kept 1", fixed.warnings[-1])

    def test_cascade_rejects_no_improvement(self):
        _, pages = extract(PDF, backend="rules")
        broken = self._broken_haiku()
        a, _ = extract(PDF, backend="llm-pdf", client=FakeClient(plain=broken))
        results = verify(a, pages)
        same = copy.deepcopy(broken)
        same["contributions"] = [c for c in same["contributions"] if c["name"] == CYCLE]
        out = cascade_llm_pdf(PDF, a, pages, results, client=FakeClient(beta=same))
        self.assertEqual([r.check_id for r in parser_errors(verify(out, pages))], ["F7.words"])
        self.assertIn("rejected 1", out.warnings[-1])

    def test_cascade_rejects_dropped_value(self):
        # Review repro: deleting the amount and marking it contingent turns the FAIL into a SKIP,
        # which lowers the Parser Error count but must not count as an improvement.
        _, pages = extract(PDF, backend="rules")
        a, _ = extract(PDF, backend="llm-pdf", client=FakeClient(plain=self._broken_haiku()))
        results = verify(a, pages)
        opus = copy.deepcopy(self.wire)
        opus["contributions"] = [c for c in opus["contributions"] if c["name"] == CYCLE]
        opus["contributions"][0].update(amount_gbp=None, amount_in_words=None, contingent=True)
        out = cascade_llm_pdf(PDF, a, pages, results, client=FakeClient(beta=opus))
        self.assertEqual([r.check_id for r in parser_errors(verify(out, pages))], ["F7.words"])
        self.assertIn("rejected 1", out.warnings[-1])
        self.assertIn("newly marked contingent", out.warnings[-1])

    def test_replay_rejects_different_pdf(self):
        import tempfile

        import extractor
        rec = json.loads(FIXTURE.read_text())
        rec["pdf_sha256"] = "0" * 64
        with tempfile.TemporaryDirectory() as d:
            fx = Path(d) / FIXTURE.parent.name / FIXTURE.name
            fx.parent.mkdir()
            fx.write_text(json.dumps(rec))
            saved, extractor.FIXTURE_DIR = extractor.FIXTURE_DIR, Path(d)
            try:
                with self.assertRaisesRegex(ValueError, "different PDF"):
                    extract(PDF, backend="llm-pdf", llm_mode="replay")
            finally:
                extractor.FIXTURE_DIR = saved


if __name__ == "__main__":
    unittest.main()
