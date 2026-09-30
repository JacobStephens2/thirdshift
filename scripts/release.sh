#!/usr/bin/env bash
# Cuts a thirdshift release: opens a PR into main that bumps the version and
# nothing else, waits for its checks, merges it with a merge commit, and tags
# that merge commit v<version>. Pushing the tag starts the dist release
# workflow.
#
# Usage: scripts/release.sh [--review] <version>
#
# Run it in a clone of this repo, signed in to gh and with claude logged in. It
# works from origin/main in a temporary worktree, so the branch checked out
# where it runs and any uncommitted changes there are neither used nor changed.
#
# claude, with no tools, writes the PR's summary from the prompt in
# release-summary.md beside this script. If it fails, the summary is GitHub's
# generated notes instead, and the script warns and carries on. With --review,
# it then prints the summary and asks whether to carry on with it, edit it in
# $EDITOR first, or stop with nothing pushed. Without it, it never reads stdin.
#
# Before it pushes anything, it refuses a release that can't be cut: a version
# that isn't plain X.Y.Z, or isn't higher than the one on main, a v<version>
# tag that already exists locally or on origin, or a latest CI run on main that
# didn't succeed.
#
# If a run is interrupted, running it again with the same version carries on
# from the first step not yet done: it opens the PR for a pushed branch, waits
# on and merges an open PR, or tags a merged one, without the refusals above,
# which the first run passed. If v<version> is already on origin, on the merge
# of the bump PR, it says so and exits 0; a v<version> tag anywhere else is
# refused.
#
# Prints a progress line on stderr for each step. Exits 0 once the tag is
# pushed. It exits 1 with nothing more pushed if it refuses or the review says
# no, and exits 1 leaving the PR open if the PR's checks fail.
set -euo pipefail
# So a failure inside $(…) stops the script too.
shopt -s inherit_errexit

# The prompt the summary agent gets ahead of the release's input.
summary_prompt=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/release-summary.md

# Seconds between reads of the PR's workflow runs.
poll_seconds=${RELEASE_POLL_SECONDS:-15}

main() {
	review=
	if [ "${1-}" = --review ]; then
		review=1
		shift
	fi
	if [ "$#" -ne 1 ]; then
		echo "usage: release.sh [--review] <version>" >&2
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
			commit_bump
			write_summary "nothing was pushed"
			git push --quiet origin "$head:refs/heads/$branch"
		else
			progress "resuming: $branch is on origin at ${head:0:7} with no PR"
			write_summary "no PR was opened"
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
# After an earlier run, exits 0 if $tag is on origin at the merge of the bump
# PR, and refuses if it is on origin anywhere else. With no earlier run,
# check_can_release refuses an existing tag in its turn.
exit_if_tagged() {
	local tagged
	if [ -z "$pr_state" ] && [ -z "$head" ]; then
		return
	fi
	tagged=$(git ls-remote origin "refs/tags/$tag" "refs/tags/$tag^{}" | tail -n 1 | cut -f 1)
	if [ -z "$tagged" ]; then
		return
	fi
	if [ "$tagged" = "$merge" ]; then
		progress "$tag is already tagged on ${merge:0:7}, the merge of $url, so there is nothing left to do"
		exit 0
	fi
	refuse "$tag already exists on origin at ${tagged:0:7}, which is not the merge of a $branch PR"
}

# commit_bump
# Refuses a release that can't be cut, then commits the version bump on main
# in a temporary worktree and sets head to it.
commit_bump() {
	local base
	base=$(git rev-parse --verify 'origin/main^{commit}')
	check_can_release "$version" "$tag" "$base"
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
}

# write_summary <what stopping leaves undone>
# Sets diff to the version diff of $head and summary to claude's summary of
# it, then with --review, asks about the summary and exits 1 on no, saying
# <what stopping leaves undone>.
write_summary() {
	local base
	base=$(git merge-base "$head" origin/main)
	progress "writing the summary with claude"
	diff=$(version_diff "$base" "$head")
	summary=$(release_summary "$base" "$tag" "$diff")
	if [ -n "$review" ] && ! summary=$(review_summary "$summary"); then
		progress "stopped at the review, so $1"
		exit 1
	fi
}

# open_pr
# Opens the bump PR for $branch with $summary and $diff and sets url to it.
open_pr() {
	url=$(gh pr create --base main --head "$branch" --title "Release $version" \
		--body "$(pr_body "$summary" "$diff")" | tail -n 1)
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

# pr_body <summary> <version diff section>
# The body of the bump PR: the summary, between markers release-notes.sh finds
# to head the Release page with it, then the version diff.
pr_body() {
	cat <<-EOF
		## Summary

		<!-- release-summary:start -->
		$1
		<!-- release-summary:end -->

		$2
	EOF
}

# review_summary <summary>
# Prints <summary> on stderr and asks on stdin whether to carry on with it,
# edit it in $EDITOR, or stop. Prints the summary to carry on with; returns
# non-zero to stop, on no, at the end of stdin, or if the editor fails.
# Called in a condition, where set -e is off, so each step's failure is
# handled here.
review_summary() {
	local answer file status
	printf '\n%s\n\n' "$1" >&2
	while :; do
		printf 'release: open the PR with this summary? [y]es / [e]dit / [n]o ' >&2
		read -r answer || return
		case $answer in
		y | yes)
			echo "$1"
			return
			;;
		e | edit)
			file=$(mktemp --suffix=.md) || return
			status=0
			# $EDITOR runs as git runs it, so it may carry arguments. Its
			# output goes to stderr, as stdout is the summary.
			{
				echo "$1" >"$file" &&
					sh -c "${EDITOR:-vi} \"\$1\"" "${EDITOR:-vi}" "$file" >&2 &&
					cat "$file"
			} || status=$?
			rm -f "$file"
			return "$status"
			;;
		n | no)
			return 1
			;;
		esac
	done
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
