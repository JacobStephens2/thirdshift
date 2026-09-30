# The Resend API key can live in a Credentials file thirdshift reads itself

A **Run notification** needs a Resend API key, and thirdshift now finds it in `RESEND_API_KEY` or, when that is unset or empty, in the **Credentials**: `~/.thirdshift/credentials.toml`, mode 0600, holding `[resend] key = "re_..."`. **Setup** asks for the key with input hidden and writes it there. This amends [ADR 0005](0005-run-notifications-through-resend.md), which took the key only from the environment. The reason is that Runs are started from cron, `nohup`, CI and agent shells, and an environment variable only reaches them if something exports it. Cron reads no shell profile at all, and a stock Ubuntu `~/.bashrc` returns before its last line in any non-interactive shell. So the old advice to add an `export` line to your shell profile left `email.always = true` Runs from cron stopping before any work. Only a file thirdshift reads itself reaches every one of those contexts. The User config still holds no secret and can still be version controlled.

## Considered Options

- **Keep env-only, and have Setup print the export lines, or write them into the shell profile.** Rejected: every shell and OS has a different file with a different interactive-only guard, and cron needs a crontab line on top of that. Printing the lines hands that fragility to the user; editing rc files and crontabs is invasive.
- **Put the key in the User config.** Rejected: the User config is meant to be safe to share and version control.
- **A single-line file holding only the key.** Not chosen: a TOML file with a `[resend]` table follows the User config's conventions (strict parsing, edited in place) and leaves room for other providers' credentials.

## Consequences

- `RESEND_API_KEY` still wins when set, so CI secrets and existing shell exports keep working.
- The Credentials file is read only when a key is needed, so a broken one can't block a Run that sends no notification. It is parsed as strictly as the User config. A file readable by group or others is still used, with a warning to `chmod 600` it.
