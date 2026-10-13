"""Known-answer controls for immutable references, human corrections and multiple cases."""
import copy
import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from review_cases import FORMAT, apply_records, comparison_layers, digest, import_review, review_summary, write_json
from pdf_fixture import make_pdf


class ReviewControls(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.case = self.root / 'case-a'
        self.case.mkdir()
        self.pdf = self.root / 'source.pdf'
        source = make_pdf(self.pdf, 'Executed contribution: 90 pounds')
        self.sha = hashlib.sha256(source).hexdigest()
        self.evidence = [{'pdf_page': 1, 'printed_page': '1', 'clause': 'Contribution definition', 'quote': 'Executed contribution: 90 pounds', 'coordinates': None}]
        self.claim = {'id': 'F01', 'title': 'Test contribution', 'priority': 'high', 'interpretation': 'Agent incorrectly proposes 100 pounds', 'annotation_origin': 'agent_proposed', 'expected_fields': {'contribution.Test.amount.value_gbp': '100'}, 'evidence': self.evidence, 'ambiguity': None, 'human_review_status': None}
        self.reference = {'reference_version': 1, 'source_sha256': self.sha, 'status': 'awaiting_human_confirmation', 'claims': [self.claim]}
        self.provenance = {'planning_reference': 'TEST/A', 'site_address': 'Synthetic site', 'pdf_pages': 1, 'sha256': self.sha, 'source_url': 'https://example.org/source', 'register_url': 'https://example.org/register', 'retrieval_date_europe_london': 'not recorded', 'identity_evidence_pdf_pages': [1]}
        write_json(self.case / 'reference.json', self.reference)
        write_json(self.case / 'provenance.json', self.provenance)
        (self.case / 'reference-freeze.sha256').write_text(digest(self.case / 'reference.json'))
        for file in ['protocol.md', 'completeness.md']:
            (self.case / file).write_text('Synthetic control')
        self.audit = {'agreement': {'provenance': {'sha256': self.sha}, 'housing': {'tenures': []}, 'contributions': [{'name': 'Test', 'amount': {'value_gbp': '100'}}]}, 'verification': []}

    def payload(self, status='confirmed', previous=None):
        return {'format': FORMAT, 'case_id': self.case.name, 'provenance_sha256': digest(self.case/'provenance.json'), 'review_origin': 'human_entered', 'source_sha256': self.sha, 'reference_version': 1, 'reference_sha256': digest(self.case / 'reference.json'), 'claims': [{'claim_id': 'F01', 'human_review_status': status, 'human_reviewer': 'Synthetic Reviewer', 'human_review_date': '2026-10-13', 'human_notes': 'Synthetic control; not a real review', 'previous_review_sha256': previous, 'corrections': {}}]}

    def ingest(self, payload):
        path = self.root / 'export.json'
        write_json(path, payload)
        return import_review(self.case, path)

    def test_correction_preserves_agent_reference_and_changes_human_metric(self):
        before = (self.case / 'reference.json').read_bytes()
        payload = self.payload('corrected')
        payload['claims'][0]['corrections'] = {'expected_fields': {'contribution.Test.amount.value_gbp': '90'}, 'evidence': self.evidence, 'interpretation': 'Human reads the executed 90-pound contribution'}
        record = self.ingest(payload)
        base, view, _, active, history = apply_records(self.case)
        self.assertEqual((self.case / 'reference.json').read_bytes(), before)
        self.assertEqual(base['claims'][0]['expected_fields']['contribution.Test.amount.value_gbp'], '100')
        self.assertEqual(view['claims'][0]['expected_fields']['contribution.Test.amount.value_gbp'], '90')
        self.assertEqual(history[0]['record_sha256'], digest(record))
        result = comparison_layers(self.case, self.audit)
        self.assertEqual(result['agent_labelled']['agree'], 1)
        self.assertEqual(result['human_confirmed']['different'], 1)
        self.assertEqual(result['human_confirmed']['compared_fields'], 1)
        self.assertEqual(review_summary(self.case)['status'], 'partially_human_reviewed')

    def test_wrong_binding_identity_values_and_pages_are_rejected_without_events(self):
        mutations = [lambda p: p.update(case_id='other-case'), lambda p: p.update(provenance_sha256='wrong'), lambda p: p.update(source_sha256='wrong'), lambda p: p.update(reference_version=2), lambda p: p.update(reference_sha256='wrong'), lambda p: p.update(review_origin='agent_proposed'), lambda p: p['claims'][0].update(human_reviewer=''), lambda p: p['claims'][0].update(human_review_date='bad-date'), lambda p: p['claims'].append(copy.deepcopy(p['claims'][0]))]
        for mutate in mutations:
            payload = self.payload()
            mutate(payload)
            with self.assertRaises(ValueError):
                self.ingest(payload)
        payload = self.payload('corrected')
        payload['claims'][0]['corrections'] = {'expected_fields': {'contribution.Test.amount.value_gbp': '90'}, 'evidence': [{**self.evidence[0], 'pdf_page': 2}]}
        with self.assertRaises(ValueError): self.ingest(payload)
        payload['claims'][0]['corrections']['evidence'] = self.evidence
        payload['claims'][0]['corrections']['expected_fields'] = {'contribution.Test.amount.value_gbp': 'NaN'}
        with self.assertRaises(ValueError): self.ingest(payload)
        self.assertEqual(list((self.case / 'reviews').glob('*.json')), [])

    def test_supersession_is_explicit_idempotent_and_rejection_not_confirmed(self):
        first = self.ingest(self.payload())
        self.assertEqual(self.ingest(self.payload()), first)
        with self.assertRaises(ValueError): self.ingest(self.payload('rejected'))
        second = self.ingest(self.payload('rejected', digest(first)))
        self.assertNotEqual(first, second)
        result = comparison_layers(self.case, self.audit)
        self.assertIsNone(result['human_confirmed'])
        self.assertEqual(review_summary(self.case)['decisions']['rejected'], 1)
        # Corrupt a predecessor: chain validation must notice, not silently use new data.
        altered = json.loads(first.read_text())
        altered['claims'][0]['human_notes'] = 'Tampered predecessor'
        write_json(first, altered)
        with self.assertRaises(ValueError): apply_records(self.case)

    def test_completion_requires_explicit_provenance_and_completeness(self):
        self.ingest(self.payload())
        self.assertNotEqual(review_summary(self.case)['status'], 'human_review_complete')
        payload = self.payload()
        payload['claims'] = [{**payload['claims'][0], 'claim_id': cid, 'scope': 'All source pages in this synthetic control'} for cid in ['@provenance', '@completeness']]
        self.ingest(payload)
        self.assertEqual(review_summary(self.case)['status'], 'human_review_complete')

    def test_human_added_obligation_extends_only_the_human_subset(self):
        payload = self.payload('added')
        entry = payload['claims'][0]
        entry['claim_id'] = 'HUMAN01'
        entry['new_claim'] = {**copy.deepcopy(self.claim), 'id': 'HUMAN01', 'title': 'Omitted payment', 'expected_fields': {'contribution.Omitted.amount.value_gbp': '25'}}
        record = self.ingest(payload)
        base, view, _, active, history = apply_records(self.case)
        self.assertEqual(len(base['claims']), 1)
        self.assertEqual(len(view['claims']), 2)
        result = comparison_layers(self.case, self.audit)
        self.assertEqual(result['agent_labelled']['compared_fields'], 1)
        self.assertEqual(result['human_confirmed']['compared_fields'], 1)
        self.assertEqual(result['human_confirmed']['missing'], 1)
        self.assertEqual(review_summary(self.case)['human_added_claims'], 1)
        later = self.payload('rejected', digest(record))
        later['claims'][0]['claim_id'] = 'HUMAN01'
        self.ingest(later)
        self.assertIsNone(comparison_layers(self.case, self.audit)['human_confirmed'])

    def test_human_can_start_a_prepared_case_without_agent_fields(self):
        self.reference['claims'] = []
        write_json(self.case / 'reference.json', self.reference)
        (self.case / 'reference-freeze.sha256').write_text(digest(self.case / 'reference.json'))
        payload = self.payload('added')
        payload['claims'][0]['claim_id'] = 'HUMAN01'
        payload['claims'][0]['new_claim'] = {**copy.deepcopy(self.claim), 'id': 'HUMAN01', 'expected_fields': {'contribution.Test.amount.value_gbp': '90'}}
        self.ingest(payload)
        result = comparison_layers(self.case, self.audit)
        self.assertIsNone(result['agent_labelled'])
        self.assertEqual(result['human_confirmed']['different'], 1)

    def test_false_confirmation_and_missing_correction_evidence_fail(self):
        payload = self.payload()
        payload['claims'][0]['corrections'] = {'expected_fields': {'contribution.Test.amount.value_gbp': '90'}, 'evidence': self.evidence}
        with self.assertRaises(ValueError): self.ingest(payload)
        payload['claims'][0]['human_review_status'] = 'corrected'
        payload['claims'][0]['corrections'].pop('evidence')
        with self.assertRaises(ValueError): self.ingest(payload)
        payload = self.payload('added')
        with self.assertRaises(ValueError): self.ingest(payload)
        self.assertEqual(list((self.case / 'reviews').glob('*.json')), [])

    @unittest.skipUnless(shutil.which('pdftoppm') and shutil.which('pdfinfo'), 'Poppler required')
    def test_two_case_registry_and_dynamic_report_overlay(self):
        registry = self.root / 'inputs/cases.json'
        for cid in ['one', 'two']:
            cmd = [sys.executable, str(ROOT / 'scripts/manage-validation.py'), '--registry', str(registry), 'add', '--id', cid, '--pdf', str(self.pdf), '--planning-ref', f'TEST/{cid}', '--site-address', f'{cid} synthetic site', '--council', 'Synthetic council', '--source-url', 'https://example.org/source', '--register-url', 'https://example.org/register', '--reference', str(self.case / 'reference.json')]
            subprocess.run(cmd, check=True, capture_output=True)
        self.assertEqual(len(json.loads(registry.read_text())['cases']), 2)
        payload = self.payload('corrected')
        payload['case_id'] = 'one'
        payload['provenance_sha256'] = digest(self.root / 'inputs/one/provenance.json')
        payload['reference_sha256'] = digest(self.root / 'inputs/one/reference.json')
        payload['claims'][0]['corrections'] = {'expected_fields': {'contribution.Test.amount.value_gbp': '90'}, 'evidence': self.evidence}
        write_json(self.root / 'export-one.json', payload)
        subprocess.run([sys.executable, str(ROOT / 'scripts/manage-validation.py'), '--registry', str(registry), 'import-review', 'one', str(self.root / 'export-one.json')], check=True, capture_output=True)
        audit = self.root / 'audit.json'
        write_json(audit, self.audit)
        manifest = json.loads(registry.read_text())
        for e in manifest['cases']: e['audit'] = '../audit.json'
        write_json(registry, manifest)
        out = self.root / 'generated'
        subprocess.run([sys.executable, str(ROOT / 'scripts/manage-validation.py'), '--registry', str(registry), 'build', '--out', str(out)], check=True, capture_output=True)
        summary = json.loads((out / 'summary.json').read_text())
        first, second = summary['cases']
        self.assertEqual(first['human_confirmed']['different'], 1)
        self.assertIsNone(second['human_confirmed'])
        self.assertIn('IMPORTED HUMAN DECISION: CORRECTED', (out / 'one/dossier.html').read_text())
        self.assertIn('90', (out / 'one/report.md').read_text())
        self.assertTrue((out / 'index.html').exists())
        self.assertEqual(json.loads((self.root / 'inputs/two/reference.json').read_text())['claims'][0]['human_review_status'], None)


if __name__ == '__main__': unittest.main()
