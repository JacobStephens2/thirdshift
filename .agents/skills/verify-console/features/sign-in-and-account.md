# Sign-in and Account

When `allowed_emails` is configured without `--no-sign-in`, the Console redirects unauthenticated requests to `/sign-in`, sets CSRF-protected session cookies on sign-in, displays `/account`, and revokes the browser session on sign-out.

## Sub-features

- `auth-redirect` redirects unauthenticated requests from `/` to `/sign-in`.
- `auth-sign-in` authenticates with faux seeded email/password and reaches `/`.
- `auth-account-and-sign-out` displays signed-in email on `/account` and ends the browser session via `Sign out this browser`.

## How to get to it (user POV)

- Open `$origin/` in an authenticated Console (`allowed_emails` configured, no `--no-sign-in`).
- Fill `Email` and `Password` on `/sign-in` and choose `Sign in`.
- Choose `Account` in `nav[aria-label="Pages"]` (`g` then `a`) and submit `Sign out this browser`.

## Driving it with Playwright

Preconditions:

- Isolated `state_dir` seeded with `node .agents/skills/verify-console/scripts/seed-home.cjs "$state_dir" --sign-in` (faux credentials `owner@example.test` / `browser test password`).
- Console started *without* `--no-sign-in`; `Doctor` confirmed `GET /` returns `303` to `/sign-in` and `GET /sign-in` returns `200`.

- **Verify unauthenticated redirect.** Navigate to `${origin}/`. Browser redirects to `${origin}/sign-in` with heading `Sign in`.
- **Sign in with faux credentials.** Fill label `Email` (`owner@example.test`) and label `Password` (`browser test password`), then click button `Sign in`. Browser navigates to `${origin}/` and renders the Inbox for `acme/widgets`.
- **Verify Account and sign out.** Click `Account` in `nav[aria-label="Pages"]`. Page heading `Account` displays `owner@example.test`; click `Sign out this browser`. Browser redirects back to `${origin}/sign-in`, and revisiting `${origin}/` redirects to `/sign-in` again.
- **Proof.** Capture `/tmp/after-desktop.XXXXXX.png` and `/tmp/after-desktop.XXXXXX.mp4` showing Sign in -> Inbox -> Account -> Sign out using only faux seeded data (never capture real `~/.thirdshift/console-accounts.json` or non-faux passwords).

## Gotchas

- Five consecutive failed password attempts trigger a one-attempt-per-minute slowdown per email address; keep the faux password exact (`browser test password`) or use a fresh `mktemp -d` `HOME`.
- Never commit or screenshot host `verify-env` secrets; keep any host-only test account details strictly faux and redacted from captures.
