"""AC-PL: exercise the package lifecycle without touching the real user's settings."""
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[1]
PLUGIN = ROOT / 'plugins/kunsu'


class LifecycleTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location('kunsu_manage', PLUGIN/'scripts/manage.py')
        cls.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.module)

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.home = Path(tmp.name)
        self.config = self.home/'.codex/config.toml'
        self.config.parent.mkdir()
        self.config.write_text('''# Keep my choices\nmodel = "my-model"\n[mcp_servers.other]\nurl = "https://fixture.invalid"\n[plugins."other@personal"]\nenabled = false\n[mcp_servers.kunsu]\nenabled = true\nurl = "http://127.0.0.1:4777/mcp"\n''')
        self.market = self.home/'.agents/plugins/marketplace.json'
        self.market.parent.mkdir(parents=True)
        self.market.write_text(json.dumps({'name':'personal','plugins':[
            {'name':'other','source':{'source':'local','path':'./plugins/other'},'custom':'keep'}]}))
        self.source = self.home/'plugins/kunsu'
        self.source.mkdir(parents=True)
        (self.source/'old.txt').write_text('previous-source')
        self.shared = self.home/'.claude/skills/kunsu'
        self.shared.mkdir(parents=True)
        (self.shared/'SKILL.md').write_text('previous-skill')
        (self.shared/'personal-note.md').write_text('keep-this-extra-file')
        self.old_config = self.config.read_bytes()
        self.old_market = self.market.read_bytes()
        self.calls = []

    def runner(self, args):
        self.calls.append(args)
        version = json.loads((PLUGIN/'plugin.json').read_text())['version']
        cache = self.home/'.codex/plugins/cache/personal/kunsu'/version
        if 'add' in args:
            shutil.copytree(self.source, cache, dirs_exist_ok=True)
            result = {}
        else:
            result = {'installed':[{'pluginId':'kunsu@personal','installed':True,
                                   'enabled':True,'version':version}]}
        return subprocess.CompletedProcess(args,0,json.dumps(result),'')

    def install(self, runner=None):
        return self.module.install(PLUGIN,self.home,'fixture-codex',runner or self.runner,
                                   lambda: {'ok':True,'tool_count':13})

    def test_install_twice_preserves_other_settings_and_shared_extras(self):
        first = self.install()
        after = self.config.read_bytes()
        second = self.install()
        self.assertTrue(first['ok'] and second['ok'])
        self.assertEqual(self.config.read_bytes(),after)
        conf = tomllib.loads(after.decode())
        old = tomllib.loads(self.old_config.decode())
        self.assertEqual(conf['model'],old['model'])
        self.assertEqual(conf['mcp_servers']['other'],old['mcp_servers']['other'])
        self.assertEqual(conf['plugins']['other@personal'],old['plugins']['other@personal'])
        self.assertFalse(conf['mcp_servers']['kunsu']['enabled'])
        self.assertTrue(conf['plugins']['kunsu@personal']['enabled'])
        self.assertEqual(conf['plugins']['kunsu@personal']['mcp_servers']['kunsu-inbox']
                         ['default_tools_approval_mode'],'approve')
        market = json.loads(self.market.read_text())
        self.assertEqual(market['plugins'][0],json.loads(self.old_market)['plugins'][0])
        self.assertEqual(sum(x['name']=='kunsu' for x in market['plugins']),1)
        self.assertEqual((self.shared/'personal-note.md').read_text(),'keep-this-extra-file')
        self.assertEqual((self.shared/'SKILL.md').read_bytes(),
                         (PLUGIN/'skills/kunsu/SKILL.md').read_bytes())
        self.assertTrue(all('remove' not in c for c in self.calls))

    def test_private_backups_and_host_failure_restore_original_files(self):
        def fail(args):
            return subprocess.CompletedProcess(args,1,'','fixture secret must not be printed')
        with self.assertRaises(self.module.LifecycleError) as error:
            self.install(fail)
        self.assertNotIn('fixture secret',str(error.exception))
        self.assertEqual(self.config.read_bytes(),self.old_config)
        self.assertEqual(self.market.read_bytes(),self.old_market)
        self.assertEqual((self.source/'old.txt').read_text(),'previous-source')
        self.assertEqual((self.shared/'SKILL.md').read_text(),'previous-skill')
        backup = next((self.home/'.kunsu/backups').glob('plugin-*'))
        self.assertEqual(backup.stat().st_mode & 0o777,0o700)
        self.assertEqual((backup/'config.toml').stat().st_mode & 0o777,0o600)
        self.assertTrue((backup/'failed-source/plugin.json').exists())

    def test_concurrent_external_edit_is_not_overwritten_on_failure(self):
        def fail(args):
            self.config.write_text('model = "external-new-choice"\n')
            return subprocess.CompletedProcess(args,1,'','')
        with self.assertRaises(self.module.LifecycleError) as error:
            self.install(fail)
        self.assertIn('conflict',str(error.exception))
        self.assertEqual(self.config.read_text(),'model = "external-new-choice"\n')

    def test_post_install_probe_failure_is_not_reported_as_success(self):
        from unittest.mock import Mock
        probe = Mock(side_effect=[{'ok':True},{'ok':False}])
        with self.assertRaises(self.module.LifecycleError):
            self.module.install(PLUGIN,self.home,'fixture-codex',self.runner,probe)
        self.assertEqual(self.config.read_bytes(),self.old_config)

    def test_invalid_settings_fail_before_any_product_mutation(self):
        for content in ('not = valid [', '[plugins]\n"kunsu@personal" = true\n'):
            self.config.write_text(content)
            with self.assertRaises(self.module.LifecycleError): self.install()
            self.assertEqual(self.config.read_text(),content)
            self.assertEqual(self.market.read_bytes(),self.old_market)
            self.assertTrue((self.source/'old.txt').exists())
        self.config.write_bytes(self.old_config)
        self.market.write_text('{"name":"company","plugins":[]}')
        with self.assertRaises(self.module.LifecycleError): self.install()
        self.assertEqual(self.config.read_bytes(),self.old_config)

    def test_symlink_or_divergent_manifest_is_rejected(self):
        copy = self.home/'package'
        shutil.copytree(PLUGIN,copy)
        (copy/'leak').symlink_to(self.config)
        with self.assertRaises(self.module.LifecycleError): self.module.validate_package(copy)
        (copy/'leak').unlink()
        manifest = json.loads((copy/'.codex-plugin/plugin.json').read_text())
        manifest['version']='99.0.0'
        (copy/'.codex-plugin/plugin.json').write_text(json.dumps(manifest))
        with self.assertRaises(self.module.LifecycleError): self.module.validate_package(copy)

    def test_drafts_cannot_escape_overwrite_or_submit(self):
        for template in ('decision','urgent','diary','retrospective','batch'):
            path = self.module.prepare_draft(PLUGIN,self.home,'pa-fixture-'+template,template)
            self.assertEqual(path.parent,self.home/'.kunsu/drafts')
            self.assertTrue((path/'question.md').is_file())
            self.assertNotIn('conversation_id:',(path/'question.md').read_text())
            with self.assertRaises(self.module.LifecycleError):
                self.module.prepare_draft(PLUGIN,self.home,path.name,template)
        for name in ('../outside','/tmp/escape','pa-a/../b',''):
            with self.assertRaises(self.module.LifecycleError):
                self.module.prepare_draft(PLUGIN,self.home,name,'diary')
        self.assertEqual(self.calls,[])
        self.assertEqual(self.config.read_bytes(),self.old_config)

    def test_cli_doctor_failure_is_actionable_and_has_no_traceback(self):
        import os
        result = subprocess.run(['python3',str(PLUGIN/'scripts/manage.py'),'doctor'],
            env=dict(os.environ,HOME=str(self.home),PYTHONDONTWRITEBYTECODE='1'),
            capture_output=True,text=True,timeout=10)
        self.assertNotEqual(result.returncode,0)
        payload = json.loads(result.stdout)
        self.assertFalse(payload['ok'])
        self.assertIn('Kunsu',payload['error'])
        self.assertNotIn('Traceback',result.stderr)


if __name__ == '__main__': unittest.main()
