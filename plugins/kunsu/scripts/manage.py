#!/usr/bin/env python3
"""Local package installation, read-only diagnosis and unsubmitted drafts. Python 3.11+."""
import argparse
from datetime import datetime, timezone
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tomllib
import uuid

sys.dont_write_bytecode = True
PLUGIN = Path(__file__).resolve().parents[1]
TEMPLATES = ('decision', 'urgent', 'diary', 'retrospective', 'batch')


class LifecycleError(Exception):
    pass


def read(path):
    return path.read_bytes() if path.exists() else None


def safe_path(path, home):
    for candidate in (path, *path.parents):
        if candidate == home:
            break
        if candidate.is_symlink():
            raise LifecycleError('Symlink destination refused; keep settings in their existing location.')


def atomic_write(path, content):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name('.' + path.name + '-' + uuid.uuid4().hex)
    with os.fdopen(os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'wb') as f:
        f.write(content)
        f.flush()
        os.fsync(f.fileno())
    os.replace(temporary, path)


def validate_package(plugin):
    files = []
    for path in plugin.rglob('*'):
        if path.is_symlink():
            raise LifecycleError('Package contains a symlink; use a contained source copy.')
        if path.is_file() and '__pycache__' not in path.parts and path.suffix != '.pyc':
            if path.name in ('.env', 'token') or 'node_modules' in path.parts:
                raise LifecycleError('Package contains credential or dependency files.')
            files.append(path.relative_to(plugin))
    try:
        manifest = json.loads((plugin/'plugin.json').read_text())
        compat = json.loads((plugin/'.codex-plugin/plugin.json').read_text())
        assert manifest['name'] == 'kunsu'
        assert re.fullmatch(r'\d+\.\d+\.\d+', manifest['version'])
        assert all(manifest[k] == compat[k] for k in ('name', 'version', 'description'))
        assert manifest['extensions']['com.openai']['interface'] == compat['interface']
        for template in TEMPLATES:
            assert (plugin/'skills/kunsu/templates'/template/'question.md').is_file()
    except (OSError, ValueError, KeyError, AssertionError):
        raise LifecycleError('Kunsu package is incomplete or manifests disagree.') from None
    return manifest, files


def configure(text):
    """Edit only three owned tables, preserving every other TOML value and comment."""
    try:
        original = tomllib.loads(text)
        for path in (('mcp_servers', 'kunsu'), ('plugins', 'kunsu@personal'),
                     ('plugins', 'kunsu@personal', 'mcp_servers', 'kunsu-inbox')):
            value = original
            for key in path:
                value = value.get(key, {})
                if not isinstance(value, dict):
                    raise ValueError('non-table setting')
        updates = [(('plugins', 'kunsu@personal'), {'enabled': True}),
                   (('plugins', 'kunsu@personal', 'mcp_servers', 'kunsu-inbox'),
                    {'default_tools_approval_mode': 'approve'})]
        if 'kunsu' in original.get('mcp_servers', {}):
            updates.append((('mcp_servers', 'kunsu'), {'enabled': False}))
        for target, values in updates:
            sections = list(re.finditer(r'(?m)^[ \t]*(\[\[?.+?\]\]?)[ \t]*(?:#[^\r\n]*)?$', text))
            found = None
            for index, section in enumerate(sections):
                header = tomllib.loads(section.group(1))
                keys = []
                while isinstance(header, dict) and len(header) == 1:
                    key, header = next(iter(header.items()))
                    keys.append(key)
                if tuple(keys) == target:
                    found = (section.end(), sections[index+1].start() if index+1 < len(sections) else len(text))
                    break
            if found is None:
                name = '.'.join(k if re.fullmatch(r'[\w-]+', k) else json.dumps(k) for k in target)
                text = text.rstrip('\n') + '\n\n[' + name + ']\n'
                found = (len(text), len(text))
            start, end = found
            body = text[start:end]
            for key, value in values.items():
                line = key + ' = ' + json.dumps(value)
                pattern = r'(?m)^[ \t]*' + re.escape(key) + r'[ \t]*=.*$'
                if re.search(pattern, body):
                    body = re.sub(pattern, line, body)
                else:
                    body = body.rstrip('\n') + '\n' + line + '\n'
            text = text[:start] + body + text[end:]
        tomllib.loads(text)
        return text
    except (ValueError, TypeError, AttributeError):
        raise LifecycleError('Invalid or unsupported Kunsu TOML tables; settings were not changed.') from None


