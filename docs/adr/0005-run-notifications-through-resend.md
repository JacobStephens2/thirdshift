# Run notifications go through the Resend HTTP API

A **Run notification** is sent by thirdshift itself with one HTTPS POST to Resend's API, using the key in the `RESEND_API_KEY` environment variable. It does not go through the machine's `mail`/`sendmail` command or through SMTP. Port 443 is open everywhere thirdshift runs, including DigitalOcean droplets and a MacBook on a home network, so one setup works on every machine. `reqwest` over rustls is already in the dependency tree through the updater, so the HTTP call adds no new native dependency to the static musl binary ([ADR 0003](0003-static-binaries-through-github-releases.md)). The evidence is in [the research note](../research/run-notifications-email.md).

## Considered Options

- **The system `mail`/`sendmail` command**. Rejected: fresh Ubuntu 22.04/24.04 and Rocky 9 servers have no `mail` command and no mail server, so every machine would need Postfix or msmtp installed and configured as a relay. macOS ships Postfix with no relay configured, so its mail goes directly to the recipient's server from a home IP and is unlikely to be accepted. Sending directly from a VM mostly fails too: port 25 is blocked by default on AWS, GCP, Azure, Hetzner and DigitalOcean, and Gmail rejects or spam-folders mail without SPF/DKIM and reverse DNS.
- **Built-in SMTP (`lettre`, e.g. a Gmail app password)**. Rejected: DigitalOcean blocks outbound 465 and 587 as well as 25, and the Runs mostly happen on DigitalOcean. Microsoft 365 is also turning off SMTP basic auth.
- **Amazon SES's HTTP API**. Deferred: it needs AWS credentials and SigV4 request signing, several times the work of Resend's single POST with a bearer key. The config is shaped so another provider can be added later.
- **Push services (ntfy, Pushover) or a user-defined end-of-Run command hook**. Not chosen: the ask was email. A hook remains a possible later addition.

## Consequences

- Each user needs a Resend account and an API key. With no verified domain, the default sender `onboarding@resend.dev` only delivers to the address of the user's own Resend account; any other recipient needs `email.from` set to an address on a verified domain.
- The API key only ever comes from the environment, never from the User config, so the config file holds no secret and can be version controlled.
- A Resend outage or a bad key costs only the Run notification. A failed send is a warning and never changes the Run's outcome, and `thirdshift email-test` exists so the setup can be checked without starting a Run.
