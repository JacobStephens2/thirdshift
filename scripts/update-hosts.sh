#!/usr/bin/env bash
# Runs `thirdshift update` across a configured list of hosts.
#
# Usage: scripts/update-hosts.sh [<hosts-file>]
#
# By default, reads `scripts/update-hosts.conf` (ignored by git).
# Copy `scripts/update-hosts.conf.example` to `scripts/update-hosts.conf` to
# configure your own host list.
#
# Supported host entries (one per line, `#` comments and blank lines ignored):
#   local                   - run `thirdshift update` on this machine
#   lima:<instance>         - run `thirdshift update` inside a Lima VM (or `lima` for default)
#   ssh <user@host> [...]   - run `thirdshift update` over SSH
#   <user@host>             - shorthand for `ssh <user@host>`
#
# If `thirdshift update` refuses because the host has a cargo/source install
# instead of a release-installer receipt (e.g., on `aarch64-unknown-linux-gnu`
# where no prebuilt release binary is published), falls back to updating from
# `~/repos/thirdshift` (if present) or `cargo install thirdshift`.

set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
hosts_file="${1:-$script_dir/update-hosts.conf}"

if [[ ! -r "$hosts_file" ]]; then
	echo "error: hosts file not found: $hosts_file" >&2
	echo "  Copy $script_dir/update-hosts.conf.example to $hosts_file and customize it." >&2
	exit 1
fi

read -r -d '' remote_cmd <<'EOF' || true
set -euo pipefail
export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"
if out=$(thirdshift update 2>&1); then
	printf '%s\n' "$out" >&2
elif [[ "$out" == *"no install receipt"* ]]; then
	if [[ -d "$HOME/repos/thirdshift/.git" ]]; then
		echo "no install receipt; updating from ~/repos/thirdshift..." >&2
		cd "$HOME/repos/thirdshift"
		git fetch --tags --quiet origin
		latest_tag=$(git --no-pager tag -l 'v[0-9]*' --sort=-v:refname | sed -n '1p')
		if [[ -n "$latest_tag" ]]; then
			current_ver=$(thirdshift --version 2>/dev/null | awk '{print $2}' || true)
			if [[ "v$current_ver" == "$latest_tag" ]]; then
				echo "thirdshift: $(date +%H:%M:%S) thirdshift $current_ver is the latest release (from source)" >&2
				exit 0
			fi
			git checkout --quiet "$latest_tag"
		else
			git pull --ff-only
		fi
		cargo install --path . --locked
	else
		echo "no install receipt; running cargo install thirdshift..." >&2
		cargo install thirdshift --locked
	fi
else
	printf '%s\n' "$out" >&2
	exit 1
fi
EOF

run_host() {
	local entry="$1"
	if [[ "$entry" == "local" ]]; then
		bash -c "$remote_cmd"
	elif [[ "$entry" == "lima" ]]; then
		limactl shell default bash -c "$remote_cmd" </dev/null
	elif [[ "$entry" == lima:* ]]; then
		local instance="${entry#lima:}"
		limactl shell "$instance" bash -c "$remote_cmd" </dev/null
	elif [[ "$entry" == ssh\ * ]]; then
		local target="${entry#ssh }"
		# shellcheck disable=SC2086
		ssh $target "bash -s" <<<"$remote_cmd"
	else
		# shellcheck disable=SC2086
		ssh $entry "bash -s" <<<"$remote_cmd"
	fi
}

failures=0
ran=0

while IFS= read -r line || [[ -n "$line" ]]; do
	# Strip comments and surrounding whitespace.
	line="${line%%#*}"
	line="${line#"${line%%[![:space:]]*}"}"
	line="${line%"${line##*[![:space:]]}"}"
	[[ -z "$line" ]] && continue

	ran=$((ran + 1))
	printf '==> %s\n' "$line" >&2
	if ! run_host "$line"; then
		printf 'FAILED: %s\n' "$line" >&2
		failures=$((failures + 1))
	fi
done <"$hosts_file"

if [[ "$ran" -eq 0 ]]; then
	echo "warning: no hosts found in $hosts_file" >&2
fi

if [[ "$failures" -gt 0 ]]; then
	printf '\n%d of %d host(s) failed to update.\n' "$failures" "$ran" >&2
	exit 1
fi
