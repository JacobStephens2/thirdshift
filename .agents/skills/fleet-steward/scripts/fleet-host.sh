#!/bin/bash
# Runs on one fleet host, fed over stdin by fleet-check.sh.
# $1 is the previous tick as "YYYY-mm-dd HH:MM:SS +zzzz" in the steward's timezone.
# Prints every Activity log line since then as "<owner>/<repo>: <time> <event>",
# then one "running: <elapsed> <command>" line per running thirdshift Command.
set -eo pipefail
since="$(date -d "$1" '+%Y-%m-%d %H:%M:%S' 2>/dev/null ||
         date -j -f '%Y-%m-%d %H:%M:%S %z' "$1" '+%Y-%m-%d %H:%M:%S')"
logs="${THIRDSHIFT_LOGS:-$HOME/.thirdshift/logs}"
check_log_directory() {
  if [ ! -r "$1" ] || [ ! -x "$1" ]; then
    echo "cannot read Activity log directory: $1" >&2
    return 1
  fi
}
if [ -d "$logs" ]; then
  check_log_directory "$logs"
fi
for owner in "$logs"/*; do
  [ -d "$owner" ] || continue
  check_log_directory "$owner"
  for repo in "$owner"/*; do
    [ -d "$repo" ] || continue
    check_log_directory "$repo"
    f="$repo/activity.log"
    [ -f "$f" ] || continue
    r=${repo#"$logs"/}
    awk -v s="$since" -v r="$r" 'substr($0,1,19) > s {print r": "substr($0,12)}' "$f"
  done
done | sort -k2
processes="$(ps -eo etime,args)"
printf '%s\n' "$processes" | awk '
  $2 ~ /(^|\/)thirdshift$/ && $3 ~ /^[a-z]/ && $3 != "console" {
    sub(/^ +/, ""); gsub(/ +/, " "); print "running: " $0
  }'
