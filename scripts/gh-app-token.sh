#!/usr/bin/env bash
# Mint a GitHub App installation token for a host's gh, so its Passes spend
# the App's quota instead of a person's (#1993, ADR 0031).
#
# Usage: gh-app-token.sh <app-id> <private-key.pem> <installation-id>
#
# Stores the token as gh's github.com login, in place of a person's. Tokens
# last an hour, so run it from cron every 30 minutes:
#
#   */30 * * * * ~/bin/gh-app-token.sh 5733116 ~/.config/thirdshift-app/key.pem <installation-id>
#
# Each gh call reads gh's stored login afresh, so a Run longer than an hour
# picks up the next token; a GH_TOKEN set once at a Pass's start would expire
# under it. Unset GH_TOKEN and GITHUB_TOKEN in cron, since they take
# precedence. Run `gh auth setup-git` once so git pushes with the same token.
# Needs openssl, curl and python3.
set -euo pipefail

if [ $# -ne 3 ]; then
    echo "usage: $0 <app-id> <private-key.pem> <installation-id>" >&2
    exit 2
fi
app_id=$1 key=$2 installation=$3

b64url() { openssl base64 -A | tr '+/' '-_' | tr -d '='; }

now=$(date +%s)
# Backdated a minute for clock drift; GitHub allows at most ten minutes.
header=$(printf '{"alg":"RS256","typ":"JWT"}' | b64url)
claims=$(printf '{"iat":%d,"exp":%d,"iss":"%s"}' $((now - 60)) $((now + 540)) "$app_id" | b64url)
signature=$(printf '%s.%s' "$header" "$claims" | openssl dgst -sha256 -sign "$key" | b64url)
jwt="$header.$claims.$signature"

response=$(curl -fsS -X POST \
    -H "Authorization: Bearer $jwt" \
    -H "Accept: application/vnd.github+json" \
    -H "X-GitHub-Api-Version: 2022-11-28" \
    "https://api.github.com/app/installations/$installation/access_tokens")

token=$(printf '%s' "$response" | python3 -c 'import json, sys; print(json.load(sys.stdin)["token"])')

printf '%s\n' "$token" | env -u GH_TOKEN -u GITHUB_TOKEN gh auth login --hostname github.com --with-token
