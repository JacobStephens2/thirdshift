# Deploying thirdshift.app

This directory holds the configuration that serves thirdshift.app. It is never served itself.

- `Caddyfile`: the site block. It serves the published copy of `site/` with automatic TLS, permanently redirects `www.thirdshift.app` to `https://thirdshift.app`, and sends `/install.sh` with a 302 to the latest release's installer.
- `publish.sh`: fetches `main` into a checkout kept only for deploying. When `site/` (apart from this directory) has changed since the last publish, it copies it into the web root and swaps it in with one rename. It writes the published commit to `commit.txt` at the site root. It is safe to re-run, and does nothing when nothing changed.
- `thirdshift-site-publish.service` and `.timer`: run `publish.sh` every five minutes as the unprivileged `thirdshift-site` user.

The deploy only ever pulls from the public repository, so GitHub holds no credentials for the server. There are no analytics.

## Install

Installing is a manual step. Run these as root on the server, which already runs Caddy.

1. Create the user, the deploy checkout and the web root:

   ```sh
   useradd --system --home-dir /var/lib/thirdshift-site --create-home --shell /usr/sbin/nologin thirdshift-site
   install -d -o thirdshift-site -g thirdshift-site -m 755 /srv/thirdshift.app
   sudo -u thirdshift-site git clone https://github.com/JacobStephens2/thirdshift.git /var/lib/thirdshift-site/checkout
   ```

2. Publish once by hand, and check that `/srv/thirdshift.app/current` points at a copy of `site/` with a `commit.txt`:

   ```sh
   sudo -u thirdshift-site sh /var/lib/thirdshift-site/checkout/site/deploy/publish.sh /var/lib/thirdshift-site/checkout /srv/thirdshift.app
   cat /srv/thirdshift.app/current/commit.txt
   ```

3. Install and start the timer:

   ```sh
   cp /var/lib/thirdshift-site/checkout/site/deploy/thirdshift-site-publish.service \
      /var/lib/thirdshift-site/checkout/site/deploy/thirdshift-site-publish.timer \
      /etc/systemd/system/
   systemctl daemon-reload
   systemctl enable --now thirdshift-site-publish.timer
   systemctl list-timers thirdshift-site-publish.timer
   ```

4. Add the contents of `Caddyfile` to the server's Caddyfile, then validate and reload:

   ```sh
   caddy validate --config /etc/caddy/Caddyfile
   systemctl reload caddy
   ```

   On a host with SELinux enforcing, label the web root so Caddy may read it:

   ```sh
   semanage fcontext -a -t httpd_sys_content_t '/srv/thirdshift.app(/.*)?'
   restorecon -R /srv/thirdshift.app
   ```

5. Check it from anywhere:

   ```sh
   curl -I https://thirdshift.app                # 200
   curl -I https://www.thirdshift.app            # 301 to https://thirdshift.app/
   curl -I https://thirdshift.app/install.sh     # 302 to the latest thirdshift-installer.sh
   ```

The timer runs `publish.sh` from the deploy checkout, so changes to the script go live with the next publish. Changes to the units or the Caddy block need steps 3 or 4 again. Publish logs are in `journalctl -u thirdshift-site-publish`.
