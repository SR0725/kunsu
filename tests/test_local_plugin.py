"""AC-LP-01/02/03/05/06: local plugin transport and selected artwork gates."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from urllib.error import HTTPError, URLError
from unittest.mock import Mock

ROOT = Path(__file__).resolve().parents[1]
PLUGIN = ROOT / 'plugins/kunsu'
BRIDGE_PATH = PLUGIN / 'scripts/mcp_bridge.py'
TOOLS = {'ask_create', 'ask_register', 'ask_wait', 'ask_answers_since', 'ask_list',
         'ask_get', 'ask_update', 'ask_cancel', 'ask_supersede', 'ask_report',
         'dnd_get', 'dnd_set', 'dnd_clear', 'ask_followup_reply'}


class TransportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location('kunsu_bridge', BRIDGE_PATH)
        cls.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.module)

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.token = Path(self.temp.name) / 'token'
        self.token.write_text('private-fixture-token\n')
        self.transport = Mock()
        self.transport.open.return_value = io.BytesIO(json.dumps({
            'jsonrpc': '2.0', 'id': 7, 'result': {'tools': [{'name': x} for x in TOOLS]}
        }).encode())
        self.bridge = self.module.Bridge(self.token, self.transport)

    def call(self, method='tools/list', params=None):
        return self.bridge.handle({'jsonrpc': '2.0', 'id': 7, 'method': method,
                                   'params': params or {}})

    def test_loopback_only_and_rotation(self):
        self.call()
        req = self.transport.open.call_args.args[0]
        self.assertEqual(req.full_url, 'http://127.0.0.1:4777/mcp')
        self.assertEqual(req.get_header('Authorization'), 'Bearer private-fixture-token')
        self.assertNotIn('private-fixture-token', req.data.decode())
        self.token.write_text('rotated-token')
        self.transport.open.return_value = io.BytesIO(b'{"jsonrpc":"2.0","id":7,"result":{}}')
        self.call('ping')
        self.assertEqual(self.transport.open.call_args.args[0].get_header('Authorization'), 'Bearer rotated-token')

    def test_no_proxy_or_redirect_handler(self):
        bridge = self.module.Bridge(self.token)
        self.assertEqual(bridge.proxy_handler.proxies, {})
        with self.assertRaises(HTTPError):
            bridge.redirect_handler.redirect_request(Mock(), None, 302, 'redirect', {}, 'https://outside.invalid/')

    def test_tool_surface_filters_and_forbids_answers(self):
        self.assertEqual({x['name'] for x in self.call()['result']['tools']}, TOOLS)
        for tool in ('answer', 'ask_answer', 'approve', 'execute'):
            self.transport.reset_mock()
            res = self.call('tools/call', {'name': tool, 'arguments': {}})
            self.assertIn('error', res)
            self.transport.open.assert_not_called()
        self.transport.reset_mock()
        self.assertIn('error', self.call('http/answer'))
        self.transport.open.assert_not_called()

    def test_unknown_upstream_tool_is_not_advertised(self):
        self.transport.open.return_value = io.BytesIO(b'{"result":{"tools":[{"name":"answer"},{"name":"dnd_get"}]}}')
        self.assertEqual(self.call()['result']['tools'], [{'name': 'dnd_get'}])

    def test_malformed_tool_name_is_an_error_not_a_process_crash(self):
        for name in ([], {}, 1, None):
            self.transport.reset_mock()
            self.assertIn('error', self.call('tools/call', {'name': name}))
            self.transport.open.assert_not_called()

    def test_source_is_preserved_not_guessed_from_server_environment(self):
        source = {'agent': 'codex', 'cwd': '/work', 'host': 'codex_app',
                  'conversation_id': '00000000-0000-4000-8000-000000000001'}
        self.call('tools/call', {'name': 'ask_create', 'arguments': {'source': source}})
        posted = json.loads(self.transport.open.call_args.args[0].data)
        self.assertEqual(posted['params']['arguments']['source'], source)

    def test_notifications_are_silent(self):
        self.assertIsNone(self.bridge.handle({'jsonrpc': '2.0', 'method': 'notifications/initialized'}))
        self.transport.open.assert_not_called()

    def test_offline_and_missing_token_recover_without_secrets(self):
        self.transport.open.side_effect = URLError('fixture private-fixture-token')
        response = self.call()
        self.assertIn('error', response)
        self.assertNotIn('private-fixture-token', json.dumps(response))
        self.token.write_text('')
        self.transport.reset_mock()
        self.assertIn('error', self.call())
        self.transport.open.assert_not_called()
        self.token.unlink()
        self.assertIn('error', self.call())
        self.token.write_text('restored')
        self.transport.open.side_effect = None
        self.assertIn('result', self.call())

    def test_stdio_bad_line_recovers_and_only_protocol_is_printed(self):
        with tempfile.TemporaryDirectory() as tmp:
            import os
            env = dict(os.environ, HOME=tmp, PYTHONDONTWRITEBYTECODE='1')
            lines = 'bad-json\n' + json.dumps({'jsonrpc':'2.0','method':'notifications/initialized'}) + '\n'
            lines += json.dumps({'jsonrpc':'2.0','id':4,'method':'ping'}) + '\n'
            result = subprocess.run(['/usr/bin/python3', '-u', str(BRIDGE_PATH)], input=lines,
                                    text=True, capture_output=True, env=env, timeout=5)
            self.assertEqual(result.returncode, 0)
            replies = [json.loads(x) for x in result.stdout.splitlines()]
            self.assertEqual(len(replies), 2)
            self.assertEqual(replies[0]['error']['code'], -32700)
            self.assertEqual(replies[1]['id'], 4)
            self.assertEqual(result.stderr, '')


class PackageTests(unittest.TestCase):
    def test_portable_and_compatibility_manifests_match(self):
        portable = json.loads((PLUGIN / 'plugin.json').read_text())
        compat = json.loads((PLUGIN / '.codex-plugin/plugin.json').read_text())
        for field in ('name', 'version', 'description'):
            self.assertEqual(portable[field], compat[field])
        presentation = portable['extensions']['com.openai']['interface']
        self.assertEqual(presentation, compat['interface'])
        self.assertLessEqual(len(presentation['shortDescription']), 30)
        for field in ('logo', 'composerIcon'):
            self.assertTrue((PLUGIN / presentation[field]).is_file())
        mcp = json.loads((PLUGIN / 'mcp.json').read_text())['mcpServers']['kunsu-inbox']
        self.assertEqual(mcp['command'], 'python3')
        self.assertEqual(mcp['args'], ['-u', '${PLUGIN_ROOT}/scripts/mcp_bridge.py'])
        self.assertNotIn('env', mcp)
        for p in PLUGIN.rglob('*'):
            self.assertFalse(p.is_symlink())
            if p.is_file():
                self.assertNotIn(p.name, ('token', '.env'))
                self.assertNotIn('node_modules', p.parts)

    def test_selected_artwork_and_native_sizes(self):
        from PIL import Image
        master = ROOT / 'assets/brand/kunsu-master.png'
        self.assertEqual(hashlib.sha256(master.read_bytes()).hexdigest(),
                         '476b1cd31f845540ebd010642c0bf5ab4d2365c14a7fc36ef3d5ae1ae9025f84')
        for filename, size in [('32x32.png',32),('128x128.png',128),('128x128@2x.png',256),('icon.png',512)]:
            with Image.open(ROOT/'src-tauri/icons'/filename) as img:
                self.assertEqual(img.size, (size,size))
                self.assertEqual(img.mode, 'RGBA')
                self.assertEqual(img.getchannel('A').getextrema(), (0,255))
        self.assertEqual((ROOT/'src-tauri/icons/icon.icns').read_bytes()[:4], b'icns')
        # 選單列用 K 剪影 template（AC-AS-14，2026-10-12 換掉圓圈）
        self.assertEqual(hashlib.sha256((ROOT/'src-tauri/icons/tray-k-template.png').read_bytes()).hexdigest(),
                         'd30cf26cb4dbe97f0bfbc741960d6c888670197f42733144a3629f3e90e8de75')
        self.assertEqual((PLUGIN/'assets/logo.png').read_bytes(), (ROOT/'src-tauri/icons/icon.png').read_bytes())
        self.assertEqual((ROOT/'ui/public/favicon.png').read_bytes(), (ROOT/'src-tauri/icons/128x128.png').read_bytes())


if __name__ == '__main__':
    unittest.main()
