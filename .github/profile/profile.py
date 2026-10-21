"""Temporary diagnostic for #894: where macOS test time goes.

Runs each named test alone, straight from its test binary, and reports wall
time against the CPU time of the test and every process it reaped. With
`--dtrace`, it also counts the programs the test exec'd and sums their
lifetimes, their time in fsync, and their time asleep, and samples which
programs are on a CPU, system-wide.
"""

import json
import os
import resource
import subprocess
import sys
import time

DTRACE = r"""
proc:::exec-success /progenyof($1)/ { start[pid] = timestamp; }
proc:::exit /start[pid]/ {
  @count[execname] = count();
  @life[execname] = sum(timestamp - start[pid]);
  start[pid] = 0;
}
syscall::fsync:entry, syscall::fsync_nocancel:entry /progenyof($1)/ { self->f = timestamp; }
syscall::fcntl:entry, syscall::fcntl_nocancel:entry /progenyof($1) && (arg1 == 51 || arg1 == 85)/ { self->f = timestamp; }
syscall::fsync:return, syscall::fsync_nocancel:return, syscall::fcntl:return,
syscall::fcntl_nocancel:return /self->f/ {
  @fsync[execname] = sum(timestamp - self->f); @fsyncs[execname] = count(); self->f = 0;
}
syscall::__semwait_signal:entry, syscall::__semwait_signal_nocancel:entry /progenyof($1)/ { self->s = timestamp; }
syscall::__semwait_signal:return, syscall::__semwait_signal_nocancel:return /self->s/ {
  @sleep[execname] = sum(timestamp - self->s); self->s = 0;
}
profile-997 { @cpu[execname] = count(); }
profile-997 /arg0/ { @kernel = count(); }
END {
  printf("== count\n"); printa("%s\t%@d\n", @count);
  printf("== life_ns\n"); printa("%s\t%@d\n", @life);
  printf("== fsync_ns\n"); printa("%s\t%@d\n", @fsync);
  printf("== fsyncs\n"); printa("%s\t%@d\n", @fsyncs);
  printf("== sleep_ns\n"); printa("%s\t%@d\n", @sleep);
  printf("== cpu_samples\n"); printa("%s\t%@d\n", @cpu);
  printf("== kernel_samples\n"); printa("kernel\t%@d\n", @kernel);
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


def run(binary, name, dtrace, index):
    trace = None
    if dtrace:
        out = f"/tmp/dtrace-{index}.txt"
        trace = subprocess.Popen(
            ["sudo", "dtrace", "-q", "-b", "64m", "-x", "dynvarsize=64m", "-o", out, "-n", DTRACE, str(os.getpid())],
            stderr=subprocess.PIPE,
        )
        time.sleep(5)
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
    if not result["passed"]:
        result["output"] = (test.stdout + test.stderr)[-3000:]
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
    for index, (binary_id, name) in enumerate(tests):
        result = run(paths[binary_id], name, dtrace, index)
        result["test"] = f"{binary_id} {name}"
        result["git"] = subprocess.run(["git", "--version"], capture_output=True, text=True).stdout.strip()
        result["fsync_off"] = "PROFILE_GIT_FSYNC_OFF" in os.environ
        print(json.dumps(result), flush=True)


main()
