#!/bin/sh
# Publishes thirdshift.app: fetches main into a checkout kept for deploying
# and, when the served part of site/ changed since the last publish, copies it
# into the web root and swaps it in atomically. Safe to re-run; it does
# nothing when nothing changed.
#
# Usage: publish.sh <checkout> <web-root>
#
# Web root layout:
#   releases/<commit>.<random>/  one copy of site/ per publish, without deploy/
#   current -> releases/...      the copy Caddy serves; swapped with one rename
#   current/commit.txt           the commit of main the copy was taken from
#
# Needs only git and GNU coreutils (mv -T), so it runs on Linux.
set -eu

# The body is a function so the whole script is read before the checkout it
# lives in is moved to the new commit.
main() {
	if [ "$#" -ne 2 ]; then
		echo "usage: publish.sh <checkout> <web-root>" >&2
		exit 2
	fi
	checkout=$1
	root=$2

	git -C "$checkout" fetch --quiet origin +refs/heads/main:refs/remotes/origin/main
	commit=$(git -C "$checkout" rev-parse --verify 'origin/main^{commit}')

	previous=
	if [ -L "$root/current" ]; then
		previous=$(readlink "$root/current")
		if site_unchanged_since "$(cat "$root/current/commit.txt" 2>/dev/null)"; then
			return
		fi
	fi

	git -C "$checkout" checkout --quiet --force --detach "$commit"

	mkdir -p "$root/releases"
	release=$(mktemp -d "$root/releases/$commit.XXXXXX")
	chmod 755 "$release"
	cp -R "$checkout/site/." "$release/"
	rm -rf "$release/deploy"
	printf '%s\n' "$commit" >"$release/commit.txt"

	rm -f "$root/current.new"
	ln -s "releases/${release##*/}" "$root/current.new"
	mv -T "$root/current.new" "$root/current"

	# Keep the copy just replaced, for requests still reading it.
	for old in "$root"/releases/*; do
		[ -e "$old" ] || continue
		case "releases/${old##*/}" in
		"releases/${release##*/}" | "$previous") ;;
		*) rm -rf "$old" ;;
		esac
	done

	echo "published $commit"
}

# Whether main's served site, site/ without deploy/, matches the commit given.
site_unchanged_since() {
	[ -n "$1" ] &&
		git -C "$checkout" cat-file -e "$1^{commit}" 2>/dev/null &&
		git -C "$checkout" diff --quiet "$1" "$commit" -- site ':(exclude)site/deploy'
}

main "$@"
