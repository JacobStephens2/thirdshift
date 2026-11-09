#!/usr/bin/env bash
# Prints the body of the GitHub Release for <tag>: the summary section of the
# origin release-<version> PR whose head is <tag>'s commit, or of the PR merged
# as that commit, then GitHub's generated notes, then <install notes>, dist's
# install instructions. If neither PR matches (say, a tag pushed by hand), or
# its body has no summary section, the body is the generated notes and the
# install instructions alone.
#
# Usage: scripts/release-notes.sh <tag> <install notes>
#
# release-notes.yml runs it in a checkout of <tag> once the Release is
# published. It builds the body from its inputs and the PR, never from the
# Release's current body, so running it again gives the same body.
set -euo pipefail
# So a failure inside $(…) stops the script too.
shopt -s inherit_errexit

main() {
	if [ "$#" -ne 2 ]; then
		echo "usage: release-notes.sh <tag> <install notes>" >&2
		exit 2
	fi
	local commit summary branch=
	commit=$(git rev-parse --verify "$1^{commit}")
	# release.sh tags vX.Y.Z on release-X.Y.Z. Any other tag shape only
	# matches a pull request merged as the tagged commit.
	if [[ ${1#v} =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
		branch=release-${1#v}
	fi
	# In an if, so failing to read the PR falls back rather than stopping the
	# script and leaving the Release with dist's body.
	if ! summary=$(bump_summary "$commit" "$branch"); then
		echo "release-notes: warning: could not read the bump PR, so the body has no summary" >&2
		summary=
	fi
	if [ -n "${summary//[[:space:]]/}" ]; then
		printf '%s\n\n' "$summary"
	fi
	gh api 'repos/{owner}/{repo}/releases/generate-notes' -f tag_name="$1" --jq .body
	printf '\n%s\n' "$2"
}

# bump_summary <commit> <release branch>
# The summary section of the origin pull request for <release branch> whose
# head is <commit>, or of the pull request merged as <commit>. A pull request
# from another repository does not count, even when its head commit is the
# same. Nothing when no matching pull request has a summary section.
bump_summary() {
	local commit=$1 branch=$2 number
	# Errors are returned explicitly, as the caller's if turns errexit off.
	# `none` is not a commit or a repository name, so a missing field cannot
	# match. The highest number is the newest matching pull request.
	number=$(gh api "repos/{owner}/{repo}/commits/$commit/pulls" \
		--jq '.[] | [(.merge_commit_sha // "none"), .head.sha, .head.ref, (.head.repo.full_name // "none"), (.base.repo.full_name // "none"), (.number | tostring)] | join(" ")' |
		awk -v commit="$commit" -v branch="$branch" '
			function release_head() {
				return branch != "" && $2 == commit && $3 == branch && $4 == $5 && $4 != "none"
			}
			($1 == commit || release_head()) && ($6 + 0 > best) { best = $6 + 0 }
			END { if (best) print best }
		') || return
	if [ -n "$number" ]; then
		gh pr view "$number" --json body --jq .body | summary_section
	fi
}

# summary_section
# The lines of the PR body on stdin between the markers release.sh's pr_body
# writes around the summary, or nothing if either marker is missing.
summary_section() {
	awk '
		{ sub(/\r$/, "") }
		$0 == "<!-- release-summary:end -->" && inside { printf "%s", text; inside = 0; done = 1 }
		inside { text = text $0 "\n" }
		$0 == "<!-- release-summary:start -->" && !done { inside = 1 }'
}

# Only when run, so the tests can source the script and call its functions.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
	main "$@"
fi
