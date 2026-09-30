#!/usr/bin/env bash
# Cuts a thirdshift release: opens a PR into main that bumps the version and
# nothing else, waits for its checks, merges it with a merge commit, and tags
# that merge commit v<version>. Pushing the tag starts the dist release
# workflow.
#
# Usage: scripts/release.sh <version>
#
# Run it in a clone of this repo, signed in to gh. It works from origin/main in
# a temporary worktree, so the branch checked out where it runs and any
# uncommitted changes there are neither used nor changed.
#
# Prints a progress line on stderr for each step. Exits 0 once the tag is
# pushed; if the PR's checks fail, it leaves the PR open and exits 1.
set -euo pipefail

# Seconds between reads of the PR's checks.
poll_seconds=${RELEASE_POLL_SECONDS:-15}

main() {
	if [ "$#" -ne 1 ]; then
		echo "usage: release.sh <version>" >&2
		exit 2
	fi
	version=$1
	tag=v$version
	branch=release-$version

	git fetch --quiet origin +refs/heads/main:refs/remotes/origin/main
	base=$(git rev-parse --verify 'origin/main^{commit}')
	repo=$PWD
	work=$(mktemp -d)
	trap 'git -C "$repo" worktree remove --force "$work" 2>/dev/null || rm -rf "$work"' EXIT
	git worktree add --quiet --detach "$work" "$base"
	cd "$work"

	progress "bumping the version to $version on $branch from main at ${base:0:7}"
	bump_version "$version"
	git commit --quiet --all --message "Release $version"
	head=$(git rev-parse HEAD)
	git push --quiet origin "HEAD:refs/heads/$branch"

	url=$(gh pr create --base main --head "$branch" --title "Release $version" \
		--body "$(pr_body)" | tail -n 1)
	progress "opened $url"

	progress "waiting for CI on $url"
	failed=$(wait_for_checks "$head")
	if [ -n "$failed" ]; then
		progress "CI failed on $url ($failed), so the PR is left open, unmerged and untagged"
		exit 1
	fi

	gh pr merge "$branch" --merge --match-head-commit "$head" >/dev/null
	git fetch --quiet origin +refs/heads/main:refs/remotes/origin/main
	merge=$(merge_commit_of "$head")
	progress "merged $url as ${merge:0:7}"

	git push --quiet origin "$merge:refs/tags/$tag"
	progress "tagged $tag on ${merge:0:7} and pushed the tag"
}

progress() {
	echo "release: $*" >&2
}

# bump_version <version>
# Sets the package's version in Cargo.toml, and its entry's in Cargo.lock, to
# <version>, leaving every other line alone.
# shellcheck disable=SC2016 # the $0 in the awk programs is awk's
bump_version() {
	local name
	name=$(awk '/^\[/ { section = $0 } section == "[package]" && /^name = / { print; exit }' Cargo.toml)
	rewrite Cargo.toml -v version="$1" '
		/^\[/ { section = $0 }
		section == "[package]" && /^version = / && !done { $0 = "version = \"" version "\""; done = 1 }
		{ print }'
	rewrite Cargo.lock -v version="$1" -v name="$name" '
		/^\[\[package\]\]/ { ours = 0 }
		$0 == name { ours = 1 }
		ours && /^version = / { $0 = "version = \"" version "\""; ours = 0 }
		{ print }'
}

# rewrite <file> <awk arguments...>
rewrite() {
	local file=$1
	shift
	awk "$@" "$file" >"$file.new"
	mv "$file.new" "$file"
}

# The bump PR's body: the summary, between markers the Release page step can
# find, then the version diff. For now the summary is GitHub's generated notes
# for the PRs merged since the last tag.
pr_body() {
	local previous notes
	previous=$(git describe --tags --abbrev=0 --match 'v[0-9]*' "$base" 2>/dev/null || true)
	notes=$(gh api 'repos/{owner}/{repo}/releases/generate-notes' \
		-f tag_name="$tag" -f target_commitish="$base" \
		${previous:+-f previous_tag_name="$previous"} --jq .body)
	cat <<-EOF
		## Summary

		<!-- release-summary:start -->
		$notes
		<!-- release-summary:end -->

		## Version diff

		\`\`\`diff
		$(git diff --no-color --unified=1 "$base" HEAD)
		\`\`\`
	EOF
}

# wait_for_checks <sha>
# Waits until <sha> has check runs and all of them have completed, then prints
# the names of those that did not succeed, comma-separated, or nothing.
wait_for_checks() {
	local runs
	while :; do
		runs=$(gh api "repos/{owner}/{repo}/commits/$1/check-runs?per_page=100" \
			--jq '.check_runs[] | "\(.status) \(.conclusion) \(.name)"')
		if [ -n "$runs" ] && ! grep -qv '^completed ' <<<"$runs"; then
			break
		fi
		sleep "$poll_seconds"
	done
	awk '$2 != "success" && $2 != "skipped" && $2 != "neutral" {
		sub(/^[^ ]+ [^ ]+ /, ""); printf "%s%s", sep, $0; sep = ", "
	}' <<<"$runs"
}

# merge_commit_of <sha>
# The merge commit on origin/main whose second parent is <sha>.
merge_commit_of() {
	local merge
	merge=$(git rev-list --merges --parents --ancestry-path "$1..origin/main" |
		awk -v head="$1" '$3 == head { print $1 }')
	if [ -z "$merge" ]; then
		progress "no merge of ${1:0:7} on main after merging the PR"
		exit 1
	fi
	echo "$merge"
}

main "$@"
