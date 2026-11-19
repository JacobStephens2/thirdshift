---
name: verify-site
description: "Verify the thirdshift static site (site/ and docs/guide.md) on an ephemeral loopback port. Use when changing site/, docs/guide.md, GLOSSARY.md, src/prompts_page.rs, or prompts/."
---

# Verify Site

Serve `site/` locally on an OS-assigned ephemeral loopback port and drive its static and interactive pages in headless Chromium without external network requests.

## Launch

1. Regenerate derived pages first when sources changed (`npm run docs --prefix site` for `docs/guide.md`, `UPDATE_PROMPTS=1 cargo test prompts_page` for `src/prompt.rs`/`src/prompts_page.rs`/`skills/`), and ensure `site/node_modules` and freshness checks pass:

```sh
test -d site/node_modules || npm ci --prefix site
npm run docs:check --prefix site
```

2. Run verification inside one `( ... )` subshell with `trap cleanup EXIT` registered *before* starting the server or any browser daemon:

```sh
(
  set -eu
  state_dir=$(mktemp -d /tmp/verify-site-state.XXXXXX)
  server_pid=''
  cleanup() {
    if [ -n "$server_pid" ] && kill -0 "$server_pid" 2>/dev/null; then
      kill "$server_pid" 2>/dev/null || true
      wait "$server_pid" 2>/dev/null || true
    fi
    rm -rf "$state_dir"
  }
  trap cleanup EXIT

  # Optional host-only faux credentials (unused by static site/; leave Launch-directory .env untouched).
  verify_env="$HOME/.config/thirdshift/verify-env/JacobStephens2/thirdshift.env"
  if [ -r "$verify_env" ]; then
    set -a; . "$verify_env"; set +a
  fi

  node .agents/skills/verify-site/scripts/serve-site.cjs site \
    >"$state_dir/site.out" 2>"$state_dir/site.err" &
  server_pid=$!

  origin=''
  for _ in $(seq 1 100); do
    kill -0 "$server_pid" 2>/dev/null || { cat "$state_dir/site.err" >&2; exit 1; }
    origin=$(head -n 1 "$state_dir/site.out" 2>/dev/null || true)
    case "$origin" in
      http://127.0.0.1:[0-9]*) break ;;
    esac
    sleep 0.1
  done
  case "$origin" in
    http://127.0.0.1:[0-9]*) ;;
    *) echo "Site server failed to report ephemeral origin" >&2; cat "$state_dir/site.err" >&2; exit 1 ;;
  esac
)
```

## Doctor

Run this read-only health check before driving and after any failure:

```sh
kill -0 "$server_pid"
for path in / /how-it-works/ /architecture/ /docs/ /prompts/ /videos/ /styles.css; do
  curl -fsS "$origin$path" >/dev/null
done
home_html=$(curl -fsS "$origin/")
printf '%s' "$home_html" | grep -q 'Your issues become pull requests overnight.'
```

## Drive

Follow `thirdshift-verify-ui`'s two-phase workflow against [features/README.md](features/README.md):

1. **Interactive exploration:** drive affected pages (both `javaScriptEnabled: true` and `false` where progressive enhancement matters, at `1440×900` desktop and `390×844` mobile) while aborting external `https://**` requests (or stubbing `youtube-nocookie.com/embed/**` for video embed tests) so offline local runs never touch live YouTube or font CDNs. Stop any CLI browser daemon explicitly in `cleanup`.
2. **Clean walkthrough pass:** write a temporary Playwright script (`script=$(mktemp "$state_dir/walkthrough.XXXXXX.cjs")`) resolving `playwright` from `$(pwd)/site/node_modules/playwright`. `site/package.json` pins Playwright `1.64.0` (`>= 1.59`), so use native `recordVideo.showActions` with `viewport: { width: 1440, height: 900 }` (plus `390×844` when responsive layout changed) and matching `recordVideo.size`. Attach `pageerror`, `console` error, `requestfailed` (filtering expected `https://**` route aborts), and `>= 500` response listeners before navigation (on `/prompts/`, also allow the expected missing deploy-only `/commit.txt` 404 via `m.location().url`), pace deliberate `~300–500 ms` beats with zero idle pauses, assert visible state after each step, and close `context` then `browser` in `finally`.

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

Inspect finished `.png` captures and extracted video frames for legibility and layout fit, keep captures outside git, and pass exact paths and `**Before/After — <flow>, <viewport>**` labels to `thirdshift-pr` for `gh --attach`.

## Cleanup

The single `trap cleanup EXIT` handler stops and waits for `$server_pid` (and any CLI browser daemon), closes Playwright contexts/browser in `finally`, removes `$state_dir` and temporary scripts, leaves `$shot` and `$video` intact at their `/tmp` `mktemp` paths, and verifies zero server or browser child processes remain after exit (`kill -0 "$server_pid"` fails).

## Helpers

- `node .agents/skills/verify-site/scripts/serve-site.cjs [site-dir]` — executable static HTTP server bound to `127.0.0.1:0`, printing `http://127.0.0.1:<port>` on stdout and shutting down cleanly on `SIGTERM`/`SIGINT`.
