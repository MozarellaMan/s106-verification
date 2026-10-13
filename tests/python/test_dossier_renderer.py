"""A synthetic one-page source controls rendering, hashing and review-file preservation."""
import csv, hashlib, json, shutil, subprocess, sys, tempfile, unittest
from html.parser import HTMLParser
from pathlib import Path
from pdf_fixture import make_pdf
ROOT=Path(__file__).resolve().parents[2]
class Links(HTMLParser):
    def __init__(self, root): super().__init__();self.root=root
    def handle_starttag(self, tag, attrs):
        for k,v in attrs:
            if k in ('src','href') and v and not v.startswith(('http:','https:','#')):
                assert (self.root/v.split('#')[0]).exists(),v
@unittest.skipUnless(shutil.which('pdftoppm'), 'Poppler required for page-render integration test')
class Renderer(unittest.TestCase):
    def test_source_render_and_destructive_controls(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);case=root/'case';case.mkdir();pdf=root/'input.pdf';out=root/'site'
            data=make_pdf(pdf)
            sha=hashlib.sha256(data).hexdigest()
            provenance=dict(sha256=sha,pdf_pages=1,planning_reference='TEST/1',site_address='Test & site',retrieval_date_europe_london='2026-10-13',source_url='https://example.org/source',register_url='https://example.org/register',identity_evidence_pdf_pages=[1])
            reference=dict(source_sha256=sha,reference_version=1,claims=[dict(id='H1',priority='high',title='<script>unsafe title</script>',interpretation='Known units',ambiguity=None,human_review_status=None,evidence=[dict(pdf_page=1,printed_page='1',clause='Definition',quote='Known source: 100 units',coordinates=None)])])
            (case/'provenance.json').write_text(json.dumps(provenance));(case/'reference.json').write_text(json.dumps(reference))
            (case/'reference-freeze.sha256').write_text(hashlib.sha256((case/'reference.json').read_bytes()).hexdigest())
            for name in ('completeness.md','protocol.md'):(case/name).write_text('Synthetic control')
            cmd=[sys.executable,str(ROOT/'scripts/build-southwark-dossier.py'),'--case',str(case),'--pdf',str(pdf),'--out',str(out)]
            result=subprocess.run(cmd,capture_output=True,text=True);self.assertEqual(result.returncode,0,result.stderr)
            self.assertTrue((out/'evidence/page-001.png').read_bytes().startswith(b'\x89PNG'))
            page=(out/'dossier.html').read_text();self.assertIn('&lt;script&gt;unsafe title&lt;/script&gt;',page);Links(out).feed(page)
            with (out/'human-review.csv').open() as file: rows=list(csv.DictReader(file))
            self.assertEqual(rows[0]['human_review_status'],'')
            csv_path=out/'human-review.csv';csv_path.write_text('saved review draft\n');subprocess.run(cmd,check=True,capture_output=True);self.assertEqual(csv_path.read_text(),'saved review draft\n')
            # Incorrect PDF must be rejected before any output exists.
            pdf.write_bytes(data+b'changed');badout=root/'badsite';bad=subprocess.run(cmd[:-1]+[str(badout)],capture_output=True,text=True);self.assertNotEqual(bad.returncode,0);self.assertIn('hash mismatch',bad.stderr);self.assertFalse(badout.exists())
            pdf.write_bytes(data)
            # An invalid reference/freeze must also fail closed.
            (case/'reference.json').write_text('{}');bad=subprocess.run(cmd,capture_output=True,text=True);self.assertNotEqual(bad.returncode,0)
if __name__=='__main__':unittest.main()
