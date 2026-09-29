# Run notifications: sending an email when a Run finishes

Research date: 2026-09-29.

**Question.** Can thirdshift send a short email when a Run finishes, on the two platforms it ships for (the static `x86_64-unknown-linux-musl` binary on Ubuntu 22.04/24.04, Rocky Linux 9 and WSL2, and the native `aarch64-apple-darwin` binary)? HTTPS must go through rustls, never OpenSSL ([ADR 0003](../adr/0003-static-binaries-through-github-releases.md)). Which paths actually deliver mail, what must the user set up for each, and what are the non-email alternatives?

**Method.**

- Distro facts came from the distros themselves. For Ubuntu: the package manifests of the official cloud images and live-server ISOs, and `apt-cache show` on Ubuntu 24.04.4. For Rocky 9: the `comps` group XML in the BaseOS and AppStream repodata, and the repo package listings.
- macOS facts came from Apple's open-source releases on GitHub (`apple-oss-distributions`): the `distribution-macOS` release manifest, and the `postfix` project's launchd plist and `main.cf`.
- lettre facts came from its source at tag `v0.11.23`, the current release on crates.io (published 2026-08-03). A throwaway crate was also built for `x86_64-unknown-linux-musl` on this machine.
- Provider rules came from the provider's own docs: Google, Microsoft Learn, AWS, Google Cloud, DigitalOcean, Hetzner, Resend, Postmark, Twilio SendGrid, ntfy, Pushover and Apple.

**Out of scope.** Choosing an option (see the last section), message wording, and where credentials are stored.

## TL;DR

- **No mail command and no MTA on a fresh server.** Ubuntu 22.04/24.04 server images and Rocky 9 minimal/server installs ship with no `mail`, `mailx`, `sendmail` or MTA. macOS still ships Postfix, and `/usr/sbin/sendmail` exists there. It has no relay host, though, so it tries direct-to-MX delivery over port 25.
- **Direct delivery from a VM or laptop mostly fails.** AWS, GCP, Azure (non-EA) and Hetzner block outbound port 25 by default. DigitalOcean also blocks 465 and 587. Residential IPs are on the Spamhaus PBL. Gmail wants SPF or DKIM plus valid reverse DNS from every sender, and says unauthenticated mail "might be marked as spam or rejected".
- **Workable email means authenticated submission to a real provider**, either SMTP on 465/587 or an HTTPS API. In Rust, `lettre` 0.11 does SMTP with rustls only (no OpenSSL) and builds a static-pie musl binary. Both the `ring` and `aws-lc-rs` backends work. thirdshift already compiles `aws-lc-rs` for musl today, through `axoupdater` → `reqwest`.
- **Gmail app passwords still work (2026)**, but need 2-Step Verification, and Google calls them "not recommended". **Microsoft 365** SMTP AUTH basic auth is set to be off by default from the end of December 2026, with final removal to be announced in H2 2027.
- **HTTP email APIs** (Resend, Postmark, SendGrid, SES) all need an API key **and a verified sending domain or sender**, and most restrict new accounts. That is more setup than "ping me when done" deserves for a solo developer without a domain.
- **ntfy** is one HTTPS POST with no account, and ntfy.sh can also forward to email (verified account needed, 5/day). **Pushover** costs $4.99 one-time per platform. Desktop notifications only help when the user sits at the machine that ran the Run, which rules them out for a headless server.

## 1. Linux: Ubuntu 22.04/24.04 and Rocky Linux 9

### 1.1 What a fresh install has

**Ubuntu.** None of `mailutils`, `bsd-mailx`, `s-nail`, `postfix`, `exim4`, `msmtp`, `ssmtp`, `sendmail`, `dma` or `nullmailer` appears in:

