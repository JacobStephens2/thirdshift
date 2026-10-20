import datetime as dt
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import urllib.request
from html.parser import HTMLParser

class Tables(HTMLParser):
    def __init__(self):
        super().__init__(); self.tables=[]; self.table=None; self.in_cell=False; self.text=''
    def handle_starttag(self, tag, attrs):
        if tag=='table': self.table=[]
        if tag=='tr' and self.table is not None: self.row=[]
        if tag in ('th','td') and self.table is not None: self.in_cell=True; self.text=''
    def handle_data(self, data):
        if self.in_cell: self.text+=data
    def handle_endtag(self, tag):
        if tag in ('th','td') and self.in_cell: self.row.append(self.text); self.in_cell=False
        if tag=='tr' and self.table is not None: self.table.append(self.row)
        if tag=='table': self.tables.append(self.table); self.table=None

binary=str(Path(sys.argv[1] if len(sys.argv)>1 else 'target/debug/thirdshift').resolve())
case=sys.argv[2] if len(sys.argv)>2 else 'ordering'
with tempfile.TemporaryDirectory(prefix='issue-826-review-') as home:
    root=Path(home)/'.thirdshift'; root.mkdir(); (root/'config.toml').write_text('[console]\nport = 0\n')
    now=dt.datetime.now(dt.timezone.utc)
    stamp=lambda mins:(now-dt.timedelta(minutes=mins)).isoformat()
    fields=Path('/proc/self/stat').read_text().rsplit(') ',1)[1].split()
    identity=Path('/proc/sys/kernel/random/boot_id').read_text().strip()+':'+fields[19]
    def record(repository, number, live):
        folder=root/'logs'/repository/'commands/issue'; folder.mkdir(parents=True,exist_ok=True)
        value=dict(version=1,kind='Run',name='run-'+str(number),repository=repository,hostname='fixture',
            issue=dict(number=number,url=f'https://github.com/{repository}/issues/{number}',title='fixture'),
            process_id=os.getpid() if live else 0,process_start=identity if live else 'gone',harness='codex',
            started_at=stamp(30),ended_at=None if live else stamp(10),outcome=None if live else 'merged',session_log=None,
            stages=[dict(name='implement',began_at=stamp(30),session_log=None)])
        (folder/f'{number}-fixture.command.json').write_text(json.dumps(value))
    record('acme/a',1,False); record('zz/z',2,True)
    old=root/'logs/acme/a/activity.log'
    old.write_text('2026-10-01 12:00:00 Run #7 started: commands/issue/7-20261001T120000+0000.log\n2026-10-01 12:20:00 Run #7 ended: merged\n')
    env=dict(os.environ,HOME=home)
    status=json.loads(subprocess.check_output([binary,'status','--all','--json'],env=env))
    server=subprocess.Popen([binary,'console','--no-sign-in'],env=env,stdout=subprocess.PIPE,text=True)
    try:
        origin=server.stdout.readline().strip()
        html=urllib.request.urlopen(origin+'/stages',timeout=5).read().decode()
        parser=Tables(); parser.feed(html)
        if case=='ordering':
            expected=[f'{c["repository"]}#{c["issue"]["number"]}' for c in status['commands'] if c['state']=='live']
            headers=[cell for table in parser.tables for cell in table[0][1:]]
            assert headers[0].startswith(expected[0]),f'Live Runs should be pinned first; got columns {headers}; live={expected}'
        elif case=='history':
            history=next(c for c in status['commands'] if c['repository']=='acme/a' and c['issue']['number']==7)
            assert history['stages']==[] and history['source']=='activity_log'
            table=next(t for t in parser.tables if any('acme/a#7' in h for h in t[0]))
            col=next(i for i,h in enumerate(table[0]) if 'acme/a#7' in h)
            cell=table[1][col]
            assert 'Not reached' not in cell,f'No stage metadata was retained for the merged historical Run; grid invents a state: {cell}'
        else: raise ValueError(case)
    finally:
        server.terminate(); server.wait(timeout=5)
