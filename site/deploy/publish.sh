#!/bin/sh
# Publishes thirdshift.app: moves a checkout kept for deploying to the tip of
# main and, when the served part of site/ changed since the last publish,
# copies it into the web root and swaps it in atomically. Safe to re-run; it does
# nothing when nothing changed.
#
# Usage: publish.sh <checkout> <web-root>
#
# Web root layout:
#   copies/<commit>.<random>/  one copy of site/ per publish, without deploy/
#   current -> copies/...      the copy Caddy serves; swapped with one rename
#   current/commit.txt         the commit of main the copy was taken from
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
	# Always, so the units and Caddy block installed from the checkout are
	# main's even when no served file changed.
	git -C "$checkout" checkout --quiet --force --detach "$commit"

	previous=
	if [ -L "$root/current" ]; then
		previous=$(readlink "$root/current")
		published=$(cat "$root/current/commit.txt" 2>/dev/null || true)
		if site_unchanged "$checkout" "$published" "$commit"; then
			return
		fi
	fi

	mkdir -p "$root/copies"
	copy=$(mktemp -d "$root/copies/$commit.XXXXXX")
	chmod 755 "$copy"
	cp -R "$checkout/site/." "$copy/"
	rm -rf "$copy/deploy"
	printf '%s\n' "$commit" >"$copy/commit.txt"

	rm -f "$root/current.new"
	ln -s "copies/${copy##*/}" "$root/current.new"
	mv -T "$root/current.new" "$root/current"

	# Keep the copy just replaced, for requests still reading it.
	for old in "$root"/copies/*; do
		[ -e "$old" ] || continue
		case "copies/${old##*/}" in
		"copies/${copy##*/}" | "$previous") ;;
		*) rm -rf "$old" ;;
		esac
	done

	echo "published $commit"
}

# site_unchanged <checkout> <published commit> <commit>
# Whether the served site, site/ without deploy/, is the same at both commits.
# An empty or unknown published commit counts as changed.
site_unchanged() {
	[ -n "$2" ] &&
		git -C "$1" cat-file -e "$2^{commit}" 2>/dev/null &&
		git -C "$1" diff --quiet "$2" "$3" -- site ':(exclude)site/deploy'
}

main "$@"
