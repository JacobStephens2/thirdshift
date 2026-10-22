#!/usr/bin/env python3
"""Focused review reproductions. Run against the freshly built reviewed binary."""
import json
import os
import re
from pathlib import Path
import select
import subprocess
import sys
import tempfile
import urllib.request

BINARY = Path(sys.argv[2] if len(sys.argv) > 2 else 'target/debug/thirdshift').resolve()


def trace_line(harness, raw, event):
    return json.dumps({'resourceLogs': [{
        'resource': {'attributes': [{'key': 'thirdshift.harness', 'value': {'stringValue': harness}}]},
        'scopeLogs': [{'logRecords': [{
            'body': {'stringValue': json.dumps(raw)},
            'attributes': [{'key': 'thirdshift.events', 'value': {'stringValue': json.dumps([event])}}],
        }]}],
    }]}) + '\n'


class ConsoleFixture:
    def __enter__(self):
        self.home = tempfile.TemporaryDirectory(prefix='thirdshift-spec-review-')
        home = Path(self.home.name)
        config = home / '.thirdshift'
        config.mkdir()
        (config / 'config.toml').write_text('[console]\nport = 0\ntheme = "focus-auto"\n')
        self.folder = config / 'logs/acme/widgets/commands/issue'
        self.folder.mkdir(parents=True)
        env = dict(os.environ, HOME=str(home))
        env.pop('RESEND_API_KEY', None)
        self.process = subprocess.Popen([str(BINARY), 'console', '--no-sign-in'],
                                        env=env, stdout=subprocess.PIPE, text=True)
        if not select.select([self.process.stdout], [], [], 10)[0]:
            self.__exit__(None, None, None)
            raise AssertionError('Console did not print its address within ten seconds')
        self.address = self.process.stdout.readline().strip()
        assert self.address.startswith('http://127.0.0.1:'), self.address
        return self

    def __exit__(self, *unused):
        self.process.terminate()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        self.process.stdout.close()
        self.home.cleanup()

    def get(self, route):
        with urllib.request.urlopen(self.address + route, timeout=5) as response:
            assert response.status == 200
            return response.read().decode()

    def read(self, harness, content, trace=True):
        suffix = 'logs.jsonl' if trace else 'jsonl'
        name = f'7-20261003T120000-0400-{harness}.{suffix}'
        path = self.folder / name
        path.write_text(content)
        try:
            inventory = self.get('/logs')
            for fragment in inventory.split('href="')[1:]:
                href, label = fragment.split('"', 1)
                if label.split('</a>', 1)[0].endswith(name):
                    return self.get(href)
            raise AssertionError(f'No reader link for {name}: {inventory}')
        finally:
            path.unlink()


def snapshots():
    failures = []
    with ConsoleFixture() as console:
        for harness in ('muse', 'opencode'):
            rows = []
            for index, text in enumerate(('Checking ', 'Checking widgets.')):
                if harness == 'muse':
                    raw = {'payload_type': 'run.output.delta', 'payload': {
                        'run_stream': {'id': 'run1'},
                        'text': 'Checking ' if index == 0 else 'widgets.',
                    }}
                    message_id = 'run1:reply:message'
                else:
                    raw = {'type': 'text', 'sessionID': 's', 'part': {
                        'id': 'p', 'messageID': 'm', 'text': text,
                    }}
                    message_id = 's:m:message'
                rows.append(trace_line(harness, raw, {
                    'type': 'message', 'id': message_id, 'parent_id': None,
                    'role': 'assistant', 'text': text,
                }))
            page = console.read(harness, ''.join(rows))
            stale = '<h3>assistant</h3><pre>Checking </pre>' in page
            current = '<h3>assistant</h3><pre>Checking widgets.</pre>' in page
            count = len(re.findall(r'<article\b[^>]*class="message"', page))
            print(f'{harness}: stale snapshot visible={stale}, current snapshot visible={current}, message count={count}')
            if stale or not current or count != 1:
                failures.append(harness)
    assert not failures, f'Each stable message ID must render one current message: {failures}'


def reasoning():
    raw = {'type': 'item.completed', 'item': {
        'id': 'r1', 'type': 'reasoning', 'text': 'Consider a safe edit.',
    }}
    expected = '<details class="thinking"><summary>Thinking</summary><pre>Consider a safe edit.</pre></details>'
    normalized = {'type': 'message', 'role': 'assistant', 'text': 'Consider a safe edit.', 'parent_id': None}
    with ConsoleFixture() as console:
        legacy_page = console.read('codex', json.dumps(raw) + '\n', trace=False)
        trace_page = console.read('codex', trace_line('codex', raw, normalized))
    assert expected in legacy_page, 'Legacy baseline did not render the expected Thinking block'
    print(f'Legacy Thinking block present={expected in legacy_page}; normalized trace Thinking block present={expected in trace_page}')
    assert expected in trace_page, 'Codex reasoning in a Session trace must retain its Thinking rendering'
    assert '<h3>assistant</h3><pre>Consider a safe edit.</pre>' not in trace_page


if __name__ == '__main__':
    {'snapshots': snapshots, 'reasoning': reasoning}[sys.argv[1]]()

