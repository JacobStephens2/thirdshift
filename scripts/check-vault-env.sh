#!/usr/bin/env bash
# Run with: va bash scripts/check-vault-env.sh
# An optional argument selects a different manifest. Values are never printed.
set -euo pipefail

manifest=${1:-/etc/vaulted-agent/manifests/bitwarden.refs}
if [[ ! -r "$manifest" ]]; then
  printf 'Cannot read manifest: %s\n' "$manifest" >&2
  exit 2
fi

present=0
empty=0
missing=0
while IFS= read -r line || [[ -n "$line" ]]; do
  # Read assignment names only; never source or evaluate the manifest.
  if [[ ! "$line" =~ ^[[:space:]]*([A-Za-z_][A-Za-z0-9_]*)[[:space:]]*= ]]; then
    continue
  fi
  name=${BASH_REMATCH[1]}
  if [[ ! -v "$name" ]]; then
    printf 'MISSING  %s\n' "$name"
    missing=$((missing + 1))
  elif [[ -z "${!name}" ]]; then
    printf 'EMPTY    %s\n' "$name"
    empty=$((empty + 1))
  else
    printf 'SET      %s\n' "$name"
    present=$((present + 1))
  fi
done < "$manifest"

if (( present + empty + missing == 0 )); then
  printf 'No environment assignments found in manifest.\n' >&2
  exit 2
fi

printf '\n%d set, %d empty, %d missing. No values printed.\n' "$present" "$empty" "$missing"
(( empty == 0 && missing == 0 ))
