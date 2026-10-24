"""Retained Spec-review reproduction: measured spans survive a died Command."""
import json, os, pathlib, subprocess, tempfile, urllib.request

binary = pathlib.Path('target/debug/thirdshift').resolve()
with tempfile.TemporaryDirectory(prefix='989-spec-died-') as temp:
    home = pathlib.Path(temp)
    folder = home / '.thirdshift/logs/acme/widgets/commands/issue'
    folder.mkdir(parents=True)
    (home / '.thirdshift/config.toml').write_text('[console]\nport = 0\n')
    session = folder / '7-20261001T120000+0000-implement.logs.jsonl'
    session.write_text(pathlib.Path('tests/fixtures/waterfall-logs.jsonl').read_text())
    session.with_name(session.name.replace('.logs.jsonl', '.spans.jsonl')).write_text(
        pathlib.Path('tests/fixtures/waterfall-spans.jsonl').read_text()
    )
    record = dict(
        version=1, kind='Run', repository='acme/widgets', name='run-7',
        issue=dict(number=7, url='https://github.com/acme/widgets/issues/7', title='Widgets'),
        hostname='fixture', process_id=0, process_start='gone', harness='muse',
        started_at='2026-10-01T12:00:00+00:00', ended_at=None,
        stages=[dict(name='implement', began_at='2026-10-01T12:00:00+00:00', session_log=str(session))],
        session_log=str(session), outcome=None
    )
    (folder / '7-fixture.command.json').write_text(json.dumps(record))
    (folder / '7-fixture.log').write_text('thirdshift: 12:00:00 starting\n')
    server = subprocess.Popen(
        [str(binary), 'console', '--no-sign-in'],
        env={**os.environ, 'HOME': str(home)},
        stdout=subprocess.PIPE, text=True
    )
    try:
        origin = server.stdout.readline().strip()
        tree = json.load(urllib.request.urlopen(origin + '/spans?command=widgets%237'))
        tools = [span for span in tree['spans'] if span['kind'] == 'tool' and span['end'] is not None]
        print('Retained tool timing:', tools)
        assert len(tools) == 2 and all(span['end'] > span['start'] for span in tools), 'Ended tool spans must retain visible intervals for a died Command'
        # console/tests/waterfall.test.cjs also checks the rendered bar widths.
    finally:
        server.terminate()
        server.wait(timeout=10)
