import json, os, pathlib, subprocess, tempfile, urllib.request

binary = pathlib.Path(__file__).resolve().parents[2] / 'target/debug/thirdshift'
with tempfile.TemporaryDirectory(prefix='827-spec-') as temp:
    home = pathlib.Path(temp)
    folder = home / '.thirdshift/logs/acme/widgets/commands/issue'
    folder.mkdir(parents=True)
    (home / '.thirdshift/config.toml').write_text('[console]\nport = 0\n')
    session = folder / '7-20261001T120000+0000-implement.jsonl'
    session.write_text(json.dumps({'type':'item.completed','item':{'id':'a','type':'command_execution','command':'rg widgets','status':'completed'}})+'\n')
    record = dict(version=1,kind='Run',repository='acme/widgets',name='run-7',
        issue=dict(number=7,url='https://github.com/acme/widgets/issues/7',title='Widgets'),
        hostname='fixture',process_id=0,process_start='gone',harness='codex',
        started_at='2026-10-01T12:00:00+00:00',ended_at='2026-10-01T12:01:00+00:00',
        stages=[dict(name='Setup',began_at='2026-10-01T12:00:00+00:00',session_log=None),dict(name='implement',began_at='2026-10-01T12:00:10.100+00:00',session_log=str(session))],
        session_log=str(session),outcome='merged')
    (folder / '7-fixture.command.json').write_text(json.dumps(record))
    (folder / '7-fixture.log').write_text('thirdshift: 12:00:00 starting\nthirdshift: 12:00:10 implement: $ rg widgets\nthirdshift: 12:01:00 finished\n')
    server = subprocess.Popen([str(binary),'console','--no-sign-in'],env={**os.environ,'HOME':str(home)},stdout=subprocess.PIPE,text=True)
    try:
        origin = server.stdout.readline().strip()
        tree = json.load(urllib.request.urlopen(origin+'/spans?command=widgets%237'))
        spans = {span['id']: span for span in tree['spans']}
        assigned = [spans[span['parent_id']]['name'] for span in tree['spans'] if span['kind'] == 'tool']
        print('Stage containing implement tool:',assigned,flush=True)
        assert assigned == ['implement'], 'Session tool must nest under implement; whole-second clock incorrectly puts it in previous Setup stage'
    finally:
        server.terminate()
        server.wait(timeout=10)
