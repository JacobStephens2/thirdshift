#!/usr/bin/env bash
# Run the replay stages one after another, one session at a time (#429 Q11,
# Q27, Q32). Weeding on thirdshift pauses for the Codex stage, which shares
# Codex's limits with it; Security runs on thirdshift pause for the Muse stage,
# which shares Muse's. Each paused line comes back on however its stage ends.
# Usage: stages.sh [stage ...]   (default: claude codex agy grok opencode muse)
set -u
here=$(cd "$(dirname "$0")" && pwd)
ROOT=$HOME/.thirdshift/replays/2026-10-09
WEEDING='cd ~/repos/thirdshift && sleep 20 && thirdshift architect base main'
SECURING='cd ~/repos/thirdshift && sleep 10 && thirdshift secure base main'
MARK='#replay-paused# '
stages=${*:-claude codex agy grok opencode muse}

log() { echo "[$(date '+%F %T')] $*"; }

# pause LINE PROCESS: comment out the crontab line, then wait for a pass
# already running to end.
pause() {
  crontab -l > "$ROOT/crontab.before-$stage-stage"
  crontab -l | awk -v w="$1" -v m="$MARK" 'index($0, w) && $0 !~ /^#/ { print m $0; next } { print }' | crontab -
  log "paused: $(crontab -l | grep -c "^$MARK") line(s) commented out"
  while pgrep -f "$2" > /dev/null; do
    log "waiting for the running '$2' to end"; sleep 60
  done
}

resume() {
  crontab -l | sed "s|^$MARK||" | crontab -
  log "crontab restored: $(crontab -l | grep -c "^$MARK") line(s) still paused"
}

trap 'exit 143' TERM INT
for stage in $stages; do
  log "stage $stage starting"
  case $stage in
    codex) pause "$WEEDING" 'thirdshift architect'; trap resume EXIT ;;
    muse) pause "$SECURING" 'thirdshift secure'; trap resume EXIT ;;
  esac
  python3 "$here/replay.py" run "$stage"
  code=$?
  log "stage $stage ended with $code"
  case $stage in
    codex|muse) resume; trap - EXIT ;;
  esac
  [ $code -eq 0 ] || log "stage $stage did not finish; the next stage runs anyway, on its own limits"
done
python3 "$here/replay.py" status
