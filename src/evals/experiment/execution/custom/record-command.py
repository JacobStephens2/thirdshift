"""Sandbox client: preserve Harness stdio and send live records to the host."""
import argparse
import json
import pathlib
import socket
import subprocess
import sys

parser = argparse.ArgumentParser()
parser.add_argument("socket")
parser.add_argument("--role", required=True)
separator = sys.argv.index("--") if "--" in sys.argv else len(sys.argv)
args = parser.parse_args(sys.argv[1:separator])
command = sys.argv[separator + 1:]
if not command:
    parser.error("expected a Harness command after --")
with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
    connection.connect(args.socket)

    def send(value):
        connection.sendall((json.dumps(value) + "\n").encode())

    send(dict(role=args.role, harness=pathlib.Path(command[0]).name,
              arguments=command[1:]))
    with subprocess.Popen(command, stdout=subprocess.PIPE) as child:
        for raw in child.stdout:
            send(dict(raw=raw.decode("utf-8", errors="replace")))
            sys.stdout.buffer.write(raw)
            sys.stdout.buffer.flush()
        code = child.wait()
    send(dict(error=None if code == 0 else f"Harness exited {code}"))
sys.exit(code if code >= 0 else 128 - code)