- the Ubuntu 24.04 cloud image manifest ([noble-server-cloudimg-amd64.manifest](https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.manifest), 664 packages)
- the 22.04 cloud image manifest ([jammy-server-cloudimg-amd64.manifest](https://cloud-images.ubuntu.com/jammy/current/jammy-server-cloudimg-amd64.manifest), 600 packages)
- the live-server ISO manifests ([ubuntu-24.04.3-live-server-amd64.manifest](https://releases.ubuntu.com/noble/ubuntu-24.04.3-live-server-amd64.manifest), [ubuntu-22.04.5-live-server-amd64.manifest](https://releases.ubuntu.com/jammy/ubuntu-22.04.5-live-server-amd64.manifest))

So a default server or cloud install has no `mail` command and no MTA.

**Rocky Linux 9.** In the [BaseOS comps](https://dl.rockylinux.org/pub/rocky/9/BaseOS/x86_64/os/repodata/), `minimal-environment` is the `core` group with `standard` optional. `server-product-environment` is `core`, `hardware-support`, `headless-management`, `server-product` and `standard`. None of those groups lists `postfix`, `s-nail`, `sendmail` or `msmtp`. `postfix` appears only in the optional **`mail-server`** group of the [AppStream comps](https://dl.rockylinux.org/pub/rocky/9/AppStream/x86_64/os/repodata/), next to `sendmail` as an option. Both `postfix` (3.5.25) and `s-nail` (14.9.22) live in AppStream. RHEL 9 replaced `mailx` with `s-nail`: "The s-nail mail processing system has replaced the mailx utility" ([RHEL 9.0 release notes, new features](https://docs.redhat.com/en/documentation/red_hat_enterprise_linux/9/html/9.0_release_notes/new-features); [Red Hat: mailx is not available in RHEL9](https://access.redhat.com/solutions/6999497)).

### 1.2 Which packages provide what (Ubuntu 24.04, `apt-cache show`)

| Package | Provides | Pulls in an MTA? |
|---|---|---|
| `mailutils` (universe) | `mailx`, `mail-reader` | Recommends `default-mta \| mail-transport-agent`, so Postfix by default |
| `bsd-mailx` (main) | `mailx`, `mail-reader` | **Depends** on `default-mta \| mail-transport-agent` |
| `s-nail` (universe) | `mail-reader` | No MTA dependency. Can submit over SMTP itself, and links `libssl3t64` |
| `postfix` (main) | `default-mta`, `mail-transport-agent` | It is the MTA. `default-mta` resolves only to `postfix` |
| `msmtp-mta` (universe) | `mail-transport-agent` (a `sendmail` shim that relays to a configured SMTP server) | It is a relay-only MTA |

Sources: [packages.ubuntu.com/noble/mailutils](https://packages.ubuntu.com/noble/mailutils), [bsd-mailx](https://packages.ubuntu.com/noble/bsd-mailx), [s-nail](https://packages.ubuntu.com/noble/s-nail), [postfix](https://packages.ubuntu.com/noble/postfix), [msmtp-mta](https://packages.ubuntu.com/noble/msmtp-mta). So `apt install mailutils` on a stock system also installs Postfix, which then tries direct delivery unless someone configures a relay host.

### 1.3 Would direct delivery from a VM reach Gmail or Outlook?

**Outbound port 25 is blocked on most clouds:**

| Cloud | Policy |
|---|---|
| AWS EC2 | Outbound port 25 is blocked to public IPv4 and IPv6 for accounts not on an allowlist. Removal is by request form, per Region. Ports 465/587 to SES "are not restricted" ([AWS Knowledge Center](https://repost.aws/knowledge-center/ec2-port-25-throttle); [SES: connecting to the SMTP endpoint](https://docs.aws.amazon.com/ses/latest/dg/smtp-connect.html): "Amazon EC2 … throttles email traffic over port 25 by default") |
| Google Cloud | "Connections to destination TCP Port 25 are blocked when the destination is external to your VPC network." "Google Cloud does not place any restrictions on traffic sent to … ports 587 or 465" ([Compute Engine: sending email](https://docs.cloud.google.com/compute/docs/tutorials/sending-mail)) |
| Azure | Port 25 is blocked for VMs on all subscription types except Enterprise Agreement / MCA-E, and even there "there's no guarantee that external domains accept the incoming emails". Authenticated relay on 587 "isn't restricted in Azure, regardless of the subscription type" ([Microsoft Learn](https://learn.microsoft.com/en-us/azure/virtual-network/troubleshoot-outbound-smtp-connectivity)) |
| DigitalOcean | "SMTP ports 25, 465, and 587 are blocked on Droplets." DigitalOcean recommends a third-party email service ([DigitalOcean docs](https://docs.digitalocean.com/support/why-is-smtp-blocked/)). **So SMTP submission fails on a Droplet too, and only an HTTPS API works there.** |
| Hetzner Cloud | "We block ports 25 and 465 by default on all cloud servers." They can be unblocked after a month and a paid invoice. "Port 587 is not blocked" ([Hetzner Cloud FAQ](https://docs.hetzner.com/cloud/servers/faq/)) |

**Residential IPs.** The Spamhaus PBL lists end-user and dynamic ranges that "should not be attempting to directly deliver unauthenticated SMTP email to any Internet mail server" ([Spamhaus PBL](https://www.spamhaus.org/blocklists/policy-blocklist/)).

**Gmail's rules for all senders** (since 1 February 2024): "Set up SPF or DKIM email authentication for your sending domains", "Ensure that sending domains or IPs have valid forward and reverse DNS records", and "Use a TLS connection". "Messages that aren't authenticated with these methods might be marked as spam or rejected." ([Google: Email sender guidelines](https://support.google.com/a/answer/81126)). A fresh VM's Postfix sends as `user@<vm-hostname>` from an IP whose reverse DNS is the cloud's generic name, with no SPF or DKIM for that domain. So even where port 25 is open, landing in a Gmail inbox is unlikely.

## 2. macOS (Apple Silicon, current)

- **Postfix still ships.** The macOS 26.5 open-source release manifest lists project `postfix`, tag `postfix-1636` ([distribution-macOS `macos-265` release.json](https://github.com/apple-oss-distributions/distribution-macOS/blob/macos-265/release.json)). Its `main.cf` sets `sendmail_path = /usr/sbin/sendmail` and `inet_interfaces = loopback-only`, and it sets **no `relayhost`** ([postfix-1636 `main.cf.default`](https://github.com/apple-oss-distributions/postfix/blob/postfix-1636/Postfix.Config/main.cf.default)).
- **Postfix starts on demand.** launchd starts `master -e 60` when a file appears in `/var/spool/postfix/maildrop` (`QueueDirectories`), so it runs for about 60 s after a `sendmail` call and then exits ([`com.apple.postfix.master.plist`](https://github.com/apple-oss-distributions/postfix/blob/postfix-1636/Postfix.LaunchDaemons/com.apple.postfix.master.plist)). An Apple Developer Forums thread from March 2026 confirms the system daemon `com.apple.postfix.master` is still present on Sequoia ([developer.apple.com/forums/thread/816382](https://developer.apple.com/forums/thread/816382)).
- **So `/usr/sbin/sendmail` accepts mail but does not reliably deliver it.** Without a relay host, Postfix looks up the recipient's MX and connects on port 25 from the Mac's (usually residential) IP. That runs into the PBL and Gmail rules in §1.3. The realistic path from a Mac CLI is the same as on Linux: authenticated submission to a real provider (SMTP 465/587 or an HTTPS API), done by the program itself. The alternative is having the user reconfigure Postfix with a `relayhost` and SASL credentials, which needs `sudo` and editing `/etc/postfix/main.cf`.

## 3. SMTP from Rust: `lettre`

Facts below are from lettre `v0.11.23` source ([Cargo.toml](https://github.com/lettre/lettre/blob/v0.11.23/Cargo.toml), [lib.rs feature docs](https://github.com/lettre/lettre/blob/v0.11.23/src/lib.rs)).

- **The default features pull in OpenSSL on Linux.** `default = ["smtp-transport", "pool", "native-tls", "hostname", "builder"]`, and native-tls "uses … OpenSSL on all other platforms" (other than Windows and macOS). So thirdshift must use `default-features = false`.
- **A rustls-only build is supported.** Features:
  - `rustls` (sync) or `tokio1-rustls` (async)
  - **exactly one crypto backend**, which "MUST also be enabled": `ring`, `aws-lc-rs`, or `rustls-no-provider` (bring your own)
  - **exactly one verifier**, which "MUST also be enabled": `rustls-platform-verifier` (OS store; lettre's "when in doubt" pick), `rustls-native-certs`, or `webpki-roots` (bundled Mozilla roots, no runtime packages)

  The old `rustls-tls` feature (= `webpki-roots` + `rustls` + `ring`) is deprecated.
- **Minimal feature set:** `default-features = false, features = ["builder", "smtp-transport", "rustls", "ring" or "aws-lc-rs", "rustls-platform-verifier" or "webpki-roots", "hostname"]`.
- **Tested here:** that feature set with `ring` built for `x86_64-unknown-linux-musl` into a `static-pie linked` ELF of about 3.1 MB. `cargo tree` showed no `openssl` or `native-tls` crate. (`openssl-probe` appears under `rustls-native-certs`; it is pure Rust and only locates CA bundle paths.) The same build with `aws-lc-rs` failed locally because `aws-lc-sys` needs a musl C compiler (`x86_64-linux-musl-gcc`). It also failed with host `gcc` (glibc symbols like `__isoc23_sscanf` at link).
- **aws-lc-rs on musl.** aws-lc-rs docs: non-FIPS builds need a C compiler, "CMake: Never required", and "Bindgen: Never required (universal pre-generated bindings are provided)". `x86_64-unknown-linux-musl` and `aarch64-apple-darwin` are supported ([Linux requirements](https://aws.github.io/aws-lc-rs/requirements/linux.html), [platform support](https://aws.github.io/aws-lc-rs/platform_support.html)). lettre notes "`aws-lc-rs` may require cmake on some platforms".
  - **thirdshift already carries this cost.** `Cargo.lock` has `aws-lc-rs 1.18.1` / `aws-lc-sys 0.45.0` through `axoupdater → axoasset → reqwest 0.13 (rustls, aws-lc-rs)`, and the v0.3.0 release job ran `sudo apt-get install musl-tools` before building. Choosing `aws-lc-rs` for lettre therefore adds no new toolchain requirement. Choosing `ring` would add a second crypto library to the binary.
- **TLS modes** (`Tls` enum, [tls.rs](https://github.com/lettre/lettre/blob/v0.11.23/src/transport/smtp/client/tls.rs)):
  - `Wrapper`: implicit TLS, port 465, "the fastest and most secure option"
  - `Required`: STARTTLS, which fails if the server can't upgrade
  - `Opportunistic`: "strongly discouraged", because it is vulnerable to MITM
  - `None`

  The constructors are `SmtpTransport::relay(host)` (implicit TLS on the submissions port 465), `SmtpTransport::starttls_relay(host)` (587, STARTTLS required), `from_url("smtps://user:pass@host")`, and `builder_dangerous` (port 25, no TLS) ([transport.rs](https://github.com/lettre/lettre/blob/v0.11.23/src/transport/smtp/transport.rs)).
- **Auth mechanisms:** `Plain` (RFC 4616), `Login` ("Obsolete but needed for some providers (like Office 365)") and `Xoauth2` ([authentication.rs](https://github.com/lettre/lettre/blob/v0.11.23/src/transport/smtp/authentication.rs)). OAuth token acquisition is the caller's job.
- **The `sendmail-transport` feature shells out** to `sendmail` (the default command) with `-i` and `-f <from>`, piping the message on stdin ([sendmail/mod.rs](https://github.com/lettre/lettre/blob/v0.11.23/src/transport/sendmail/mod.rs)). It needs no TLS feature, but it is only as good as the local MTA (§1–2).

## 4. Consumer mail providers' SMTP

**Gmail / Google Workspace.**

- Server: `smtp.gmail.com`, "For SSL, enter 465" / "For TLS, enter 587". Authenticate with the full address "and an app password". Limit: "The sending limit is 2,000 messages per day" ([Google Workspace: send email from a printer, scanner or app](https://knowledge.workspace.google.com/admin/gmail/send-email-from-a-printer-scanner-or-app)).
- App passwords are still available in 2026. "App passwords can only be used with accounts that have 2-Step Verification turned on." They are unavailable for Advanced Protection accounts, for accounts with security-key-only 2SV, and possibly for work or school accounts. Google says they "aren't recommended and are unnecessary in most cases" ([Google Account Help: sign in with app passwords](https://support.google.com/accounts/answer/185833)).
- Plain-password ("less secure apps") SMTP stopped on 14 March 2025: "SMTP, and POP will no longer work with legacy passwords (basic authentication)", "with the exception of app passwords" ([Google Workspace: transition from less secure apps to OAuth](https://knowledge.workspace.google.com/admin/sync/transition-from-less-secure-apps-to-oauth)).
- Sending through Gmail with the user's own address as From passes Gmail's own SPF/DKIM, so it lands.

**Microsoft 365 / Exchange Online.**

- Basic auth is gone for every protocol except SMTP AUTH. "Although SMTP AUTH is currently available, Microsoft has announced plans to retire Basic authentication for SMTP AUTH". Microsoft suggests the Graph API as an alternative ([Microsoft Learn, updated 2026-07-10](https://learn.microsoft.com/en-us/exchange/clients-and-mobile-in-exchange-online/deprecation-of-basic-authentication-exchange-online)).
- The linked timeline post of January 2026 says:
  - unchanged through December 2026
  - **disabled by default for existing tenants at the end of December 2026**, with admins still able to re-enable it
  - unavailable by default for new tenants after that
  - final removal date to be announced in H2 2027

  Source: [Exchange Team blog, "Updated Exchange Online SMTP AUTH Basic Authentication Deprecation Timeline"](https://techcommunity.microsoft.com/blog/exchange/updated-exchange-online-smtp-auth-basic-authentication-deprecation-timeline/4489835). This page returned 403 to direct fetch, so the dates come from its indexed text.
- In practice a Microsoft 365 user needs OAuth (XOAUTH2 with an Entra app registration) for durable SMTP.

## 5. HTTP email APIs

All four need an HTTPS POST with an API key, which fits the existing `reqwest`/rustls stack and works on DigitalOcean where SMTP ports are blocked. All four also need **a sender identity the provider has verified**, and most gate new accounts:

- **Resend.** "You must add and verify at least one domain to send emails" ([Resend: domains](https://resend.com/docs/dashboard/domains/introduction.md)). Without one, the shared `resend.dev` sender can only send "testing emails to your own email address" ([Resend: errors](https://resend.com/docs/api-reference/errors.md)). That limit happens to be enough for "email me when my Run finishes".
- **Postmark.** Senders must be confirmed Sender Signatures on a domain you control (no Gmail or Yahoo From). Until the account is approved, you can only send to domains you have added and verified ([Postmark: managing sender signatures](https://postmarkapp.com/developer/user-guide/managing-your-account/managing-sender-signatures), [account approval](https://postmarkapp.com/support/article/1084-how-does-the-account-approval-process-work)).
- **SendGrid.** Offers Single Sender Verification (confirm one address) or Domain Authentication. It warns against free-mailbox addresses because "Messages from this domain might fail a DMARC check" ([Twilio SendGrid: sender verification](https://www.twilio.com/docs/sendgrid/ui/sending-email/sender-verification)).
- **Amazon SES.** New accounts are in a per-Region sandbox: "You can only send mail **to** verified email addresses and domains", 200/day, 1/s. Even in production, every From identity must be verified ([SES: request production access](https://docs.aws.amazon.com/ses/latest/dg/request-production-access.html)). Verifying the single address you both send from and to is enough in the sandbox for self-notification. It also needs AWS credentials and SigV4 signing, or SES SMTP credentials.

Compared with SMTP through the user's own Gmail, these APIs trade "an app password on an account you already have" for "a new account, an API key and (usually) DNS records on a domain you own".

## 6. Non-email alternatives for "ping me when done"

- **ntfy.**
  - Publishing is a single POST: `curl -d "Backup successful 😀" ntfy.sh/mytopic`, with optional `Title`, `Priority` (1–5) and `Tags` headers ([ntfy: publishing](https://docs.ntfy.sh/publish/)).
  - No account is needed. "The topic is essentially a password, so pick something that's not easily guessable." Subscribers use the Android or iOS app or the web app.
  - ntfy can also forward to email with an `Email:` header. On ntfy.sh, though, "anonymous email sending was disabled due to abuse … you must verify your email in the web app's Account section. The daily limit for free users is 5 emails per visitor per day" ([ntfy docs source, `publish.md`](https://github.com/binwiederhier/ntfy/blob/main/docs/publish.md#e-mail-notifications)).
  - ntfy is self-hostable.
- **Pushover.** POST `token`, `user` and `message` to `https://api.pushover.net/1/messages.json`. Each account gets 10,000 messages a month free ([Pushover API](https://pushover.net/api)). The client apps cost "$4.99 USD one-time purchase on each platform" after a 30-day trial ([Pushover pricing](https://pushover.net/pricing)). The user must register an application to get a token.
- **Desktop notifications.**
  - macOS: `osascript -e 'display notification "…" with title "…"'` via Standard Additions ([Mac Automation Scripting Guide: Displaying Notifications](https://developer.apple.com/library/archive/documentation/LanguagesUtilities/Conceptual/MacAutomationScriptingGuide/DisplayNotifications.html)).
  - Linux: `notify-send` (`libnotify-bin`) or the Freedesktop notification spec over the D-Bus session bus ([notify-send(1)](https://manpages.ubuntu.com/manpages/noble/man1/notify-send.1.html), [Desktop Notifications Specification](https://specifications.freedesktop.org/notification-spec/latest/)). This needs a graphical session, so it does nothing on a headless server or over SSH.
  - The pure-Rust `notify-rust` crate (4.18.1) covers both platforms.
  - Desktop notifications are only useful when the user is at the machine that ran the Run.

## Implications for thirdshift

These are the options, not a decision:

1. **Shell out to `sendmail` / `mail`** (or lettre's `sendmail-transport`).
   - *For:* no credentials in thirdshift and no TLS code.
   - *Against:* absent on stock Ubuntu and Rocky servers. Present on macOS but it delivers nothing reliably, and on any fresh box it falls into port-25 blocks and Gmail's SPF/DKIM/rDNS rules. It only works for users who have already configured a relay (`msmtp-mta`, or Postfix with a `relayhost`). Failures are silent: mail sits in a queue.
2. **Built-in SMTP submission with `lettre`** (rustls, `aws-lc-rs` to match the existing stack, implicit TLS on 465 or STARTTLS-required on 587).
   - *For:* works from AWS, GCP, Azure, Hetzner and laptops. With a Gmail app password and self-addressed mail, delivery is good and errors come back synchronously. It fits ADR 0003, with no new toolchain because `aws-lc-sys` already builds for musl in CI.
   - *Against:* thirdshift has to store an SMTP password. Gmail requires 2SV and calls app passwords "not recommended". Microsoft 365 basic auth is going away (off by default from the end of December 2026). SMTP is fully blocked on DigitalOcean.
3. **An HTTP email API** (Resend, Postmark, SendGrid, SES) over the rustls `reqwest` stack.
   - *For:* HTTPS works everywhere, including DigitalOcean. Good deliverability.
   - *Against:* each user needs a provider account, an API key and usually a verified domain (Resend's shared sender allows self-only testing sends; SES sandbox allows verified-to-verified). Each provider is its own integration.
4. **ntfy (optionally with ntfy.sh's email forward)**.
   - *For:* one unauthenticated HTTPS POST and no secrets beyond an unguessable topic. It works everywhere HTTPS does, and a self-hosted server is possible. Email forwarding comes almost free.
   - *Against:* it is a push notification rather than email unless the user verifies an ntfy.sh account (5 emails/day). The topic name is the only secret. It depends on a third-party service.
5. **Pushover.** Similar to ntfy but with an account, an app token and a $4.99 client, in exchange for a polished, reliable push service. Not email.
6. **Desktop notification.** Zero setup on a Mac or Linux desktop, useless on headless servers. At best a complement to one of the above.

A generic "run this command when a Run finishes" hook would let users wire up any of these themselves. It is also the escape hatch if thirdshift builds in only one option.
