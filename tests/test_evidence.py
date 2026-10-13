"""evidence_found(): two-column OCR interleaving re-locates, changed or misplaced evidence does not.

    .venv/bin/python -m unittest discover -s tests

The pages are verbatim RapidOCR output from the Southwark sample, so no data download is needed.
"""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "s106-audit"))

from schema import Evidence  # noqa: E402
from verifier import evidence_found  # noqa: E402

PAGE_4 = '''"Affordable Housing has the meaning given inparagraph 1of Schedule 4;
Pnm MaximumProvision"
"Affordable Housing means the sumof f10,058.60(ten thousand and fifty eightpounds
MonitoringContribution" andsixtypence)Index Linkedtobepaidby theOwner tothe
Council in accordance withparagraph1of Schedule 6and applied
bytheCounciltowardsthecostofmonitoring theprovisionof the
Affordable Housing Units;
'''

PAGE_12 = '''"Cycle Hire Docking Station means the sumof f12,650(twelve thousand and sixhundred and
Contribution" ftypounds) Index Linked to be paid by the Owner to the Council in
accordancewithparagraph1ofSchedule6andtobegivenby the
Council to Transport for London to enableTransport for London to
provide a Cycle Hire Docking Station within the vicinity of the
Development;
"Default Employment in the means a financial contribution of f73,100 (seventy three thousand
EndUseContribution" one hundred pounds) Index Linked which may be payable by the
'''

PAGE_17 = '''"Environmental Health means the sumoff10,120(ten thousandandone hundred and
Officer Contribution" twentypounds)Index Linked tobepaid by theOwner totheCouncil
inaccordancewithparagraph1ofSchedule6andappliedby thie
Council towardsthecostofmonitoring theDemolitionEnvironmental
'''

PAGES = {4: PAGE_4, 12: PAGE_12, 17: PAGE_17}


def found(page: int, snippet: str) -> bool:
    return evidence_found(Evidence(page=page, snippet=snippet), PAGES)


class InterleavingTest(unittest.TestCase):
    def test_left_column_term_spliced_at_line_wrap(self):
        self.assertTrue(found(4, "£10,058.60 (ten thousand and fifty eight pounds and sixty pence) Index Linked"))
        self.assertTrue(found(12, "£73,100 (seventy three thousand one hundred pounds) Index Linked"))
        self.assertTrue(found(17, "£10,120 (ten thousand and one hundred and twenty pounds) Index Linked"))

    def test_pound_sign_read_as_f(self):
        self.assertTrue(found(4, "means the sum of £10,058.60 (ten thousand and fifty eight pounds"))
        self.assertTrue(found(12, "means the sum of £12,650 (twelve thousand and six hundred and"))

    def test_one_garbled_plain_word(self):
        # OCR reads "the" as "thie"; 16 plain words allow one skip
        self.assertTrue(found(17, "in accordance with paragraph 1 of Schedule 6 and applied by the Council "
                                  "towards the cost of monitoring"))

    def test_exact_substring_still_matches(self):
        self.assertTrue(found(12, "Index Linked to be paid by the Owner"))


class StrictnessTest(unittest.TestCase):
    def test_changed_figure_fails(self):
        self.assertFalse(found(12, "means the sum of £12,560 (twelve thousand and six hundred and"))
        self.assertFalse(found(4, "£10,058.50 (ten thousand and fifty eight pounds and sixty pence) Index Linked"))

    def test_figure_must_be_digit_bounded(self):
        self.assertFalse(found(12, "means the sum of £2,650 (twelve thousand and six hundred and"))
        self.assertFalse(found(12, "means the sum of £12,65 (twelve thousand and six hundred and"))

    def test_changed_number_word_fails(self):
        self.assertFalse(found(4, "£10,058.60 (ten thousand and fifty eight pounds and seventy pence) Index Linked"))
        self.assertFalse(found(17, "£10,120 (ten thousand and two hundred and twenty pounds) Index Linked"))

    def test_dropped_words_are_not_absorbed_by_a_gap(self):
        # the page reads "six hundred and [fifty] pounds"; the snippet leaves out "and fifty"
        self.assertFalse(found(12, "£12,650 (twelve thousand and six hundred pounds)"))
        self.assertFalse(found(17, "£10,120 (ten thousand and one hundred pounds) Index Linked"))

    def test_joined_number_words_at_line_wrap_are_not_absorbed(self):
        # review repro: the snippet leaves out "and twenty", which OCR joined at the line start
        page = {1: "means the sumoff10,120(ten thousand and one hundred\nandtwenty pounds)Index Linked\n"}
        ev = Evidence(page=1, snippet="£10,120 (ten thousand and one hundred pounds) Index Linked")
        self.assertFalse(evidence_found(ev, page))
        page = {1: "means the sumoff10,010(ten thousand and\nandten pounds)Index Linked\n"}
        ev = Evidence(page=1, snippet="£10,010 (ten thousand and pounds) Index Linked")
        self.assertFalse(evidence_found(ev, page))

    def test_digits_at_line_wrap_are_not_absorbed(self):
        page = {1: "means the sum of f12,650\nClause 4.2 Index Linked to be paid by the Owner\n"}
        ev = Evidence(page=1, snippet="means the sum of £12,650 Index Linked to be paid")
        self.assertFalse(evidence_found(ev, page))
        # the same layout without the figure re-locates
        page = {1: "means the sum of f12,650\nContribution\" Index Linked to be paid by the Owner\n"}
        self.assertTrue(evidence_found(ev, page))

    def test_snippet_from_another_page_fails(self):
        self.assertTrue(found(4, "£10,058.60 (ten thousand and fifty eight pounds and sixty pence) Index Linked"))
        self.assertFalse(found(12, "£10,058.60 (ten thousand and fifty eight pounds and sixty pence) Index Linked"))
        self.assertFalse(found(17, "£73,100 (seventy three thousand one hundred pounds) Index Linked"))

    def test_missing_page_or_evidence(self):
        self.assertFalse(found(99, "means the sum of £12,650"))
        self.assertFalse(evidence_found(None, PAGES))


if __name__ == "__main__":
    unittest.main()
