"""Milestone 1, evidence integrity: field-level grounding, honest outcomes, basis, OCR cache keys.

    .venv/bin/python -m unittest discover -s tests

No deed download or credentials needed: pages are short verbatim RapidOCR excerpts from the
Southwark sample (19/AP/1773), and the cache test uses throwaway files.
"""
import sys
import tempfile
import unittest
from decimal import Decimal
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "s106-audit"))

import ocr  # noqa: E402
from schema import (AffordableBasis, AgreementMetadata, Evidence, HousingProvision, MixRow,  # noqa: E402
                    MixTable, Section106Agreement, Tenure, TenureAllocation)
from verifier import (ErrorClass, Input, Status, check_claimed_pct, check_mix_table,  # noqa: E402
                      check_unit_consistency, numbers_in, resolve, value_in_snippet)

# Verbatim OCR lines (spaces dropped as RapidOCR drops them).
PAGES = {
    5: '"AffordableHousingUnits" means the 76 Residential Units (271 Habitable Rooms) tobe',
    14: "445 sqm ancillary plant and equipment; and 253 residential\napartments(C3),35.75%affordablebyhabitable room,andother",
    22: '3 "Market Housing Units" meansthe177ResidentialUnitsofMarketHousingcomprised',
    91: ("UnitType Social Rented Housing Intermediate Housing\n1 bed 10 7\n2bed 24 5\n3bed 24 6\n"
         "Sub-Total 58 18\nTotal 76 (271 habitable rooms)"),
}
TOTAL = Evidence(page=14, snippet="253 residential apartments")
MARKET = Evidence(page=22, snippet="the 177 Residential Units of Market Housing")
AFFORDABLE = Evidence(page=5, snippet="the 76 Residential Units (271 Habitable Rooms)")


def agreement(**housing) -> Section106Agreement:
    meta = AgreementMetadata(council="London Borough of Southwark", source_pdf="x.pdf", page_count=109,
                             text_source="ocr")
    return Section106Agreement(metadata=meta, housing=HousingProvision(**housing))


class ValueInSnippetTest(unittest.TestCase):
    def test_digit_forms(self):
        self.assertTrue(value_in_snippet(Decimal("10058.60"), "the sum of £10,058.60 (ten thousand"))
        self.assertTrue(value_in_snippet(Decimal("10058.6"), "the sum of f10,058.60"))  # OCR reads £ as f
        self.assertTrue(value_in_snippet(Decimal("35.75"), "35.75%affordablebyhabitable room"))
        self.assertTrue(value_in_snippet(50, "Prior to the Occupation of 5o%of theDevelopment"))  # OCR o for 0
        self.assertTrue(value_in_snippet(212, "three bedroom units (212) Habitable Rooms"))

    def test_word_forms(self):
        self.assertTrue(value_in_snippet(76, "seventy six affordable housing units"))
        self.assertIn(Decimal(683100), numbers_in("six hundred and eighty three thousand and one hundred pounds"))

    def test_absent_or_wrong_value(self):
        self.assertFalse(value_in_snippet(75, "the 76 Residential Units (271 Habitable Rooms)"))
        self.assertFalse(value_in_snippet(Decimal("10058.00"), "the sum of £10,058.60"))
        self.assertFalse(value_in_snippet(7, "the 76 Residential Units"))  # digits inside a number don't count
        self.assertFalse(value_in_snippet(None, "76"))

    def test_text_values(self):
        self.assertTrue(value_in_snippet("ten thousand and fifty eight pounds", "(tenthousandandfiftyeightpounds"))