def marketplace(value):
    if not isinstance(value, dict) or value.get('name') != 'personal' or not isinstance(value.get('plugins'), list):
        raise LifecycleError('Expected the personal marketplace; no marketplace was changed.')
    result = json.loads(json.dumps(value))
    entries = result['plugins']
    if any(not isinstance(x, dict) for x in entries):
        raise LifecycleError('Invalid marketplace entries; no marketplace was changed.')
    own = next((x for x in entries if x.get('name') == 'kunsu'),
               {'name': 'kunsu', 'category': 'Productivity',
                'policy': {'installation': 'AVAILABLE', 'authentication': 'ON_INSTALL'}})
    own['source'] = {'source': 'local', 'path': './plugins/kunsu'}
    result['plugins'] = [x for x in entries if x.get('name') != 'kunsu'] + [own]
    return result


def codex_program():
    bundled = Path('/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex')
    if bundled.is_file() and os.access(bundled, os.X_OK):
        return str(bundled)
    program = shutil.which('codex')
    if not program:
        raise LifecycleError('Install Codex desktop or a compatible Codex CLI first.')
    return program


def run(args):
    return subprocess.run(args, capture_output=True, text=True, timeout=120)


def doctor(plugin=PLUGIN, home=None):
    home = home or Path.home()
    manifest, _ = validate_package(plugin)
    spec = importlib.util.spec_from_file_location('kunsu_private_bridge', plugin/'scripts/mcp_bridge.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    bridge = module.Bridge(home/'.kunsu/token')
    names = None
    for number, (method, params) in enumerate((('initialize', {'protocolVersion':'2024-11-05',
            'capabilities':{}, 'clientInfo':{'name':'kunsu-doctor','version':manifest['version']}}),
            ('tools/list', {}), ('tools/call', {'name':'dnd_get', 'arguments':{}})), 1):
        reply = bridge.handle({'jsonrpc':'2.0', 'id':number, 'method':method, 'params':params})
        if 'error' in reply:
            raise LifecycleError(reply['error']['message'])
        result = reply.get('result', {})
        if result.get('isError'):
            raise LifecycleError('Kunsu read-only diagnosis failed; open Kunsu and run its doctor.')
        if method == 'tools/list':
            names = {x['name'] for x in result.get('tools', [])}
            if names != module.TOOLS:
                raise LifecycleError('Kunsu tool catalog differs; rebuild/install Kunsu before retrying.')
    return {'ok':True, 'version':manifest['version'], 'tool_count':len(names),
            'checks':['initialize', 'tools/list', 'dnd_get'], 'mutations':False}


def install(plugin=PLUGIN, home=None, codex=None, runner=run, probe=None):
    home = home or Path.home()
    manifest, files = validate_package(plugin)
    config = home/'.codex/config.toml'
    market = home/'.agents/plugins/marketplace.json'
    source = home/'plugins/kunsu'
    shared = home/'.claude/skills/kunsu'
    for path in (config, market, source, shared): safe_path(path, home)
    for rel in files:
        try: tail = rel.relative_to('skills/kunsu')
        except ValueError: continue
        safe_path(shared/tail, home)
    old_config, old_market = read(config), read(market)
    next_config = configure((old_config or b'').decode()).encode()
    try:
        next_market = marketplace(json.loads(old_market) if old_market else {'name':'personal','plugins':[]})
    except (ValueError, TypeError):
        raise LifecycleError('Invalid personal marketplace; no settings were changed.') from None
    next_market = (json.dumps(next_market, ensure_ascii=False, indent=2) + '\n').encode()
    codex = codex or codex_program()
    probe = probe or (lambda: doctor(plugin, home))
    if not probe().get('ok'):
        raise LifecycleError('Open Kunsu and resolve its doctor before installation.')
    backup = home/'.kunsu/backups'/('plugin-' + datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ-') + uuid.uuid4().hex[:8])
    backup.mkdir(parents=True, mode=0o700)
    os.chmod(backup, 0o700)
    for name, content in (('config.toml', old_config), ('marketplace.json', old_market)):
        if content is not None: atomic_write(backup/name, content)
    if shared.exists(): shutil.copytree(shared, backup/'shared-skill', symlinks=True)
    writes = []
    stage = home/'plugins'/('.kunsu-stage-' + uuid.uuid4().hex)
    moved_old = moved_new = False
    lock_path = home/'.kunsu/plugin-install.lock'
    with lock_path.open('a') as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise LifecycleError('Another Kunsu installation is active; retry after it completes.') from None
        try:
            for rel in files: atomic_write(stage/rel, (plugin/rel).read_bytes())
            if source.exists():
                source.rename(backup/'previous-source')
                moved_old = True
            stage.rename(source)
            moved_new = True
            updates = [(config, old_config, next_config), (market, old_market, next_market)]
            for rel in files:
                try: tail = rel.relative_to('skills/kunsu')
                except ValueError: continue
                path = shared/tail
                safe_path(path, home)
                updates.append((path, read(path), (plugin/rel).read_bytes()))
            for path, before, after in updates:
                if read(path) != before:
                    raise LifecycleError('Concurrent settings conflict; no external edits will be overwritten.')
                writes.append((path, before, after))
                atomic_write(path, after)
            installed = runner([codex, 'plugin', 'add', 'kunsu@personal', '--json'])
            if installed.returncode:
                raise LifecycleError('Codex plugin add failed. Local files restored; retry the supported installer.')
            listed = runner([codex, 'plugin', 'list', '--marketplace', 'personal', '--json'])
            if listed.returncode: raise LifecycleError('Codex could not verify the installed plugin.')
            matches = [x for x in json.loads(listed.stdout).get('installed', []) if x.get('pluginId') == 'kunsu@personal']
            if len(matches) != 1 or not all(matches[0].get(k) for k in ('installed', 'enabled')) or matches[0]['version'] != manifest['version']:
                raise LifecycleError('Installed Codex plugin version/enablement did not match source.')
            cache = home/'.codex/plugins/cache/personal/kunsu'/manifest['version']
            if any(not (cache/rel).is_file() or (cache/rel).read_bytes() != (source/rel).read_bytes() for rel in files):
                raise LifecycleError('Installed cache differs from source; re-run the supported installer.')
            if not probe().get('ok'):
                raise LifecycleError('Post-install Kunsu diagnosis failed.')
            receipt = {'ok':True, 'version':manifest['version'], 'backup_directory':str(backup),
                       'source':str(source), 'cache_matches_source':True, 'new_dialog_required':True}
            atomic_write(backup/'receipt.json', (json.dumps(receipt, ensure_ascii=False, indent=2)+'\n').encode())
            return receipt
        except Exception:
            conflicts = []
            for path, before, after in reversed(writes):
                if read(path) != after:
                    conflicts.append(str(path))
                    continue
                if before is not None: atomic_write(path, before)
                else:
                    target = backup/'created-files'/path.relative_to(home)
                    target.parent.mkdir(parents=True, exist_ok=True)
                    path.rename(target)
            if moved_new: source.rename(backup/'failed-source')
            if moved_old: (backup/'previous-source').rename(source)
            message = 'Installation failed; local files restored, attempted files retained. Backup: ' + str(backup)
            if conflicts: message += '. Concurrent conflict: external edits retained; review the backup before recovery.'
            raise LifecycleError(message) from None


def prepare_draft(plugin, home, question_id, template):
    if not re.fullmatch(r'(?:pa|rt)-[A-Za-z0-9][A-Za-z0-9_-]{0,100}', question_id) or template not in TEMPLATES:
        raise LifecycleError('Use a stable pa-/rt- id and a supported template; paths are not allowed.')
    path = home/'.kunsu/drafts'/question_id
    safe_path(path, home)
    try:
        path.mkdir(parents=True, exist_ok=False)
    except FileExistsError:
        raise LifecycleError('Draft already exists; it was not overwritten. Update it deliberately or choose another id.') from None
    atomic_write(path/'question.md', (plugin/'skills/kunsu/templates'/template/'question.md').read_bytes())
    return path


def main():
    parser = argparse.ArgumentParser(description='Kunsu 本機外掛：安裝、唯讀診斷與草稿準備')
    commands = parser.add_subparsers(dest='command', required=True)
    commands.add_parser('install')
    commands.add_parser('doctor')
    draft = commands.add_parser('draft')
    draft.add_argument('--template', choices=TEMPLATES, required=True)
    draft.add_argument('--id', required=True)
    args = parser.parse_args()
    try:
        if args.command == 'install': result = install()
        elif args.command == 'doctor': result = doctor()
        else:
            path = prepare_draft(PLUGIN, Path.home(), args.id, args.template)
            result = {'ok':True, 'draft_dir':str(path), 'submitted':False,
                      'next':'Edit question.md, then ~/.kunsu/bin/kunsu ask <draft_dir> --dry-run and inspect preview_png.'}
    except (LifecycleError, OSError, ValueError, subprocess.TimeoutExpired):
        # Keep raw filesystem/host errors and credentials out of logs.
        error = sys.exc_info()[1]
        message = str(error) if isinstance(error, LifecycleError) else 'Kunsu operation failed; check local paths/permissions and retry.'
        print(json.dumps({'ok':False, 'error':message}, ensure_ascii=False))
        return 1
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0


if __name__ == '__main__': sys.exit(main())
