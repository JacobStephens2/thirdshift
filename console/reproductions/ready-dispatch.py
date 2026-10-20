import datetime as dt
import json, os, pathlib, re, subprocess, tempfile, urllib.request
binary = str(pathlib.Path(os.environ.get('THIRDSHIFT_BINARY', 'target/debug/thirdshift')).resolve())
with tempfile.TemporaryDirectory(prefix='console-ready-review-') as home:
    root = pathlib.Path(home) / '.thirdshift'
    folder = root / 'logs/acme/widgets/commands/pickup'
    folder.mkdir(parents=True)
    (root / 'config.toml').write_text('[console]\nport = 0\n')
    now = dt.datetime.now(dt.timezone.utc).isoformat()
    common = dict(version=1, repository='acme/widgets', hostname='fixture', process_id=0,
                  process_start='gone', harness='codex', started_at=now, ended_at=now,
                  session_log=None, stages=[], outcome='ready for review', cause=None,
                  pull_request='https://github.com/acme/widgets/pull/20')
    dispatched = dict(common, kind='Run', name='acme/widgets#7',
                      issue=dict(number=7, url='https://github.com/acme/widgets/issues/7', title='Review me'))
    outer = dict(common, kind='Pickup run', name='pickup-fixture', issue=None,
                 dispatched=[dispatched])
    (folder / 'pickup-fixture.command.json').write_text(json.dumps(outer))
    (folder / 'pickup-fixture.log').write_text('done\n')
    env = dict(os.environ, HOME=home)
    status = json.loads(subprocess.check_output([binary,'status','--json'],env=env))
    server = subprocess.Popen([binary,'console','--no-sign-in'],env=env,stdout=subprocess.PIPE,text=True)
    try:
        origin = server.stdout.readline().strip()
        page = urllib.request.urlopen(origin,timeout=5).read().decode()
        section = page.split('data-group="ready for review"',1)[1].split('</details>',1)[0]
        top = section.split('<ul class="children">',1)[0]
        print('CLI top-level rows:', [(c['kind'], c.get('issue'), c['group']) for c in status['commands']])
        assert any(c['group']=='ready for review' and c['kind']=='Run' and c.get('issue',{}).get('number')==7 for c in status['commands']), 'Dispatched ready Run is absent as its own Ready for review row in status'
        assert 'acme/widgets#7' in top, 'Dispatched ready Run is absent as its own Ready for review row in Console'
    finally:
        server.terminate(); server.wait(timeout=5)
