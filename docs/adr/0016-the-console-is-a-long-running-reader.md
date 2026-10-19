# The Console runs all the time, but only reads and does what the CLI does

ADR 0009 rejected a long-running `thirdshift watch`: it has to be kept alive, it keeps running the old binary after `thirdshift update`, and when it hangs nobody notices. The **Console** needs something listening, so `thirdshift console` runs all the time, on 127.0.0.1, behind the machine's own Caddy or Apache. It stays within the spirit of ADR 0009 for three reasons:
- **It never starts work.** Every Command is still started by the operating system's scheduler or by a person, and the Console's only action, **Stop**, is the CLI's `thirdshift stop`.
- **It survives updates.** It re-executes itself when `thirdshift update` replaces the binary.
- **A hang costs no work.** A Console that hangs loses no Run; the next request shows it's down.

It reads what each Command writes about itself, its **Command record** and its logs, so it remembers nothing that a restart would lose, except sign-ins.

## Considered Options

- **No thirdshift process, with the web server serving the records, the logs and a static page.** Rejected:
  - The web server runs as its own user and can't enter the home folder.
  - Keeping **Security run** logs off the page would mean a deny rule in the web server's config for every Security log folder. A kind of log added later would leak until someone remembered its rule.
  - In the Console, thirdshift runs as its owner and decides what to show in code, under test.
- **Started on demand in a terminal or tunnel.** Rejected as the main way: a page that is down whenever nobody remembered to start it is no use from a phone. It still works this way with `--no-sign-in` over an SSH tunnel.

## Consequences

- `console install` writes a system unit that runs as the user, the same way other long-running apps are kept on both machines today. It prints a script for `sudo` that writes the unit and the proxy's site file.
- Every view and action on the page is also a CLI command with `--json`, and a test pairs them. Agents use the CLI, not the page, and no HTTP API is promised to anyone.
- Async code (axum on tokio) lives only inside the Console. The rest of thirdshift stays blocking.
