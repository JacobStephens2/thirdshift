#!/usr/bin/env bash
# Run with: bash scripts/check-vault-codex.sh
# Optional Codex flags follow the script name, e.g. --no-daemon.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd -- "$script_dir/.."

prompt='Test whether the credentials injected by vaulted-agent reach your shell tools.
Run exactly this command once through your shell tool:

bash scripts/check-vault-env.sh

Show the complete status-only output and the command exit code. Exit 0 means all
manifest variables are non-empty; exit 1 means some are empty or missing; exit 2
means the manifest could not be checked. Treat a nonzero exit as a test result.
Do not print credential values, dump the environment, fetch credentials, modify
files or configuration, or try to fix the result. Finish after reporting it.'

exec va codex --yolo "$@" "$prompt"
