"""Reproduce the missing Codex edit patch, from the Spec review of #819.

Run from the repository root: python3 console/reproductions/codex-edit-diff.py
This intentionally fails until patch evidence is available in retained logs.
The Codex FileUpdateChange schema contains only path and kind:
https://github.com/openai/codex/blob/main/sdk/typescript/src/items.ts
"""
import html
import json
import os
import pathlib
import subprocess
import tempfile
import urllib.request

binary = str(pathlib.Path("target/debug/thirdshift").resolve())
with tempfile.TemporaryDirectory() as home:
    root = pathlib.Path(home) / ".thirdshift"
    folder = root / "logs/acme/widgets/commands/issue"
    folder.mkdir(parents=True)
    (root / "config.toml").write_text("[console]\nport = 0\n")
    event = {
        "type": "item.completed",
        "item": {
            "id": "f1",
            "type": "file_change",
            "changes": [{"path": "widgets.rs", "kind": "update"}],
            "status": "completed",
        },
    }
    (folder / "7-20261003T120000-0400-implement.jsonl").write_text(json.dumps(event) + "\n")
    env = dict(os.environ, HOME=home)
    listed = json.loads(subprocess.check_output([binary, "logs", "--json"], env=env))
    server = subprocess.Popen(
        [binary, "console", "--no-sign-in"], env=env, stdout=subprocess.PIPE, text=True
    )
    try:
        origin = server.stdout.readline().strip()
        with urllib.request.urlopen(origin + "/logs/" + listed["logs"][0]["id"], timeout=5) as response:
            page = html.unescape(response.read().decode())
        assert "--- a/widgets.rs" in page and "+++ b/widgets.rs" in page and "@@" in page, page
    finally:
        server.terminate()
        server.wait(timeout=5)
