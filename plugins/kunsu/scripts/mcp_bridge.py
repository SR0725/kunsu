#!/usr/bin/env python3
"""Private stdio transport for the existing loopback Kunsu MCP, no dependencies."""
import json
from pathlib import Path
import sys
from urllib.error import HTTPError
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener

ENDPOINT = 'http://127.0.0.1:4777/mcp'
TOOLS = frozenset(('ask_create', 'ask_register', 'ask_wait', 'ask_answers_since',
                   'ask_list', 'ask_get', 'ask_update', 'ask_cancel',
                   'ask_supersede', 'ask_report', 'dnd_get', 'dnd_set', 'dnd_clear',
                   'ask_followup_reply'))


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise HTTPError(req.full_url, code, 'Kunsu does not follow redirects', headers, fp)


class Bridge:
    def __init__(self, token_path=None, transport=None):
        self.token_path = token_path or Path.home() / '.kunsu/token'
        self.proxy_handler = ProxyHandler({})
        self.redirect_handler = NoRedirect()
        self.transport = transport or build_opener(self.proxy_handler, self.redirect_handler)

    @staticmethod
    def error(request_id, code, message):
        return {'jsonrpc': '2.0', 'id': request_id, 'error': {'code': code, 'message': message}}

    def handle(self, message):
        if not isinstance(message, dict):
            return self.error(None, -32600, 'Expected a JSON-RPC object.')
        if 'id' not in message:
            return None
        request_id = message['id']
        method = message.get('method')
        if method not in ('initialize', 'ping', 'tools/list', 'tools/call'):
            return self.error(request_id, -32601, 'Kunsu supports initialize, ping and tools only.')
        params = message.get('params', {})
        if not isinstance(params, dict):
            return self.error(request_id, -32602, 'params must be an object.')
        if method == 'tools/call' and (not isinstance(params.get('name'), str) or params['name'] not in TOOLS):
            return self.error(request_id, -32602, 'Unknown Kunsu tool. Answering and approval are UI-only.')
        try:
            token = self.token_path.read_text().strip()
            if not token:
                raise ValueError('empty token')
        except (OSError, ValueError):
            return self.error(request_id, -32001, 'Kunsu token is unavailable. Open the installed Kunsu app first.')
        timeout = 20
        if method == 'tools/call' and params.get('name') == 'ask_wait':
            args = params.get('arguments', {})
            seconds = args.get('timeout_seconds', 0) if isinstance(args, dict) else 0
            if isinstance(seconds, (int, float)) and not isinstance(seconds, bool) and 0 <= seconds <= 540:
                timeout = max(20, seconds + 5)
        request = Request(ENDPOINT, data=json.dumps(message, ensure_ascii=False).encode(),
                          headers={'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json',
                                   'Accept': 'application/json'}, method='POST')
        try:
            with self.transport.open(request, timeout=timeout) as response:
                result = json.loads(response.read())
            if not isinstance(result, dict):
                raise ValueError('invalid upstream response')
            if method == 'tools/list' and isinstance(result.get('result'), dict):
                result['result']['tools'] = [x for x in result['result'].get('tools', [])
                                            if isinstance(x, dict) and x.get('name') in TOOLS]
            return result
        except Exception:
            # Never echo headers, exceptions or caller payloads (may contain private evidence).
            return self.error(request_id, -32002, 'Cannot reach authenticated Kunsu. Open Kunsu or run ~/.kunsu/bin/kunsu doctor, then retry.')


def main():
    bridge = Bridge()
    for line in sys.stdin:
        try:
            message = json.loads(line)
        except (ValueError, TypeError):
            reply = Bridge.error(None, -32700, 'Invalid JSON.')
        else:
            reply = bridge.handle(message)
        if reply is not None:
            print(json.dumps(reply, ensure_ascii=False, separators=(',', ':')), flush=True)


if __name__ == '__main__':
    main()
