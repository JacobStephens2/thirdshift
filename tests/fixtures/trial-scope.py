"""Exercise real capped Trials; cleanup names only each fixture's unique unit."""

import concurrent.futures
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import uuid


HARNESS = r'''#!/usr/bin/python3
import json, os, pathlib, sys, time
if '--version' in sys.argv:
    print('fixture-harness-1.0')
    raise SystemExit(0)
root = pathlib.Path.cwd()
number = len(list(root.glob('observed-*.json')))
for old in root.glob('pid-*'):
    stat = pathlib.Path('/proc') / old.read_text().strip() / 'stat'
    assert not stat.exists() or stat.read_text().split(') ')[1].startswith('Z'), 'previous session left detached work alive'
group = next(line.split(':', 2)[2] for line in pathlib.Path('/proc/self/cgroup').read_text().splitlines() if line.startswith('0::'))
limit = (pathlib.Path('/sys/fs/cgroup') / group.lstrip('/') / 'memory.max').read_text().strip()
(root / f'observed-{number}.json').write_text(json.dumps(dict(group=group, limit=limit)))
(root / 'supervisor').write_text(str(os.getppid()))
child = os.fork()
if child == 0:
    os.setsid()
    if os.fork() != 0:
        os._exit(0)
    for fd in range(3):
        null = os.open('/dev/null', os.O_RDWR)
        os.dup2(null, fd)
        if null > 2:
            os.close(null)
    os.execve('/bin/sh', ['sh', '-c', f'echo $$ > pid-{number}; exec sleep 600'], {})
os.waitpid(child, 0)
deadline = time.monotonic() + 5
while not (root / f'pid-{number}').exists():
    assert time.monotonic() < deadline, 'detached fixture never became ready'
    time.sleep(0.005)
mode = (root / 'mode').read_text()
if mode == 'interrupt':
    time.sleep(600)
if mode == 'resume':
    print(json.dumps(dict(type='system', subtype='init', session_id=f'fixture-{number}')))
    print(json.dumps(dict(type='system', subtype='task_started', task_id='background', description='sleep 600')))
print('{"type":"result","subtype":"success","result":"Done"}')
if mode == 'resume':
    print(json.dumps(dict(type='system', subtype='task_updated', task_id='background', patch=dict(status='killed'))))
'''


def running(pid):
    try:
        return not (Path('/proc') / str(pid) / 'stat').read_text().split(') ')[1].startswith('Z')
    except FileNotFoundError:
        return False


def exercise(binary, environment, mode):
    unit = 'thirdshift-trial-fixture-' + uuid.uuid4().hex + '.scope'
    with tempfile.TemporaryDirectory(prefix='thirdshift-trial-scope-') as directory:
        root = Path(directory)
        repo = root / 'repo'
        repo.mkdir()
        subprocess.run(['git', 'init', '-q', str(repo)], check=True)
        (repo / '.thirdshift-issue.md').write_text('# Review fixture\n')
        (repo / 'mode').write_text(mode)
        (root / 'home').mkdir()
        (root / 'bin').mkdir()
        harness = root / 'bin' / 'claude'
        harness.write_text(HARNESS)
        harness.chmod(0o755)
        request = root / 'request.json'
        request.write_text(json.dumps(dict(
            version=1, repository=str(repo), session_kind='code-review',
            inputs=dict(issue_url='https://github.com/acme/widgets/issues/7', base='main', branch='issue-7', pr_url='https://github.com/acme/widgets/pull/8'),
            harness='claude', model=None, effort=None, output_dir=str(root / 'output'),
        )))
        env = dict(environment, HOME=str(root / 'home'), PATH=str(root / 'bin') + os.pathsep + environment['PATH'],
                   THIRDSHIFT_TRIAL_NO_SANDBOX='1', THIRDSHIFT_TRIAL_SCOPE=unit,
                   GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1')
        command = ['systemd-run', '--user', '--scope', '--quiet', '--collect', '--unit=' + unit,
                   '--property=MemoryMax=128M', '--', binary, '--trial', str(request)]
        child = subprocess.Popen(command, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            if mode == 'interrupt':
                deadline = time.monotonic() + 10
                while not (repo / 'pid-0').exists():
                    assert child.poll() is None, 'Trial ended before detached work started'
                    assert time.monotonic() < deadline, 'Trial fixture never became ready'
                    time.sleep(0.01)
                os.kill(int((repo / 'supervisor').read_text()), signal.SIGINT)
            stdout, stderr = child.communicate(timeout=30)
            assert child.returncode == 0, (stdout, stderr)
            result = json.loads(stdout)
            assert result['outcome'] == ('void' if mode == 'interrupt' else 'succeeded'), (result, stderr)
            if mode == 'interrupt':
                assert result['void_cause'] == 'interrupted', result
            observations = [json.loads(path.read_text()) for path in repo.glob('observed-*.json')]
            assert len(observations) == (2 if mode == 'resume' else 1), (observations, stderr)
            for observed in observations:
                assert observed['group'].endswith('/' + unit), f'Trial Harness moved out of its dedicated scope: {observed}'
                assert observed['limit'] != 'max' and int(observed['limit']) <= 128 * 1024 * 1024, f'Trial Harness escaped its 128M cap: {observed}'
            for path in repo.glob('pid-*'):
                assert not running(int(path.read_text())), f'detached process survived {mode}: {path.read_text()}'
            if mode != 'interrupt':
                trace = Path(result['log']).read_text()
                assert 'fixture-harness-1.0' in trace, 'borrowed cleanup lost Harness version evidence'
            print(f'{mode}: capped, detached work stopped, supervisor and version evidence preserved')
        finally:
            subprocess.run(['systemctl', '--user', 'stop', unit], env=env, capture_output=True, timeout=15)
            if child.poll() is None:
                child.kill()
            child.communicate(timeout=10)


if __name__ == '__main__':
    environment = os.environ.copy()
    environment['XDG_RUNTIME_DIR'] = environment.get('XDG_RUNTIME_DIR') or f'/run/user/{os.geteuid()}'
    probe = subprocess.run(['systemctl', '--user', 'show', '-p', 'SystemState', '--value'], env=environment, capture_output=True)
    if probe.returncode != 0:
        sys.exit(77)
    binary = str(Path(sys.argv[1]).resolve())
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as workers:
        list(workers.map(lambda mode: exercise(binary, environment, mode), ['normal', 'normal']))
    for mode in ['resume', 'interrupt']:
        exercise(binary, environment, mode)
