#!/usr/bin/env python3
"""Stopped Session evidence must be scanned when the logical Trial completes."""
import hashlib, json, os, pathlib, shutil, signal, subprocess, sys, tempfile, time
binary = pathlib.Path(sys.argv[1]).resolve()
root = pathlib.Path(tempfile.mkdtemp(prefix='1880-interrupted-'))
home, bins, repo = root/'home', root/'bin', root/'repo'
for d in (home,bins,repo,root/'tmp'): d.mkdir()
env = {'PATH':str(bins)+':'+os.environ['PATH'],'HOME':str(home),'XDG_CONFIG_HOME':str(home/'.config'),'TMPDIR':str(root/'tmp'),'GIT_CONFIG_NOSYSTEM':'1','THIRDSHIFT_TRIAL_NO_SANDBOX':'1'}
def cmd(args, cwd=repo, check=True):
    r = subprocess.run([str(x) for x in args],cwd=cwd,env=env,text=True,capture_output=True)
    if check and r.returncode: raise RuntimeError(f'{args}: {r.stdout}\n{r.stderr}')
    return r
def git(*args): return cmd(['git',*args]).stdout.strip()
def write(path, text, executable=False):
    path.parent.mkdir(parents=True,exist_ok=True); path.write_text(text)
    if executable:path.chmod(0o755)
write(home/'.gitconfig','[user]\n name = Review\n email = review@example.invalid\n[gc]\n auto = 0\n')
for name, body in {'systemctl':'case "$*" in "--user show -p SystemState --value") echo running ;; esac','systemd-run':'while [ "$#" -gt 0 ] && [ "$1" != -- ]; do shift; done\nshift\nexec "$@"','journalctl':'exit 0','claude':'echo fake-claude-1','gh':'exit 1'}.items():write(bins/name,'#!/bin/sh\n'+body+'\n',True)
git('init','--initial-branch=main')
write(repo/'README.md','fixture\n');git('add','.');git('commit','-qm','Initial fixture');base=git('rev-parse','HEAD')
git('remote','add','origin','https://github.com/acme/widgets.git')
git('checkout','-qb','candidate');write(repo/'src/prompt.rs','candidate\n');git('add','.');git('commit','-qm','Candidate');candidate=git('rev-parse','HEAD')
git('checkout','-qb','reference-fix',base);write(repo/'test_feature.py','pass\n');git('add','.');git('commit','-qm','Real fix title');fix=git('rev-parse','HEAD');git('checkout','-q','candidate')
suite=home/'.thirdshift/evals/suites/process';task=suite/'tasks/fixture'
write(suite/'suite.toml','format = 1\ntrials_per_arm = 1\n[dependencies]\nfactory_skills = []\nsession_prompts = ["src/prompt.rs"]\nharness_defaults = []\nmodel_defaults = []\neffort_defaults = []\n')
write(task/'spec.md','# Fixture\nDo the task.\n')
write(task/'task.toml',f'''format = 1
state = "approved"
session = "process"
provenance = "real"
environment = "host"
repository = {json.dumps(str(repo))}
base = "{base}"
change = "{fix}"
criteria = "Implement the task."
[inputs]
".thirdshift-issue.md" = "spec.md"
[trial]
issue_url = "https://github.com/acme/widgets/issues/7"
base = "main"
branch = "issue-7"
pr_url = "https://github.com/acme/widgets/pull/1"
[process_grading]
test_files = ["test_feature.py"]
fail_to_pass = [["true"]]
pass_to_pass = [["true"]]
''')
arm_script = '''#!/usr/bin/env python3
import json, pathlib, sys, time
request=json.load(open(sys.argv[2])); out=pathlib.Path(request['output_dir']);out.mkdir(parents=True,exist_ok=True)
root=pathlib.Path(ROOT)
if not (root/'resume').exists():
    (out/'trial-process.jsonl').write_text(json.dumps({'type':'assistant','message':FIX})+'\\n')
    (root/('started-'+('baseline' if '--baseline--' in str(out) else 'candidate'))).touch()
    while True:time.sleep(1)
(out/'trial-process.jsonl').write_text(json.dumps({'type':'assistant','message':'No reference here'})+'\\n')
print(json.dumps({'version':1,'outcome':'delivered','timings':{'total_ms':1,'session_ms':1},'void_cause':None,'cause':None,'session_id':None,'log':str(out/'trial-process.jsonl'),'model_evidence':None,'reports':{'standards':None,'spec':None}}))
'''.replace('ROOT',repr(str(root))).replace('FIX',repr(fix))
for commit in (base,candidate):write(home/f'.thirdshift/evals/builds/{commit}/thirdshift',arm_script,True)
args=[str(binary),'experiment','candidate','--json','--yes','--min-discriminating-tasks','1','--void-retries','0']
log=open(root/'initial.stdout','w');err=open(root/'initial.stderr','w')
p=subprocess.Popen(args,cwd=repo,env=env,stdout=log,stderr=err)
try:
    end=time.monotonic()+30
    while not all((root/('started-'+a)).exists() for a in ('baseline','candidate')):
        if p.poll() is not None:raise RuntimeError((root/'initial.stderr').read_text())
        if time.monotonic()>end:raise RuntimeError('start timeout '+(root/'initial.stderr').read_text())
        time.sleep(.02)
    p.send_signal(signal.SIGTERM);p.wait(timeout=30)
finally:
    if p.poll() is None:p.kill();p.wait()
    log.close();err.close()
(root/'resume').touch()
r=cmd(args);view=json.loads(r.stdout)
evidence=[]
for session in view['sessions']:
    selected=cmd([binary,'experiments',view['id'],'--session',session['id']])
    if fix in selected.stdout:evidence.append(session['id'])
print(json.dumps({'root':str(root),'state':view['state'],'verdict':view['verdict']['verdict'],'outcomes':[r['outcome'] for r in view['results']],'leak_hits':view['leak_hits'],'retained_leaking_sessions':evidence},indent=2))
assert evidence, 'fixture failed to retain a leaking Session'
assert any(hit['identifier']==fix and hit['decision'] is None for hit in view['leak_hits']), 'retained leaking Session evidence has no pending hit after Stop and resume'
assert view['verdict']['verdict'] == 'waiting_on_leaks', 'retained hits must hold the Verdict'
shutil.rmtree(root)
