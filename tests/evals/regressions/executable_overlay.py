#!/usr/bin/env python3
import json
import os
from pathlib import Path
import subprocess
import tempfile

binary = Path(os.environ.get('THIRDSHIFT_TEST_BINARY', 'target/debug/thirdshift')).resolve()
with tempfile.TemporaryDirectory(prefix='spec-executable-overlay-') as temporary:
    root = Path(temporary)
    source = root / 'source'
    source.mkdir()
    home = root / 'home'
    env = dict(os.environ, HOME=str(home), GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')
    def git(*args):
        return subprocess.check_output(['git', '-c', 'user.name=Fixture', '-c',
            'user.email=fixture@example.invalid', *args], cwd=source, env=env, text=True).strip()
    git('init', '-q')
    (source / 'src').mkdir()
    (source / 'src/counter.py').write_text('COUNT = 1\n')
    git('add', '.')
    git('commit', '-qm', 'Base')
    base = git('rev-parse', 'HEAD')
    (source / 'src/counter.py').write_text('COUNT = 2\n')
    (source / 'tests').mkdir()
    regression = source / 'tests/regression.py'
    regression.write_text('#!/usr/bin/env python3\nfrom pathlib import Path\nassert Path("src/counter.py").read_text() == "COUNT = 2\\n"\n')
    regression.chmod(0o755)
    subprocess.run(['./tests/regression.py'], cwd=source, env=env, check=True)
    git('add', '.')
    git('commit', '-qm', 'Fix and executable separate regression')
    change = git('rev-parse', 'HEAD')
    suite = home / '.thirdshift/evals/suites/process'
    task = suite / 'tasks/executable'
    task.mkdir(parents=True)
    (suite / 'suite.toml').write_text('format = 1\ntrials_per_arm = 3\n[dependencies]\nfactory_skills = []\nsession_prompts = []\nharness_defaults = []\nmodel_defaults = []\neffort_defaults = []\n')
    (task / 'task.toml').write_text(f'''format = 1
state = "approved"
session = "process"
provenance = "real"
environment = "host"
repository = {json.dumps(str(source))}
base = "{base}"
change = "{change}"
criteria = "Fix the counter."
checks = []
[inputs]
[process_grading]
test_files = ["tests/regression.py"]
test_fakes = []
fail_to_pass = [["./tests/regression.py"]]
pass_to_pass = [["./tests/regression.py"]]
''')
    result = subprocess.run([str(binary), 'evals', 'prepare', 'process', 'executable',
        str(root / 'prepared')], env=env, text=True, capture_output=True)
    assert result.returncode == 0, result.stderr
