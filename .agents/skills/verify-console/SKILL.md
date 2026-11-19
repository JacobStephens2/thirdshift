---
name: verify-console
description: "Verify the thirdshift Console web UI and API against isolated faux Command records on an ephemeral port. Use when changing src/console.rs, src/console/, src/config/console.rs, src/config/console/, or console/."
---

# Verify Console

Drive the live `thirdshift console` server against isolated faux Command records and a local stub `gh`. Every command runs inside one `( ... )` subshell that binds an OS-assigned ephemeral loopback port, owns its temporary `HOME`, and tears down the server and browser before exiting while keeping proof captures intact.

## Launch

1. Build the binary when `target/debug/thirdshift` (or `$THIRDSHIFT_BINARY`) is missing or stale, and ensure `console/node_modules` is installed:

```sh
cargo build --locked
test -d console/node_modules || npm ci --prefix console
```

2. Run verification inside one `( ... )` subshell with `trap cleanup EXIT` registered *before* starting the server or any browser daemon:

```sh
(
  set -eu
  state_dir=$(mktemp -d /tmp/verify-console-state.XXXXXX)
  server_pid=''
  cleanup() {
    if [ -n "$server_pid" ] && kill -0 "$server_pid" 2>/dev/null; then
      kill "$server_pid" 2>/dev/null || true
      wait "$server_pid" 2>/dev/null || true
    fi
    rm -rf "$state_dir"
  }
  trap cleanup EXIT

  # Optional host-only faux credentials (leave Launch-directory .env untouched).
  verify_env="$HOME/.config/thirdshift/verify-env/JacobStephens2/thirdshift.env"
  if [ -r "$verify_env" ]; then
    set -a; . "$verify_env"; set +a
  fi

  node .agents/skills/verify-console/scripts/seed-home.cjs "$state_dir"
  HOME="$state_dir" PATH="$state_dir/bin:$PATH" \
    "${THIRDSHIFT_BINARY:-./target/debug/thirdshift}" console --no-sign-in \
    >"$state_dir/console.out" 2>"$state_dir/console.err" &
  server_pid=$!

  origin=''
  for _ in $(seq 1 100); do
    kill -0 "$server_pid" 2>/dev/null || { cat "$state_dir/console.err" >&2; exit 1; }
    origin=$(head -n 1 "$state_dir/console.out" 2>/dev/null || true)
    case "$origin" in
      http://127.0.0.1:[0-9]*) break ;;
    esac
    sleep 0.1
  done
  case "$origin" in
    http://127.0.0.1:[0-9]*) ;;
    *) echo "Console failed to report ephemeral origin" >&2; cat "$state_dir/console.err" >&2; exit 1 ;;
  esac

  # Pass --sign-in to seed-home.cjs and omit --no-sign-in when verifying sign-in.
)
```

Set `THIRDSHIFT_CONSOLE_ASSETS=src/console/assets` in the server command only when verifying uncompiled CSS/JS asset edits in debug builds.

## Doctor

Run this read-only health check before the first drive and after any failure:

```sh
kill -0 "$server_pid"
whoami_json=$(curl -fsS "$origin/api/v1/whoami")
printf '%s' "$whoami_json" | grep -q '"api_version":"v1"'
inbox_html=$(curl -fsS "$origin/")
printf '%s' "$inbox_html" | grep -q 'acme/widgets'
```

In `--sign-in` mode, check `curl -sS -o /dev/null -w '%{http_code}' "$origin/"` returns `303` and `sign_in_html=$(curl -fsS "$origin/sign-in")` contains `Sign in`.

## Drive

Follow `thirdshift-verify-ui`'s two-phase workflow against [features/README.md](features/README.md):

1. **Interactive exploration:** drive the affected routes and controls with default CSP enforced (using `@playwright/cli`, `agent-browser`, or a short Node script resolving `console/node_modules/playwright`) while checking ARIA snapshots, console errors, `pageerror`, failed requests, `5xx` responses, keyboard shortcuts, and `390×844` phone responsiveness. Stop any CLI browser daemon explicitly in `cleanup` before exiting.
2. **Clean walkthrough pass:** write a temporary Playwright script (`script=$(mktemp "$state_dir/walkthrough.XXXXXX.cjs")`) resolving `playwright` from `$(pwd)/console/node_modules/playwright`. `console/package.json` pins Playwright `1.64.0` (`>= 1.59`), so use native `recordVideo.showActions` with `viewport: { width: 1440, height: 900 }` (and `390×844` when responsive layout changed) and matching `recordVideo.size`. Pass `bypassCSP: true` on the recording context only so the Console's `style-src 'self'` CSP does not block Playwright's `showActions` cursor/click overlay (keep Phase 1 exploration without `bypassCSP` so real CSP regressions still fail). Attach `pageerror`, `console` error, `requestfailed`, and `>= 500` response listeners before navigation, pace deliberate `~300–500 ms` beats with zero idle pauses, assert expected DOM/ARIA state after every action, and close `context` then `browser` in `finally`.

## Evidence

Create proof paths outside `$state_dir` so teardown preserves them:

```sh
shot=$(mktemp /tmp/after-desktop.XXXXXX.png)
video=$(mktemp /tmp/after-desktop.XXXXXX.mp4)
```

Pair `Before`/`After` `.png` screenshots for static layout/copy changes and `After` `.mp4` walkthroughs (`Before` `.mp4` too for bug fixes or changes to existing interactive flows). Transcode Playwright's `.webm` video and verify size, non-empty output, and H.264 `+faststart`:

```sh
ffmpeg -y -i "$raw_video" -an -c:v libx264 -pix_fmt yuv420p \
  -movflags +faststart -crf 28 "$video"
test -s "$video" && test "$(wc -c < "$video")" -le 8388608
```

Inspect finished `.png` captures and extracted video frames for legibility and faux-only data, keep captures outside git, and pass exact paths and `**Before/After — <flow>, <viewport>**` labels to `thirdshift-pr` for `gh --attach`.

## Cleanup

The single `trap cleanup EXIT` handler stops and waits for `$server_pid` (and any CLI browser daemon), closes Playwright contexts/browser in `finally`, removes `$state_dir` (and any temporary scripts), leaves `$shot` and `$video` intact at their `/tmp` `mktemp` paths, and verifies zero server or browser child processes remain after exit (`kill -0 "$server_pid"` fails).

## Helpers

- `node .agents/skills/verify-console/scripts/seed-home.cjs <mktemp-home-dir> [--sign-in]` — executable helper that seeds `.thirdshift/config.toml` (`port = 0`), faux Command records/logs/traces/spans (`acme/widgets#7`, `#8`, `#9` and `acme/gadgets#10`), a local stub `gh`, and (with `--sign-in`) faux `console-accounts.json` (`owner@example.test` / `browser test password`).
