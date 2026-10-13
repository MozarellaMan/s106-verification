"""Trigger labels are decided by the cited phrase, and the gate rejects unsupported triggers.

    .venv/bin/python -m unittest discover -s tests

Phrases are the ones Claude Haiku 5.5 cited across live runs on the Southwark sample,
including the runs whose labels drifted.
"""
import sys
import unittest
from decimal import Decimal
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "s106-audit"))

from schema import Evidence, FinancialContribution, TriggerEvent  # noqa: E402
from triggers import classify, imposes_payment  # noqa: E402
from verifier import Status, check_trigger, ocr_digits  # noqa: E402

E = TriggerEvent

# Verbatim RapidOCR output for Schedule 11 para 3 (p89 of the sample): "50%" reads as "5o%".
PAGE_89 = """3. CarbonGreenFund Contribution
3.1 Prior to the Occupation of 5o%of theDevelopment(or upon such other dateas may be
agreed between the Council and the Owner) the Council and the Owner shall compare the
environmental performance of the Development against the Agreed Carbon Targets and
3.5 The Carbon GreenFundContribution(as agreed between the Council and the Owner or
determined in accordance withclause 19of this Agreement) shall be payable within 20
WorkingDaysofsuchagreementordetermination."""


class ClassifyTest(unittest.TestCase):
    def test_cited_phrases(self):
        cases = {
            "within 28 days of receiving written request": E.ON_DEMAND,
            "the Owner will pay the Default Employment in the End Use Contribution within 28 days": E.ON_DEMAND,
            "shall be payable within 20 Working Days of such agreement or determination.": E.ON_DEMAND,
            "To pay the Site and Development Contributions on or before Implementation.": E.PRE_IMPLEMENTATION,
            "not to Implement the Development until the Green Run-off Rate Off-Set": E.PRE_IMPLEMENTATION,
            "to pay the Bus Contribution on or before first Occupation of any part of the Development": E.PRE_OCCUPATION,
            "The Owner shall not Occupy or permit Occupation of more than 50 per cent of the Market": E.PRE_OCCUPATION,
            "To pay the Carbon Green Fund Contribution in accordance with the procedure outlined in paragraph 3": E.OTHER,
            "prior to Commencement of the Development": E.PRE_COMMENCEMENT,
            "on completion of this Agreement": E.ON_COMPLETION,
            "on the date hereof": E.ON_COMPLETION,
        }
        for phrase, event in cases.items():
            with self.subTest(phrase=phrase):
                self.assertEqual(classify(phrase), event)

    def test_ocr_spacing_does_not_change_the_label(self):
        self.assertEqual(classify("TopaytheCycleHireDockingStationContributiononorbeforeOccupation."), E.PRE_OCCUPATION)
        self.assertEqual(classify("within twenty eight days of demand"), E.ON_DEMAND)

    def test_earliest_phrase_wins(self):
        self.assertEqual(classify("Not to Occupy until paid, and to pay within 28 days of demand"), E.PRE_OCCUPATION)

    def test_unrecognised(self):
        self.assertIsNone(classify("as the Council may direct"))
        # Review repro: construction completion is not completion of the agreement.
        self.assertIsNone(classify("on completion of construction"))
        self.assertIsNone(classify("upon completion of the Development"))
        self.assertIsNone(classify(None))


def contribution(phrase: str, clause: str, event: TriggerEvent, model: TriggerEvent | None = None):
    return FinancialContribution(name="Carbon Green Fund Contribution", amount_gbp=Decimal("475991.00"),
                                 trigger_event=event, trigger_event_model=model, trigger_clause=clause,
                                 trigger_evidence=Evidence(page=89, snippet=phrase))


CLAUSE_3_5 = ("The Carbon Green Fund Contribution (as agreed between the Council and the Owner or determined in "
              "accordance with clause 19 of this Agreement) shall be payable within 20 Working Days of such "
              "agreement or determination.")
