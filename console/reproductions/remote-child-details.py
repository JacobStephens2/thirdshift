import datetime
import html
import json
import os
from pathlib import Path
import re
import selectors
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

binary = str(Path(os.environ.get('THIRDSHIFT_BINARY', Path(__file__).resolve().parents[2] / 'target/debug/thirdshift')).resolve())
processes = []

def cli(home, *args, input=None):
    return subprocess.check_output([binary, *args], env={**os.environ, 'HOME': str(home)}, text=True, input=input)

def start(home):
    p = subprocess.Popen([binary, 'console', '--no-sign-in'], env={**os.environ, 'HOME': str(home), 'THIRDSHIFT_FLEET_INTERVAL_MS': '100'}, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    processes.append(p)
    with selectors.DefaultSelector() as ready:
        ready.register(p.stdout, selectors.EVENT_READ)
        assert ready.select(15), 'Console startup timeout'
    return p.stdout.readline().strip()

def get(url):
    try:
        with urllib.request.urlopen(url, timeout=10) as response:
            return response.status, response.read().decode()
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode()

with tempfile.TemporaryDirectory(prefix='thirdshift-child-details-') as tmp:
    caller, owner = [Path(tmp) / name for name in ['caller', 'owner']]
    for home in [caller, owner]:
        (home / '.thirdshift').mkdir(parents=True)
        (home / '.thirdshift/config.toml').write_text('[console]\nport = 0\n')
    folder = owner / '.thirdshift/logs/acme/widgets/commands/issue'
    folder.mkdir(parents=True)
    now = datetime.datetime.now(datetime.timezone.utc).isoformat()
    parent_path = folder / '7-fixture.command.json'
    base = dict(version=1, kind='Spec run', name='parent', repository='acme/widgets', hostname='fixture', process_id=0, process_start='gone', harness='codex', started_at=now, ended_at=now, stages=[dict(name='Tickets', began_at=now)], session_log=None, outcome='failed')
    parent = {**base, 'issue': dict(number=7, url='https://github.com/acme/widgets/issues/7', title='Parent issue')}
    child_path = folder / '7-fixture.command.issue-99.child.json'
    child = {**base, 'kind':'Run', 'name':'child', 'parent_record':str(parent_path), 'issue':dict(number=99, url='https://github.com/acme/widgets/issues/99', title='Child issue'), 'stages':[dict(name='implement', began_at=now)]}
    parent_path.write_text(json.dumps(parent))
    child_path.write_text(json.dumps(child))
    if '--utf8' in sys.argv:
        (folder / '7-fixture.log').write_bytes(b'IMPORTANT failure cause: ' + b'\xff' * 2000 + b'\nlast remote line\n')
    else:
        (folder / '7-fixture.log').write_text('Command output\n')
    try:
        owner_url = start(owner)
        token = cli(owner, 'console', 'token', 'create', 'caller').strip()
        cli(caller, 'console', 'fleet', 'add', 'remote', owner_url, input=token+'\n')
        caller_url = start(caller)
        subscription = urllib.request.urlopen(caller_url + '/events', timeout=10)
        deadline = time.time()+10
        while True:
            _, inbox = get(caller_url)
            rows = re.findall(r'<li class="command[^>]*>', inbox)
            children = [row for row in rows if 'data-machine="remote"' in row and '::child' in row]
            if children:
                break
            assert time.time()<deadline, inbox
            time.sleep(.05)
        child_row = children[0]
        path = html.unescape(re.search(r'data-peek="([^"]+)"', child_row).group(1))
        owner_path = '/peek?' + urllib.parse.urlencode({'command':str(child_path)+'::child'})
        assert get(owner_url + owner_path)[0] == 200
        status, body = get(caller_url + path)
        print('Remote child selector:', html.unescape(re.search(r'data-command="([^"]+)"', child_row).group(1)))
        print('Remote response:', status, body[:250])
        assert status == 200, 'A remote child Ticket row must open its own details in place'
        assert 'Child issue' in body
        if '--utf8' in sys.argv:
            def preview(value):
                return value.split('aria-label="Recent Command output"')[1].split('<pre>')[1].split('</pre>')[0]
            local_preview, remote_preview = preview(get(owner_url + owner_path)[1]), preview(body)
            print('Owner contains failure cause:', 'IMPORTANT failure cause:' in local_preview)
            print('Remote contains failure cause:', 'IMPORTANT failure cause:' in remote_preview)
            assert local_preview == remote_preview, 'Remote preview must match the owning Console at UTF-8 tail boundaries'
    finally:
        for process in processes:
            process.terminate()
        for process in processes:
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
