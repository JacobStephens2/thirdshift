import copy
import datetime
import http.server
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading

binary = sys.argv[2] if len(sys.argv) > 2 else str(Path('target/debug/thirdshift').resolve())
mode = sys.argv[1]
assert mode in ('order', 'name-collision')
with tempfile.TemporaryDirectory(prefix='fleet-spec-review-') as temp:
    home = Path(temp)
    root = home / '.thirdshift'
    records = root / 'logs/acme/widgets/commands/issue'
    records.mkdir(parents=True)
    (root / 'config.toml').write_text('[console]\nport = 0\n')
    now = datetime.datetime.now(datetime.timezone.utc).isoformat()
    record = {
        'version': 1, 'kind': 'Run', 'repository': 'acme/widgets',
        'name': 'local-failed', 'issue': None, 'hostname': 'fixture',
        'process_id': 0, 'process_start': 'gone', 'harness': 'codex',
        'model': 'gpt-6', 'effort': 'high', 'started_at': now,
        'ended_at': now, 'stages': [], 'session_log': None,
        'outcome': 'failed', 'cause': None, 'issue_branch': None,
        'pull_request': None,
    }
    (records / '1-fixture.command.json').write_text(json.dumps(record))
    env = dict(os.environ, HOME=str(home))
    def run(*args, **kwargs):
        return subprocess.run([binary, *args], env=env, text=True,
                              capture_output=True, check=True, **kwargs).stdout
    local = json.loads(run('status', '--json'))
    assert len(local['commands']) == 1, local
    remote = copy.deepcopy(local)
    remote['hostname'] = 'remote-host'
    remote['active_runs'] = 1
    row = remote['commands'][0]
    row.update(name='remote-silent', outcome=None, ended_at=None,
               state='running', needs_you_kind='silent', group='needs you')
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path == '/api/v1/whoami':
                value = dict(version=1, api_version='v1', hostname='remote-host',
                             thirdshift_version='fixture', scopes=['read'])
            else:
                assert self.path.startswith('/api/v1/status?all='), self.path
                value = remote
            body = json.dumps(value).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        def log_message(self, *args):
            pass
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        alias = 'remote' if mode == 'order' else local['hostname']
        address = f'http://127.0.0.1:{server.server_port}'
        run('console', 'fleet', 'add', alias, address, input='fixture-token\n')
        merged = json.loads(run('status', '--json'))
        assert len(merged['commands']) == 2, merged
        text = run('status')
        if mode == 'order':
            observed = [r['needs_you_kind'] for r in merged['commands']]
            print('Merged Needs you kinds:', observed)
            print(text)
            assert observed == ['silent', 'failed'], 'Silent must precede ended Failed across Fleet machines'
        else:
            print('JSON contains machines:', [r['machine'] for r in merged['commands']])
            print(text)
            assert 'remote-silent' in text, 'A paired Fleet entry named after the local hostname loses its row in human status'
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
