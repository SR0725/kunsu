"""AC-PL-04/05: opt-in installed-app dry-runs; never creates or answers a question."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import unittest
from urllib.request import ProxyHandler, build_opener
import uuid

PLUGIN = Path(__file__).resolve().parents[1]/'plugins/kunsu'


@unittest.skipUnless(os.environ.get('KUNSU_LIVE_SMOKE') == '1', 'Requires the installed local Kunsu app')
class InstalledWorkflowTests(unittest.TestCase):
    def test_all_five_templates_preview_without_registering_questions(self):
        spec = importlib.util.spec_from_file_location('kunsu_manage_live', PLUGIN/'scripts/manage.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        opener = build_opener(ProxyHandler({}))
        def health():
            with opener.open('http://127.0.0.1:4777/health', timeout=10) as response:
                return json.loads(response.read())
        before = health()
        results = []
        for template in module.TEMPLATES:
            with self.subTest(template=template):
                draft = module.prepare_draft(PLUGIN,Path.home(),
                    'pa-plugin-qa-'+uuid.uuid4().hex[:8]+'-'+template,template)
                run = subprocess.run([str(Path.home()/'.kunsu/bin/kunsu'),'ask',str(draft),'--dry-run'],
                    capture_output=True,text=True,timeout=75)
                self.assertEqual(run.returncode,0,run.stdout)
                response = json.loads(run.stdout)
                self.assertTrue(response['ok'])
                self.assertTrue(response['dry_run'])
                self.assertEqual(response['issues'],[])
                self.assertTrue(Path(response['preview_png']).is_file())
                results.append({'template':template,'ok':True,'draft_dir':str(draft),
                                'preview_png':response['preview_png']})
        after = health()
        self.assertEqual(before['pending_count'],after['pending_count'])
        self.assertEqual(before['last_answer_at'],after['last_answer_at'])
        report = {'ok':True,'templates':results,'pending_unchanged':True,'last_answer_unchanged':True}
        receipt = os.environ.get('KUNSU_QA_RECEIPT')
        if receipt:
            Path(receipt).write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')


if __name__ == '__main__': unittest.main()
