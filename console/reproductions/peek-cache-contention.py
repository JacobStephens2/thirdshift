#!/usr/bin/env python3
"""After cargo build, check cached details during another Command's cold parse.

Linux only: IN_OPEN pins the overlapping requests to actual Session trace IO.
The 128 MiB OTLP fixture makes lock contention visible even on a fast machine.
"""

import concurrent.futures
import ctypes
import json
import os
import pathlib
import select
import subprocess
import tempfile
import time
import urllib.request


def check():
    binary = str(pathlib.Path(__file__).resolve().parents[2] / 'target/debug/thirdshift')
    with tempfile.TemporaryDirectory(prefix='thirdshift-peek-contention-') as directory:
        task_home = pathlib.Path(directory)
        config = task_home / '.thirdshift'
        records = config / 'logs/acme/widgets/commands/issue'
        records.mkdir(parents=True)
        (config / 'config.toml').write_text('[console]\nport = 0\n')
        for number in [1, 2]:
            trace = records / f'{number}-fixture.logs.jsonl'
            if number == 1:
                trace.write_text('{"resourceLogs":[]}\n')
            else:
                line = json.dumps({'resourceLogs': [{'scopeLogs': [{'logRecords': [
                    {'body': {'stringValue': 'x' * (8 * 1024 * 1024)}}
                ]}]}]}) + '\n'
                with trace.open('w') as output:
                    for _ in range(16):
                        output.write(line)
            record = dict(
                version=1, kind='Run', repository='acme/widgets',
                name=f'acme/widgets#{number}',
                issue=dict(number=number, url=f'https://github.com/acme/widgets/issues/{number}', title='Widgets'),
                hostname='factory', process_id=0, process_start='gone', harness='claude',
                started_at='2026-10-19T11:00:00-04:00', ended_at='2026-10-19T11:01:00-04:00',
                stages=[dict(name='implement', began_at='2026-10-19T11:00:00-04:00', session_log=str(trace))],
                outcome='merged',
            )
            (records / f'{number}-fixture.command.json').write_text(json.dumps(record))
        env = dict(os.environ, HOME=directory)
        server = subprocess.Popen(
            [binary, 'console', '--no-sign-in'], env=env,
            stdout=subprocess.PIPE, text=True,
        )
        try:
            assert select.select([server.stdout], [], [], 10)[0], 'Console startup timed out'
            origin = server.stdout.readline().strip()
            assert origin.startswith('http://127.0.0.1:'), origin

            def peek(number):
                began = time.monotonic()
                with urllib.request.urlopen(
                    origin + '/peek?command=widgets%23' + str(number), timeout=45,
                ) as response:
                    assert response.status == 200
                    response.read()
                return time.monotonic() - began

            print('warm selected details seconds', peek(1), flush=True)
            libc = ctypes.CDLL(None, use_errno=True)
            descriptor = libc.inotify_init1(os.O_NONBLOCK | os.O_CLOEXEC)
            assert descriptor >= 0
            try:
                watched = libc.inotify_add_watch(
                    descriptor, os.fsencode(records / '2-fixture.logs.jsonl'), 0x20,  # IN_OPEN
                )
                assert watched >= 0
                with concurrent.futures.ThreadPoolExecutor() as executor:
                    cold = executor.submit(peek, 2)
                    assert select.select([descriptor], [], [], 15)[0], 'cold reader did not open trace'
                    os.read(descriptor, 8192)
                    warm_seconds = peek(1)
                    cold_seconds = cold.result(timeout=45)
                    print('cold details seconds', cold_seconds, flush=True)
                    print('cached unrelated details seconds', warm_seconds, flush=True)
                    assert warm_seconds < 1, f'cached details took {warm_seconds:.3f}s during an unrelated parse'
            finally:
                os.close(descriptor)
        finally:
            server.terminate()
            try:
                server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait()
            server.stdout.close()


if __name__ == '__main__':
    check()