CLAUSE_3_1 = ("Prior to the Occupation of 50% of the Development (or upon such other date as may be agreed between "
              "the Council and the Owner) the Council and the Owner shall compare the environmental performance of "
              "the Development against the Agreed Carbon Targets")


class CheckTriggerTest(unittest.TestCase):
    def run_check(self, c):
        return {r.check_id.split(".")[1]: r for r in check_trigger(c, {89: PAGE_89}, "F5", [8])}

    def test_payment_clause_passes_and_records_model_label(self):
        c = contribution("payable within 20 Working Days of", CLAUSE_3_5, E.ON_DEMAND, model=E.OTHER)
        r = self.run_check(c)
        self.assertEqual(r["trig"].status, Status.PASS)
        self.assertEqual(r["tlabel"].status, Status.PASS)
        self.assertIn("model proposed 'other'", r["tlabel"].detail)

    def test_assessment_clause_is_not_a_trigger(self):
        # Live run 1 cited the assessment timing (3.1), which makes nothing payable.
        c = contribution("Prior to the Occupation of 50% of the Development", CLAUSE_3_1, E.PRE_OCCUPATION)
        r = self.run_check(c)
        self.assertEqual(r["trig"].status, Status.FAIL)
        self.assertIn("does not impose a payment", r["trig"].detail)

    def test_invented_payment_clause_fails(self):
        # Review repro: a real timing phrase wrapped in a payment covenant the deed doesn't contain.
        c = contribution("Prior to the Occupation of 50% of the Development",
                         "Prior to the Occupation of 50% of the Development the Owner shall pay the Carbon Green "
                         "Fund Contribution to the Council", E.PRE_OCCUPATION)
        r = self.run_check(c)
        self.assertEqual(r["trig"].status, Status.FAIL)
        self.assertIn("payment clause not re-locatable", r["trig"].detail)

    def test_question_clause_present_in_source_fails(self):
        # Re-review repros: the clause is verbatim in the deed, but it asks whether a sum is payable.
        for clause in ("Prior to Occupation the Council shall assess whether the Carbon Green Fund Contribution "
                       "is payable.",
                       "Prior to Occupation the Council shall assess if the Contribution is payable",
                       "PriortoOccupationtheCouncilshallassessiftheContributionispayable"):
            with self.subTest(clause=clause):
                c = contribution("Prior to Occupation", clause, E.PRE_OCCUPATION)
                r = {x.check_id.split(".")[1]: x for x in check_trigger(c, {89: PAGE_89 + "\n" + clause}, "F1", [8])}
                self.assertEqual(r["trig"].status, Status.FAIL)
                self.assertIn("does not impose a payment", r["trig"].detail)
                self.assertNotIn("tlabel", r)

    def test_phrase_must_sit_in_the_clause(self):
        c = contribution("Prior to the Occupation of 50% of the Development", CLAUSE_3_5, E.PRE_OCCUPATION)
        r = self.run_check(c)
        self.assertEqual(r["trig"].status, Status.FAIL)
        self.assertIn("not part of the cited clause", r["trig"].detail)

    def test_misquoted_phrase_fails(self):
        c = contribution("payable within 10 Working Days of", CLAUSE_3_5.replace("20", "10"), E.ON_DEMAND)
        self.assertEqual(self.run_check(c)["trig"].status, Status.FAIL)

    def test_label_must_follow_phrase(self):
        c = contribution("payable within 20 Working Days of", CLAUSE_3_5, E.PRE_OCCUPATION)
        r = self.run_check(c)
        self.assertEqual(r["trig"].status, Status.PASS)
        self.assertEqual(r["tlabel"].status, Status.FAIL)


