"""Measure one Linux executable's peak RSS with an empty, isolated user state."""
import json
import os
from pathlib import Path
import resource
import subprocess
import sys
import tempfile


if sys.platform != "linux":
    raise SystemExit("The prototype memory comparison requires Linux (ru_maxrss is KiB).")
with tempfile.TemporaryDirectory(prefix="thirdshift-memory-") as home:
    env = dict(os.environ, HOME=home, XDG_CONFIG_HOME=home + "/config",
               XDG_STATE_HOME=home + "/state", XDG_CACHE_HOME=home + "/cache",
               THIRDSHIFT_PROTOTYPE_INBOX=home + "/inbox.json")
    args = [str(Path(sys.argv[1]).resolve()), *sys.argv[2:]]
    child = subprocess.run(args, env=env, capture_output=True, text=True)
    print(json.dumps({"command": args, "peakRssBytes": resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss * 1024,
                      "exitCode": child.returncode, "output": child.stdout, "stderr": child.stderr}))
    raise SystemExit(child.returncode)
