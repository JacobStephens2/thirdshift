#!/usr/bin/env bash
# Ubuntu's package and WSL image hosts can fail through hosted-runner DNS
# while public resolvers still answer. Change only a disposable CI runner.
set -euo pipefail

hosts=(archive.ubuntu.com azure.archive.ubuntu.com ports.ubuntu.com security.ubuntu.com)
failed=
for hostname in "${hosts[@]}"; do
	if ! getent ahostsv4 "$hostname" >/dev/null; then failed=$hostname; break; fi
done
if [ -z "$failed" ]; then
	echo 'Ubuntu package hosts resolve through runner DNS'
	exit 0
fi
if [ "${GITHUB_ACTIONS:-}" != true ]; then
	echo "DNS failed for $failed; refusing to change DNS outside GitHub Actions" >&2
	exit 1
fi
echo "Runner DNS failed for $failed; using public DNS on this disposable runner"
printf 'nameserver 1.1.1.1\nnameserver 8.8.8.8\n' | sudo tee /etc/resolv.conf >/dev/null
for hostname in "${hosts[@]}"; do
	getent ahostsv4 "$hostname" >/dev/null
done