class ImposesPaymentTest(unittest.TestCase):
    def test_obligations(self):
        for clause in ("To pay the Site and Development Contributions on or before Implementation.",
                       "1.3 TopaytheCycleHireDockingStationContributiononorbeforeOccupation.",
                       "Unless a lesser amount is specified in writing, to pay the Bus Contribution on or before "
                       "first Occupation",
                       "The Owner covenants with the Council to pay the Administration Cost within 28 days",
                       "the Owner will pay the Default Employment in the End Use Contribution within 28 days",
                       CLAUSE_3_5,
                       "Not to Implement the Development until the Site and Development Contributions have been "
                       "paid to the Council in full.",
                       "The Council shall assess the Contribution. The Contribution is payable within 28 days of "
                       "demand.",
                       "The sum (if any) is payable within 28 days of the Assessment.",
                       # "if" inside words is not a condition
                       "The specified sum is payable within 28 days of the date notified to the Owner",
                       "Lift Contribution is payable on or before Occupation"):
            with self.subTest(clause=clause):
                self.assertTrue(imposes_payment(clause))

    def test_mentions_are_not_obligations(self):
        # Review repro: mentioning a payable amount is not imposing payment.
        for clause in ("Prior to Occupation the Council shall assess the amount payable",
                       # Re-review repro: "is payable" inside a question is not an obligation.
                       "Prior to Occupation the Council shall assess whether the Carbon Green Fund Contribution "
                       "is payable.",
                       "PriortoOccupationtheCouncilshalldeterminewhethertheContributionispayable",
                       # Re-review repro: the same "assess if" question, with and without OCR spacing.
                       "Prior to Occupation the Council shall assess if the Contribution is payable",
                       "PriortoOccupationtheCouncilshallassessiftheContributionispayable",
                       "the Council will check if the Contribution shall be payable",
                       "3.2 IftheViabilityReviewshowsasurplustheContributionispayable",
                       "(a) If the Viability Review shows a surplus the Contribution is payable",
                       "If the Viability Review shows a surplus the Affordable Housing Contribution is payable",
                       "the Council shall calculate the sum to be paid by the Owner",
                       CLAUSE_3_1):
            with self.subTest(clause=clause):
                self.assertFalse(imposes_payment(clause))


class OcrDigitsTest(unittest.TestCase):
    def test_letters_inside_numbers(self):
        self.assertEqual(ocr_digits("of 5o%of the"), "of 50%of the")
        self.assertEqual(ocr_digits("sum of f1o,058"), "sum of f10,058")
        self.assertEqual(ocr_digits("l0 days"), "10 days")

    def test_letters_next_to_words_untouched(self):
        self.assertEqual(ocr_digits("withparagraph1of Schedule"), "withparagraph1of Schedule")
        self.assertEqual(ocr_digits("Block1 and Lot2"), "Block1 and Lot2")



class RetryTest(unittest.TestCase):
    """A truncated structured output (invalid JSON) is retried, then reported."""

    def test_retries_then_succeeds(self):
        from unittest import mock

        from pydantic import BaseModel, ValidationError

        import extractor

        class M(BaseModel):
            x: int

        try:
            M.model_validate_json('{"x": "1')
        except ValidationError as e:
            truncated = e
        calls = [truncated, ("out", {"input_tokens": 1, "output_tokens": 1}, "claude-haiku-5-5")]

        def fake(*a, **k):
            r = calls.pop(0)
            if isinstance(r, Exception):
                raise r
            return r

        with mock.patch.object(extractor, "_call_claude", side_effect=fake):
            out, usage, _ = extractor._call_with_retry(None, "claude-haiku-5-5", "", "", "medium", False)
        self.assertEqual(out, "out")
        self.assertEqual(usage["attempts"], 2)

        with mock.patch.object(extractor, "_call_claude", side_effect=truncated):
            with self.assertRaisesRegex(RuntimeError, "invalid structured output 3 times"):
                extractor._call_with_retry(None, "claude-haiku-5-5", "", "", "medium", False)



class GroupTermTest(unittest.TestCase):
    def test_plural_group_terms_only(self):
        from extractor import GROUP_TERM
        self.assertTrue(GROUP_TERM.search("Site and Development Contributions"))
        self.assertTrue(GROUP_TERM.search("TfL Contributions"))
        self.assertFalse(GROUP_TERM.search("Bus Contribution"))
        self.assertFalse(GROUP_TERM.search("Administration Cost"))


if __name__ == "__main__":
    unittest.main()