class ResolveTest(unittest.TestCase):
    """Each (assertion holds?, inputs grounded?) combination gets its own, honest outcome."""

    good = [Input("total units", 253, TOTAL)]

    def test_holds_and_grounded_passes(self):
        self.assertEqual(resolve("X", "d", True, self.good, PAGES).status, Status.PASS)

    def test_holds_but_ungrounded_needs_review(self):
        r = resolve("X", "d", True, [Input("total units", 250, TOTAL)], PAGES)
        self.assertEqual(r.status, Status.REVIEW)
        self.assertIsNone(r.error_class)
        self.assertIn("250 not stated in its cited snippet", r.detail)

    def test_fails_and_grounded_is_discrepancy_with_caveat(self):
        r = resolve("X", "d", False, self.good, PAGES)
        self.assertEqual(r.error_class, ErrorClass.DOCUMENT_DISCREPANCY)
        self.assertIn("confirm against the original deed", r.detail)

    def test_snippet_found_but_value_not_in_it_is_parser_error(self):
        # Review repro (B): the quotation exists in the deed, but the extracted value isn't in it.
        # This used to be classed as a Document Discrepancy ("all inputs verified verbatim").
        r = resolve("X", "d", False, [Input("total units", 250, TOTAL)], PAGES)
        self.assertEqual(r.error_class, ErrorClass.PARSER_ERROR)

    def test_missing_value_is_parser_error(self):
        r = resolve("X", "d", False, [Input("total units", None, TOTAL)], PAGES)
        self.assertEqual(r.error_class, ErrorClass.PARSER_ERROR)
        self.assertIn("not extracted", r.detail)

    def test_relocation_still_required(self):
        r = resolve("X", "d", True, [Input("total units", 253, Evidence(page=22, snippet="253 residential apartments"))],
                    PAGES)
        self.assertEqual(r.status, Status.REVIEW)
        self.assertIn("not re-locatable on p22", r.detail)


class ConsistentButInventedTest(unittest.TestCase):
    def test_arithmetic_alone_no_longer_passes(self):
        # Review repro (B): 250 = 174 + 76 adds up, but 250 and 174 aren't what the cited text says.
        # Previously a PASS, because evidence was only consulted on failure.
        a = agreement(total_units=250, total_units_evidence=TOTAL, market_units=174, market_units_evidence=MARKET,
                      affordable_units=76, affordable_units_evidence=AFFORDABLE)
        r = check_unit_consistency(a, PAGES)
        self.assertEqual(r.status, Status.REVIEW)
        self.assertIn("total units: 250", r.detail)
        self.assertIn("market units: 174", r.detail)

    def test_true_values_pass(self):
        a = agreement(total_units=253, total_units_evidence=TOTAL, market_units=177, market_units_evidence=MARKET,
                      affordable_units=76, affordable_units_evidence=AFFORDABLE)
        self.assertEqual(check_unit_consistency(a, PAGES).status, Status.PASS)


class PercentageBasisTest(unittest.TestCase):
    """Review concern E: an unsupported or unstated basis must not fall back to a units calculation."""

    def base(self, basis):
        return agreement(total_units=253, total_units_evidence=TOTAL, affordable_units=76,
                         affordable_units_evidence=AFFORDABLE, claimed_affordable_pct=Decimal("30.04"),
                         claimed_affordable_basis=basis,
                         claimed_affordable_evidence=Evidence(page=14, snippet="35.75%affordable"))

    def test_floorspace_basis_skips(self):
        # 76/253 = 30.04%, so a silent units fallback would wrongly PASS this.
        r = check_claimed_pct(self.base(AffordableBasis.FLOORSPACE), PAGES)
        self.assertEqual(r.status, Status.SKIP)
        self.assertIn("floorspace", r.detail)
        self.assertIsNone(r.actual)

    def test_unstated_basis_skips(self):
        r = check_claimed_pct(self.base(None), PAGES)
        self.assertEqual(r.status, Status.SKIP)
        self.assertIn("basis", r.detail)

    def test_units_basis_still_computes(self):
        r = check_claimed_pct(self.base(AffordableBasis.UNITS), PAGES)
        self.assertEqual(r.actual, "76/253 = 30.04%")
        # The cited snippet says 35.75, not 30.04, so the match is unproven rather than passed.
        self.assertEqual(r.status, Status.REVIEW)


