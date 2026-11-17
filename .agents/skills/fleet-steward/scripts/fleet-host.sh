#!/bin/bash
# Runs on one fleet host, fed over stdin by fleet-check.sh.
# $1 is the previous tick as "YYYY-mm-dd HH:MM:SS +zzzz" in the steward's timezone.
# Prints every Activity log line since then as "<owner>/<repo>: <time> <event>",
# then one "running: <elapsed> <command>" line per running thirdshift Command.
since="$(date -d "$1" '+%Y-%m-%d %H:%M:%S' 2>/dev/null ||
         date -j -f '%Y-%m-%d %H:%M:%S %z' "$1" '+%Y-%m-%d %H:%M:%S')"
logs="${THIRDSHIFT_LOGS:-$HOME/.thirdshift/logs}"
for f in "$logs"/*/*/activity.log; do
  [ -f "$f" ] || continue
  r=${f%/activity.log}; r=${r#"$logs"/}
  awk -v s="$since" -v r="$r" 'substr($0,1,19) > s {print r": "substr($0,12)}' "$f"
done | sort -k2 | cut -c1-330
ps -eo etime,args | grep -E '^ *[0-9:-]+ ([^ ]*/)?thirdshift [a-z]' |
  grep -v -e grep -e 'thirdshift console' | sed -E 's/ +/ /g; s/^ //; s/^/running: /'
