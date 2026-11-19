# Sign-in and Account

When `allowed_emails` is configured without `--no-sign-in`, the Console redirects unauthenticated requests to `/sign-in`, sets CSRF-protected session cookies on sign-in, displays `/account` and `/console-log`, and revokes the browser session on sign-out.

## Sub-features

- `auth-redirect` redirects unauthenticated requests from `/` to `/sign-in` and returns `401` on unauthenticated `/api/v1/whoami`.
- `auth-sign-in` authenticates with faux seeded email/password and reaches `/`.
- `auth-console-log` records account events (such as sign-in) on `/console-log` (`ul.log-list`).
- `auth-account-and-sign-out` displays signed-in email on `/account` and ends the browser session via `Sign out this browser`.

## How to get to it (user POV)

- Open `$origin/` in an authenticated Console (`allowed_emails` configured, no `--no-sign-in`).
- Fill `Email` and `Password` on `/sign-in` and choose `Sign in`.
- Choose `Console log` (`g` then `c`) in `nav[aria-label="Pages"]` to inspect the `/console-log` Console log list.
- Choose `Account` in `nav[aria-label="Pages"]` (`g` then `a`) and submit `Sign out this browser`.

## Driving it with Playwright

Preconditions:

- Isolated `state_dir` seeded with `node .agents/skills/verify-console/scripts/seed-home.cjs "$state_dir" --sign-in` (faux credentials `owner@example.test` / `browser test password`).
- Console started *without* `--no-sign-in`; `Doctor` confirmed `GET /` returns `303` to `/sign-in`, `GET /api/v1/whoami` returns `401`, and `GET /sign-in` returns `200`.

- **Verify unauthenticated redirect.** Navigate to `${origin}/`. Browser redirects to `${origin}/sign-in` with heading `Sign in`.
- **Sign in with faux credentials.** Fill label `Email` (`owner@example.test`) and label `Password` (`browser test password`), then click button `Sign in`. Browser navigates to `${origin}/` and renders the Inbox for `acme/widgets`.
- **Verify Console log.** Click `Console log` in `nav[aria-label="Pages"]` (or press `g` then `c`). `${origin}/console-log` renders `<h1>Console log</h1>` and `ul.log-list li` includes the sign-in account event for `owner@example.test`.
- **Verify Account and sign out.** Click `Account` in `nav[aria-label="Pages"]`. Page heading `Account` displays `owner@example.test`; click `Sign out this browser`. Browser redirects back to `${origin}/sign-in`, and revisiting `${origin}/` or `${origin}/console-log` redirects to `/sign-in` again.
- **Proof.** Capture `/tmp/after-desktop.XXXXXX.png` and `/tmp/after-desktop.XXXXXX.mp4` showing Sign in -> Inbox -> Console log -> Account -> Sign out using only faux seeded data (never capture real `~/.thirdshift/console-accounts.json` or non-faux passwords).

## Gotchas

- Five consecutive failed password attempts trigger a one-attempt-per-minute slowdown per email address; keep the faux password exact (`browser test password`) or use a fresh `mktemp -d` `HOME`.
- Never commit or screenshot host `verify-env` secrets; keep any host-only test account details strictly faux and redacted from captures.