class MixTableGroundingTest(unittest.TestCase):
    def table(self, rows):
        T = Tenure
        return agreement(
            affordable_units=76, affordable_units_evidence=AFFORDABLE, affordable_habitable_rooms=271,
            tenures=[TenureAllocation(tenure=T.SOCIAL_RENT, units=58, evidence=Evidence(page=5, snippet="the 76")),
                     TenureAllocation(tenure=T.INTERMEDIATE, units=18, evidence=Evidence(page=5, snippet="the 76"))],
            mix_table=MixTable(rows=[MixRow(unit_type=u, units_by_tenure={T.SOCIAL_RENT: s, T.INTERMEDIATE: i})
                                     for u, s, i in rows],
                               subtotals={T.SOCIAL_RENT: 58, T.INTERMEDIATE: 18}, total_units=76,
                               total_habitable_rooms=271,
                               evidence=Evidence(page=91, snippet="Total 76 (271 habitable rooms)")))

    def test_rows_on_page_pass(self):
        a = self.table([("1 bed", 10, 7), ("2 bed", 24, 5), ("3 bed", 24, 6)])
        h5 = [r for r in check_mix_table(a, PAGES) if r.check_id == "H5"]
        self.assertTrue(all(r.status == Status.PASS for r in h5), [r.detail for r in h5])

    def test_invented_row_that_still_sums_needs_review(self):
        # 10+24+24 and 7+5+6 both still sum correctly with the cells moved between rows
        a = self.table([("1 bed", 24, 7), ("2 bed", 10, 5), ("3 bed", 24, 6)])
        h5 = [r for r in check_mix_table(a, PAGES) if r.description.startswith("Mix table: Σ Social")][0]
        self.assertEqual(h5.status, Status.REVIEW)
        self.assertIn("row '1 bed' not found", h5.detail)

    def test_repeated_counts_need_repeats_on_the_line(self):
        page = {91: PAGES[91].replace("3bed 24 6", "3bed 24 24")}
        a = self.table([("1 bed", 10, 7), ("2 bed", 24, 5), ("3 bed", 24, 24)])
        rows_ok = check_mix_table(a, page)
        self.assertNotIn("row '3 bed'", " ".join(r.detail for r in rows_ok))
        rows_bad = check_mix_table(a, PAGES)  # the real line has one 24, not two
        self.assertIn("row '3 bed' not found", " ".join(r.detail for r in rows_bad))


class OcrCacheKeyTest(unittest.TestCase):
    """Review concern A: a different document under the same filename must not reuse cached text."""

    def test_same_name_different_content_gets_a_new_cache(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(ocr, "CACHE_ROOT", Path(tmp) / "cache"):
            pdf = Path(tmp) / "deed.pdf"
            pdf.write_bytes(b"version one")
            first = ocr._cache_dir(pdf)
            (first / "page_001.txt").write_text("old text")
            pdf.write_bytes(b"version two")
            second = ocr._cache_dir(pdf)
            self.assertNotEqual(first, second)
            self.assertFalse((second / "page_001.txt").exists())
            self.assertEqual(ocr._cache_dir(pdf), second)  # stable for unchanged content

    def test_settings_version_and_manifest(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(ocr, "CACHE_ROOT", Path(tmp)):
            pdf = Path(tmp) / "deed.pdf"
            pdf.write_bytes(b"content")
            d = ocr._cache_dir(pdf)
            self.assertTrue(d.name.endswith(ocr.OCR_VERSION))
            import json
            m = json.loads((d / "manifest.json").read_text())
            self.assertEqual(m["sha256"], ocr.pdf_sha256(pdf))
            self.assertEqual(m["ocr_version"], ocr.OCR_VERSION)
            with mock.patch.object(ocr, "OCR_VERSION", "v2-different-settings"):
                self.assertNotEqual(ocr._cache_dir(pdf), d)


if __name__ == "__main__":
    unittest.main()
