"""Temporary diagnostic for #894: where macOS test time goes.

Runs each named test alone, straight from its test binary, and reports wall
time against the CPU time of the test and every process it reaped. With
`--dtrace`, it also counts exec'd programs and sums their lifetimes, and
sums the time each program spent asleep.
"""

import json
import os
import resource
import subprocess
import sys
import time

DTRACE = r"""
proc:::exec-success { start[pid] = timestamp; }
proc:::exit /start[pid]/ {
  @count[execname] = count();
  @life[execname] = sum(timestamp - start[pid]);
  start[pid] = 0;
}
syscall::__semwait_signal:entry, syscall::__semwait_signal_nocancel:entry { self->s = timestamp; }
syscall::__semwait_signal:return, syscall::__semwait_signal_nocancel:return /self->s/ {
  @sleep[execname] = sum(timestamp - self->s); self->s = 0;
}
syscall::select:entry, syscall::poll:entry, syscall::kevent:entry,
syscall::kevent64:entry, syscall::wait4:entry, syscall::read:entry { self->w = timestamp; }
syscall::select:return, syscall::poll:return, syscall::kevent:return,
syscall::kevent64:return, syscall::wait4:return, syscall::read:return /self->w/ {
  @blocked[execname, probefunc] = sum(timestamp - self->w); self->w = 0;
}
END {
  printf("== count\n"); printa("%s %@d\n", @count);
  printf("== life_ns\n"); printa("%s %@d\n", @life);
  printf("== sleep_ns\n"); printa("%s %@d\n", @sleep);
  printf("== blocked_ns\n"); printa("%s %s %@d\n", @blocked);
}
"""


def binaries():
    listed = subprocess.run(
        ["cargo", "nextest", "list", "--message-format", "json"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    suites = json.loads(listed)["rust-suites"]
    return {suite["binary-id"]: suite["binary-path"] for suite in suites.values()}


def run(binary, name, dtrace):
    trace = None
    if dtrace:
        out = f"/tmp/dtrace-{os.getpid()}.txt"
        trace = subprocess.Popen(
            ["sudo", "dtrace", "-q", "-o", out, "-n", DTRACE],
            stderr=subprocess.PIPE,
        )
        time.sleep(3)
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    started = time.monotonic()
    test = subprocess.run(
        [binary, "--exact", name, "--test-threads", "1"],
        capture_output=True,
        text=True,
    )
    wall = time.monotonic() - started
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    result = {
        "wall": round(wall, 2),
        "user": round(after.ru_utime - before.ru_utime, 2),
        "sys": round(after.ru_stime - before.ru_stime, 2),
        "passed": test.returncode == 0,
    }
    if trace:
        subprocess.run(["sudo", "pkill", "-INT", "-x", "dtrace"])
        _, err = trace.communicate()
        result["dtrace_stderr"] = err.decode()[-2000:]
        try:
            with open(out) as f:
                result["dtrace"] = f.read()
        except OSError as error:
            result["dtrace"] = str(error)
    return result


def main():
    dtrace = "--dtrace" in sys.argv
    tests = [line.split() for line in open(sys.argv[1]) if line.strip()]
    paths = binaries()
    for binary_id, name in tests:
        result = run(paths[binary_id], name, dtrace)
        result["test"] = f"{binary_id} {name}"
        print(json.dumps(result), flush=True)


main()
