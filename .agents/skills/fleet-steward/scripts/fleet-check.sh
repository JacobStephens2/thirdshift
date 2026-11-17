#!/bin/bash
# Fleet check for one steward tick. Prints the GitHub quota, open agent-failed issues,
# each host's Activity log lines since the previous tick with its running Commands,
# and the Console's Needs you rows.
#
# The fleet itself (owners, hosts, Console, extra noise filters) lives in a config file:
#   $THIRDSHIFT_FLEET_CONFIG, else ~/.thirdshift/fleet/fleet.conf
# Start from ../fleet.conf.example.
set -o pipefail
S="$(cd "$(dirname "$0")" && pwd)"

FLEET="$HOME/.thirdshift/fleet"
CONFIG="${THIRDSHIFT_FLEET_CONFIG:-$FLEET/fleet.conf}"
if [ ! -f "$CONFIG" ]; then
  echo "fleet-check: no fleet config at $CONFIG" >&2
  echo "copy $(dirname "$S")/fleet.conf.example there and fill it in" >&2
  exit 2
fi
OWNERS=(); HOSTS=(local); CONSOLE_URL=""; IGNORE=()
# shellcheck source=../fleet.conf.example
. "$CONFIG"

# Lines that never need the steward: idle skips, waits and routine Triage chatter.
NOISE=(
  'skipped: (no Ready|no eligible|no waiting|at the Command limit)'
  'Quota wait'
  'User config warning'
  'Triage run filing complete'
  'Triage run #[0-9]+ started'
  'Base branch CI step skipped'
  'still open for the Day shift'
  'waiting for triage'
  'goes first'
  'is still open: pick'
)
filter="$(printf '%s\n' "${NOISE[@]}" "${IGNORE[@]}" | paste -sd'|' -)"
quiet() { grep -Ev -- "$filter"; }

STATE="$FLEET/last-tick"
mkdir -p "$(dirname "$STATE")"
hour_ago() { date -v-60M '+%Y-%m-%d %H:%M:%S' 2>/dev/null || date -d '-60 min' '+%Y-%m-%d %H:%M:%S'; }
since="$(cat "$STATE" 2>/dev/null || hour_ago)"
now="$(date '+%Y-%m-%d %H:%M:%S')"
since_tz="$since $(date +%z)"
echo "## since $since (now $now)"

gh api rate_limit --jq '.resources|"quota core \(.core.remaining)/\(.core.limit) reset \(.core.reset|strflocaltime("%H:%M")); search \(.search.remaining)"'

echo "## open agent-failed"
for o in "${OWNERS[@]}"; do
  gh api -X GET search/issues -f q="label:agent-failed state:open user:$o" -f per_page=50 \
    --jq '.items[]|"\(.repository_url|sub(".*/repos/";""))#\(.number) \(.title[0:70]) [\(.labels|map(.name)|join(","))]"'
done | sort | quiet

# Run the collector on one host. A host is `local`, `lima:<instance>`, or an ssh destination.
collect() {
  case "$1" in
    local) bash -s -- "$since_tz" ;;
    lima:*)
      if limactl list "${1#lima:}" 2>/dev/null | grep -q Running; then
        limactl shell "${1#lima:}" -- bash -s -- "$since_tz"
      else
        echo "not running"
      fi ;;
    *) ssh -o ConnectTimeout=8 -o BatchMode=yes "$1" "bash -s -- '$since_tz'" ;;
  esac < "$S/fleet-host.sh" 2>&1
}
for h in "${HOSTS[@]}"; do
  echo "## $h"
  out="$(collect "$h" | quiet)"
  sed '/^running: /d; /^$/d' <<<"$out" | tail -40
  running="$(grep '^running: ' <<<"$out" | sed 's/^running: //' | paste -sd';' - | cut -c1-600)"
  echo "running: ${running:-idle}"
done

if [ -n "$CONSOLE_URL" ]; then
  echo "## Console Needs you"
  curl -s -m 20 "$CONSOLE_URL/?section=overview" | python3 "$S/needs-you.py" "$CONSOLE_URL"
fi

echo "$now" > "$STATE"
