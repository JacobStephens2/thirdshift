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
# Before it pushes anything, it refuses a release that can't be cut: a version
# that isn't plain X.Y.Z, or isn't higher than the one on main, a v<version>
# tag that already exists locally or on origin, or a latest CI run on main that
# didn't succeed.
#
# Prints a progress line on stderr for each step. Exits 0 once the tag is
# pushed. It exits 1 if it refuses, or if the PR's checks fail, in which case it
# leaves the PR open.
set -euo pipefail
# So a failure inside $(…) stops the script too.
shopt -s inherit_errexit

# Seconds between reads of the PR's workflow runs.
poll_seconds=${RELEASE_POLL_SECONDS:-15}

main() {
	if [ "$#" -ne 1 ]; then
		echo "usage: release.sh <version>" >&2
		exit 2
	fi
	version=$1
	tag=v$version
	branch=release-$version

	fetch_main
	base=$(git rev-parse --verify 'origin/main^{commit}')
	check_can_release "$version" "$tag" "$base"
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

	# Built first so a failure to build it stops the script, as it would not
	# inside the gh command line.
	body=$(pr_body "$base" "$tag")
	url=$(gh pr create --base main --head "$branch" --title "Release $version" \
		--body "$body" | tail -n 1)
	progress "opened $url"

	progress "waiting for CI on $url"
	failed=$(wait_for_workflow_runs "$head")
	if [ -n "$failed" ]; then
		progress "CI failed on $url ($failed), so the PR is left open, unmerged and untagged"
		exit 1
	fi

	gh pr merge "$branch" --merge --match-head-commit "$head" >/dev/null
	fetch_main
	merge=$(merge_commit_of "$head")
	progress "merged $url as ${merge:0:7}"

	git push --quiet origin "$merge:refs/tags/$tag"
	progress "tagged $tag on ${merge:0:7} and pushed the tag"
}

progress() {
	echo "release: $*" >&2
}

# refuse <reason>
refuse() {
	progress "refusing to release $version: $*"
	exit 1
}

# check_can_release <version> <tag> <base>
# Refuses unless <version> is plain X.Y.Z and higher than the version in the
# manifest at <base>, main's tip, no <tag> exists on origin or locally, and
# the latest CI run on main is on <base> and succeeded.
check_can_release() {
	local current short=${3:0:7} latest sha status conclusion
	if ! [[ $1 =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
		refuse "it is not a plain X.Y.Z version, such as 0.4.0"
	fi
	current=$(git show "$3:Cargo.toml" | package_field version)
	if ! is_higher "$1" "$current"; then
		refuse "it is not higher than $current on main"
	fi

	# Origin first: fetching main brings in the tags on it, so a tag found
	# locally may have come from origin.
	if [ -n "$(git ls-remote --tags origin "refs/tags/$2")" ]; then
		refuse "$2 already exists on origin"
	fi
	if git rev-parse --quiet --verify "refs/tags/$2" >/dev/null; then
		refuse "$2 already exists locally"
	fi

	latest=$(gh run list --branch main --workflow ci.yml --event push --limit 1 \
		--json headSha,status,conclusion --jq '.[] | "\(.headSha) \(.status) \(.conclusion)"')
	read -r sha status conclusion <<<"$latest"
	if [ "$sha" != "$3" ]; then
		refuse "CI has not run on main at $short yet"
	elif [ "$status" != completed ]; then
		refuse "CI on main at $short is $status, not finished"
	elif [ "$conclusion" != success ]; then
		refuse "CI on main at $short ended in $conclusion"
	fi
}

# is_higher <version> <other>
# Whether plain semver <version> is higher than <other>, part by part.
is_higher() {
	local -a ours theirs
	local i
	IFS=. read -ra ours <<<"$1"
	IFS=. read -ra theirs <<<"$2"
	for i in 0 1 2; do
		if ((ours[i] != theirs[i])); then
			((ours[i] > theirs[i]))
			return
		fi
	done
	return 1
}

fetch_main() {
	git fetch --quiet origin +refs/heads/main:refs/remotes/origin/main
}

# package_field <key>
# The value of <key> in the [package] section of the manifest on stdin.
package_field() {
	awk -F '"' -v key="$1" '
		/^\[/ { section = $0 }
		section == "[package]" && $1 == key " = " { print $2; exit }'
}

# bump_version <version>
# Sets the package's version in Cargo.toml, and its entry's in Cargo.lock, to
# <version>, leaving every other line alone.
# shellcheck disable=SC2016 # the $0 in the awk programs is awk's
bump_version() {
	local name
	name=$(package_field name <Cargo.toml)
	rewrite Cargo.toml -v version="$1" '
		/^\[/ { section = $0 }
		section == "[package]" && /^version = / && !done { $0 = "version = \"" version "\""; done = 1 }
		{ print }'
	rewrite Cargo.lock -v version="$1" -v name="$name" '
		/^\[\[package\]\]/ { ours = 0 }
		$0 == "name = \"" name "\"" { ours = 1 }
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

# pr_body <base> <tag>
# The body of the bump PR on HEAD, which <tag> will name, off <base>: the
# summary, between markers the Release page step can find, then the version
# diff. For now the summary is GitHub's generated notes for the PRs merged
# since the last tag.
pr_body() {
	local previous notes
	previous=$(git describe --tags --abbrev=0 --match 'v[0-9]*' "$1" 2>/dev/null || true)
	notes=$(gh api 'repos/{owner}/{repo}/releases/generate-notes' \
		-f tag_name="$2" -f target_commitish="$1" \
		${previous:+-f previous_tag_name="$previous"} --jq .body)
	cat <<-EOF
		## Summary

		<!-- release-summary:start -->
		$notes
		<!-- release-summary:end -->

		## Version diff

		\`\`\`diff
		$(git diff --no-color --unified=1 "$1" HEAD)
		\`\`\`
	EOF
}

# wait_for_workflow_runs <sha>
# Waits until <sha> has workflow runs and all of them have completed, then
# prints the names of those that did not succeed, comma-separated, or nothing.
# A workflow run completes only once all its jobs have, including jobs that
# start after others finish, which a commit's check runs would not show yet.
wait_for_workflow_runs() {
	local runs
	while :; do
		runs=$(gh api "repos/{owner}/{repo}/actions/runs?head_sha=$1&per_page=100" \
			--jq '.workflow_runs[] | "\(.status) \(.conclusion) \(.name)"')
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
