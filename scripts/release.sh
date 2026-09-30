#!/usr/bin/env bash
# Cuts a thirdshift release: opens a PR into main that bumps the version and
# nothing else, waits for its checks, merges it with a merge commit, and tags
# that merge commit v<version>. Pushing the tag starts the dist release
# workflow.
#
# Usage: scripts/release.sh <version>
#
# Run it in a clone of this repo, signed in to gh and with claude logged in. It
# works from origin/main in a temporary worktree, so the branch checked out
# where it runs and any uncommitted changes there are neither used nor changed.
#
# claude, with no tools, writes the PR's summary from the prompt in
# release-summary.md beside this script. If it fails, the summary is GitHub's
# generated notes instead, and the script warns and carries on.
#
# If a run is interrupted, running it again with the same version carries on
# from the first step not yet done: it opens the PR for a pushed branch, waits
# on and merges an open PR, or tags a merged one. If v<version> is already on
# origin, on the merge of the bump PR, it says so and exits 0; a v<version>
# tag anywhere else is refused.
#
# Prints a progress line on stderr for each step. Exits 0 once the tag is
# pushed; if the PR's checks fail, it leaves the PR open and exits 1.
set -euo pipefail
# So a failure inside $(…) stops the script too.
shopt -s inherit_errexit

# The prompt the summary agent gets ahead of the release's input.
summary_prompt=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/release-summary.md

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
	find_earlier_run
	exit_if_tagged

	case $pr_state in
	"")
		if [ -z "$head" ]; then
			push_bump
		else
			progress "resuming: $branch is on origin at ${head:0:7} with no PR"
		fi
		open_pr
		wait_and_merge
		;;
	OPEN)
		progress "resuming: $url is open"
		wait_and_merge
		;;
	MERGED)
		progress "resuming: $url was merged as ${merge:0:7}"
		;;
	*)
		progress "$url was closed without merging, so there is nothing to resume; reopen it or delete $branch to start over"
		exit 1
		;;
	esac

	git push --quiet origin "$merge:refs/tags/$tag"
	progress "tagged $tag on ${merge:0:7} and pushed the tag"
}

# find_earlier_run
# Sets what an earlier run for this version got done: pr_state (OPEN, MERGED,
# CLOSED, or empty with no PR), url, head and merge from the newest PR from
# $branch, or with no PR, head from $branch on origin if it was pushed.
find_earlier_run() {
	local pr
	pr=$(gh pr list --head "$branch" --base main --state all --limit 1 \
		--json state,url,headRefOid,mergeCommit \
		--jq '.[0] // empty | "\(.state) \(.url) \(.headRefOid) \(.mergeCommit.oid // "")"')
	read -r pr_state url head merge <<<"$pr"
	if [ -z "$pr_state" ]; then
		head=$(git ls-remote origin "refs/heads/$branch" | cut -f 1)
		if [ -n "$head" ]; then
			git fetch --quiet origin "refs/heads/$branch"
		fi
	fi
}

# exit_if_tagged
# Exits 0 if $tag is on origin at the merge of the bump PR, and 1 if it is on
# origin anywhere else.
exit_if_tagged() {
	local tagged
	tagged=$(git ls-remote origin "refs/tags/$tag" "refs/tags/$tag^{}" | tail -n 1 | cut -f 1)
	if [ -z "$tagged" ]; then
		return
	fi
	if [ "$tagged" = "$merge" ]; then
		progress "$tag is already tagged on ${merge:0:7}, the merge of $url, so there is nothing left to do"
		exit 0
	fi
	progress "$tag already exists on origin at ${tagged:0:7}, which is not the merge of a $branch PR, so it is left alone"
	exit 1
}

