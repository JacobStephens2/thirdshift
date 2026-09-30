#!/usr/bin/env bash
# Prints the body of the GitHub Release for <tag>: the summary section of the
# bump PR that release.sh merged as <tag>'s commit, then GitHub's generated
# notes, then <install notes>, dist's install instructions. If no PR was merged
# as that commit (say, a tag pushed by hand), or its body has no summary
# section, the body is the generated notes and the install instructions alone.
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
	local commit summary
	commit=$(git rev-parse --verify "$1^{commit}")
	# In an if, so failing to read the PR falls back rather than stopping the
	# script and leaving the Release with dist's body.
	if ! summary=$(bump_summary "$commit"); then
		echo "release-notes: warning: could not read the bump PR, so the body has no summary" >&2
		summary=
	fi
	if [ -n "${summary//[[:space:]]/}" ]; then
		printf '%s\n\n' "$summary"
	fi
	gh api 'repos/{owner}/{repo}/releases/generate-notes' -f tag_name="$1" --jq .body
	printf '\n%s\n' "$2"
}

# bump_summary <commit>
# The summary section of the body of the PR merged as <commit>, or nothing.
# GitHub lists every PR the commit belongs to, including one whose head it
# is, so only a PR whose merge commit it is counts.
bump_summary() {
	local number
	# Errors are returned explicitly, as the caller's if turns errexit off.
	number=$(gh api "repos/{owner}/{repo}/commits/$1/pulls" \
		--jq '.[] | "\(.merge_commit_sha) \(.number)"' |
		awk -v commit="$1" '$1 == commit && !number { number = $2 } END { print number }') ||
		return
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
