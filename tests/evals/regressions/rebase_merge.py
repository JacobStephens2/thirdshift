#!/usr/bin/env python3
import json
import os
from pathlib import Path
import subprocess
import tempfile

binary = Path(os.environ.get('THIRDSHIFT_TEST_BINARY', 'target/debug/thirdshift')).resolve()
with tempfile.TemporaryDirectory(prefix='spec-rebase-merge-') as temporary:
    root = Path(temporary)
    source = root / 'source'
    source.mkdir()
    home = root / 'home'
    env = dict(os.environ, HOME=str(home), GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')
    def git(*args, extra_env=None):
        return subprocess.check_output(['git', '-c', 'user.name=Fixture', '-c',
            'user.email=fixture@example.invalid', *args], cwd=source,
            env=dict(env, **(extra_env or {})), text=True).strip()
    git('init', '-q', '-b', 'main')
    git('remote', 'add', 'origin', 'https://github.com/acme/widgets.git')
    (source / 'src').mkdir()
    (source / 'src/counter.py').write_text('COUNT = 1\n')
    git('add', '.')
    git('commit', '-qm', 'Base')
    base = git('rev-parse', 'HEAD')
    git('checkout', '-q', '-b', 'issue-7')
    (source / 'src/counter.py').write_text('COUNT = 2\n')
    git('commit', '-qam', 'Implement Ticket')
    first_pr_commit = git('rev-parse', 'HEAD')
    (source / 'tests').mkdir()
    (source / 'tests/regression.py').write_text('from pathlib import Path\nassert Path("src/counter.py").read_text() == "COUNT = 2\\n"\n')
    git('add', '.')
    git('commit', '-qm', 'Add separate regression test')
    original_head = git('rev-parse', 'HEAD')
    # GitHub rebase-merged both commits, updating their committer timestamps.
    # Its mergeCommit points at the final rebased commit, and commits/HEAD
    # still describe the original pull-request branch.
    git('checkout', '-q', 'main')
    git('cherry-pick', first_pr_commit, original_head,
        extra_env={'GIT_COMMITTER_DATE': '2001-01-01T00:00:00Z'})
    merge = git('rev-parse', 'HEAD')
    subprocess.run(['python3', 'tests/regression.py'], cwd=source, env=env, check=True)
    ticket = {'url': 'https://github.com/acme/widgets/issues/7', 'title': 'Fix counter',
        'body': 'Count twice.', 'labels': {'nodes': [{'name': 'bug'}],
        'pageInfo': {'hasNextPage': False}}, 'subIssues': {'totalCount': 0}}
    candidate = {'number': 17, 'url': 'https://github.com/acme/widgets/pull/17',
        'baseRefName': 'main', 'headRefName': 'issue-7', 'headRefOid': original_head,
        'mergeCommit': {'oid': merge},
        'commits': {'totalCount': 2, 'nodes': [{'commit': {'oid': first_pr_commit,
            'parents': {'nodes': [{'oid': base}]}}}]},
        'closingIssuesReferences': {'nodes': [ticket], 'pageInfo': {'hasNextPage': False}}}
    response = root / 'github.json'
    response.write_text(json.dumps({'data': {'repository': {'pullRequests': {'nodes': [candidate]}}}}))
    bin_directory = root / 'bin'
    bin_directory.mkdir()
    gh = bin_directory / 'gh'
    gh.write_text('#!/usr/bin/env python3\nimport os\nprint(open(os.environ["FIXTURE_GITHUB"]).read())\n')
    gh.chmod(0o755)
    env.update(PATH=str(bin_directory) + os.pathsep + os.environ['PATH'], FIXTURE_GITHUB=str(response))
    result = subprocess.run([str(binary), 'evals', 'propose-process', str(source)],
        env=env, text=True, capture_output=True)
    assert result.returncode == 0, result.stderr
    report = json.loads(result.stdout)
    assert len(report['proposals']) == 1, report
    name = report['proposals'][0]['task']
    manifest = home / '.thirdshift/evals/suites/process/tasks' / name / 'task.toml'
    assert f'base = "{base}"' in manifest.read_text(), manifest.read_text()