# push_bump
# Commits the version bump on main in a temporary worktree, pushes it to
# $branch and sets head to it.
push_bump() {
	local base
	base=$(git rev-parse --verify 'origin/main^{commit}')
	repo=$PWD
	work=$(mktemp -d)
	trap 'git -C "$repo" worktree remove --force "$work" 2>/dev/null || rm -rf "$work"' EXIT
	git worktree add --quiet --detach "$work" "$base"

	progress "bumping the version to $version on $branch from main at ${base:0:7}"
	(
		cd "$work"
		bump_version "$version"
		git commit --quiet --all --message "Release $version"
	)
	head=$(git -C "$work" rev-parse HEAD)
	git push --quiet origin "$head:refs/heads/$branch"
}

# open_pr
# Opens the bump PR for $head and sets url to it.
open_pr() {
	local base body
	base=$(git merge-base "$head" origin/main)
	progress "writing the summary with claude"
	# Built first so a failure to build it stops the script, as it would not
	# inside the gh command line.
	body=$(pr_body "$base" "$head" "$tag")
	url=$(gh pr create --base main --head "$branch" --title "Release $version" \
		--body "$body" | tail -n 1)
	progress "opened $url"
}

# wait_and_merge
# Waits for CI on $head, then merges its PR and sets merge to the merge
# commit, or leaves the PR open and exits 1 if CI failed.
wait_and_merge() {
	local failed
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
}

progress() {
	echo "release: $*" >&2
}

fetch_main() {
	git fetch --quiet origin +refs/heads/main:refs/remotes/origin/main
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

# pr_body <base> <head> <tag>
# The body of the bump PR on <head>, which <tag> will name, off <base>: the
# summary, between markers the Release page step can find, then the version
# diff.
pr_body() {
	local diff summary
	diff=$(version_diff "$1" "$2")
	summary=$(release_summary "$1" "$3" "$diff")
	cat <<-EOF
		## Summary

		<!-- release-summary:start -->
		$summary
		<!-- release-summary:end -->

		$diff
	EOF
}

# version_diff <base> <head>
# The version diff section: <head>'s changes from <base>.
version_diff() {
	cat <<-EOF
		## Version diff

		\`\`\`diff
		$(git diff --no-color --unified=1 "$1" "$2")
		\`\`\`
	EOF
}

# release_summary <base> <tag> <version diff section>
# The agent's summary of the release, or, if the agent fails or prints
# nothing, a note saying so and GitHub's generated notes.
release_summary() {
	local previous input summary
	previous=$(git describe --tags --abbrev=0 --match 'v[0-9]*' "$1" 2>/dev/null || true)
	input=$(summary_input "$1" "$previous" "$3")
	# In an if, so the agent failing falls back rather than stopping the script.
	if summary=$({ cat "$summary_prompt" && echo && echo "$input"; } |
		claude -p --tools '' --strict-mcp-config) &&
		[ -n "${summary//[[:space:]]/}" ]; then
		echo "$summary"
		return
	fi
	progress "warning: the agent summary is unavailable, so the summary is GitHub's generated notes"
	echo "_The agent summary was unavailable, so this is GitHub's generated notes._"
	echo
	gh api 'repos/{owner}/{repo}/releases/generate-notes' \
		-f tag_name="$2" -f target_commitish="$1" \
		${previous:+-f previous_tag_name="$previous"} --jq .body
}

# summary_input <base> <previous tag> <version diff section>
# What the agent summarises: the version diff, then the title, number and body
# of each PR merged into <base> since <previous tag>, oldest first. The bump PR
# isn't merged yet, so it isn't among them.
summary_input() {
	local number title body
	cat <<-EOF
		$3

		## Pull requests merged since ${2:-the first commit}
	EOF
	git log --reverse --merges --format=%s "${2:+$2..}$1" |
		sed -n 's/^Merge pull request #\([0-9][0-9]*\) from .*/\1/p' |
		while read -r number; do
			title=$(gh pr view "$number" --json title --jq .title)
			body=$(gh pr view "$number" --json body --jq .body)
			printf '\n### #%s: %s\n\n%s\n' "$number" "$title" "$body"
		done
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
