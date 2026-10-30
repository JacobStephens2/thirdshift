#!/usr/bin/env bash
# Cuts a thirdshift release: opens a PR into main that bumps the version and
# nothing else, waits for its checks, and tags that bump commit v<version>.
# Pushing the tag starts the dist release workflow. It waits for that workflow
# to succeed before merging the PR into main with a merge commit.
#
# Usage: scripts/release.sh [--review] [--with-experiment] <version>
#
# Run it in a clone of this repo, signed in to gh and the default Harness. It
# works from origin/main in a temporary worktree, so the branch checked out
# where it runs and any uncommitted changes there are neither used nor changed.
#
# The User config's default Harness writes the PR's summary from the prompt in
# release-summary.md beside this script. If it fails, the summary is GitHub's
# generated notes instead, and the script warns and carries on. With --review,
# it then prints the summary and asks whether to carry on with it, edit it in
# $EDITOR first, or stop with nothing pushed. A failed or inconclusive release
# Experiment also asks whether to carry on, even without --review.
#
# Before it pushes anything, it refuses a release that can't be cut: a version
# that isn't plain X.Y.Z, or isn't higher than the one on main, a v<version>
# tag that already exists locally or on origin. The bump starts from the newest
# main commit whose latest push CI run succeeded, newer than the last release
# tag. If none qualifies, it waits for a run to succeed, still considering older
# pending commits when main moves. It never waits just for the tip to go green.
# With --with-experiment, an Experiment compares that content commit with its
# last release tag, selecting touched suites. A pass carries on; pending Scores
# are listed and waited for; fail/inconclusive Verdicts show intervals and
# require an override.
# Untouched suites or a Baseline with no Trial flag need no Experiment.
# Experiments are skipped by default, on a fresh run and on every resume,
# with a progress note. Supply --with-experiment on each invocation to run one.
#
# If a run is interrupted, running it again with the same version carries on
# from the first step not yet done: it opens the PR for a pushed branch, waits
# on an open PR, tags its bump commit, waits for publication, then merges it,
# without the refusals
# above, which the first run passed. A merged PR is tagged at its bump head too.
# If v<version> is already on origin at the bump head, it waits for the release
# workflow, then merges an open PR. If the workflow succeeded or the GitHub
# Release exists, it merges immediately. A bump PR
# closed without merging is refused while release-<version> is on origin;
# once that branch is deleted, the next run starts over.
# A resumed PR must belong to origin and match its branch tip while open, or
# it is refused, even once tagged. Its tree (or a pushed branch with no PR)
# must also be exactly the version bump in Cargo.toml and Cargo.lock from
# its merge base, or it is refused before merging or tagging.
# A pushed branch is checked against the same release Experiment before its
# PR is opened. A release already tagged normally skips checks and the Experiment.
# On resume after a failed unpublished workflow, it re-cuts from the newest
# main commit newer than the bump's parent whose latest push CI succeeded:
# it runs the Experiment when requested, regenerates the summary and version
# diff (honoring --review), force-pushes the branch, updates the open PR body,
# waits for its checks, then force-pushes the tag and waits for publication.
# An old merged bump gets a new PR. If no newer green commit exists, it reruns
# the failed jobs and waits. Published releases are never moved. A recovery
# interrupted between branch and tag pushes resumes that exact version bump;
# unrelated tags are refused.
# Before the branch exists, a local Git ref pins the content commit so a
# restart keeps the same Candidate even if main advances during interruption.
# Main can gain content after selection: the tagged tree is still exactly the
# Candidate plus the version bump, and merging later preserves it in history.
#
# Prints a progress line on stderr for each step, stamped with the local time
# as HH:MM:SS, with the release workflow run's URL once it waits for that run,
# and the GitHub Release's URL last.
# Exits 0 once the release workflow run for the tag has succeeded: the GitHub
# Release is published with its final body and the crate is on crates.io. It
# exits 1 with nothing more pushed if it refuses or the review says no, exits
# 1 leaving the PR open if the PR's checks fail, and exits 1 leaving the tag in
# place and the PR open if the release workflow run fails, naming its failed
# jobs and run URL. Recovery is only attempted on a subsequent invocation.
# A failed merge after publication leaves the release standing, names
# the PR for the Day shift and still exits 0.
# It waits with no overall timeout, so Ctrl-C and running it again
# with the same version carries on waiting.
set -euo pipefail
# So a failure inside $(…) stops the script too.
shopt -s inherit_errexit

# The prompt the summary agent gets ahead of the release's input.
summary_prompt=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/release-summary.md
summary_manifest=$(dirname "$summary_prompt")/../Cargo.toml

# Seconds between reads of main's CI, the PR's runs and the release workflow.
poll_seconds=${RELEASE_POLL_SECONDS:-15}

main() {
	review=
	with_experiment=
	while :; do
		case ${1-} in
		--review) review=1 ;;
		--with-experiment) with_experiment=1 ;;
		*) break ;;
		esac
		shift
	done
	if [ "$#" -ne 1 ]; then
		echo "usage: release.sh [--review] [--with-experiment] <version>" >&2
		exit 2
	fi
	version=$1
	tag=v$version
	branch=release-$version

	fetch_main
	find_earlier_run
	case $pr_state in
	OPEN | MERGED) validate_bump_pr ;;
	esac
	find_existing_tag

	case $pr_state in
	"")
		if [ -z "$head" ]; then
			commit_bump
			write_summary "nothing was pushed"
			git push --quiet origin "$head:refs/heads/$branch"
			git update-ref -d "refs/thirdshift/release-candidates/$version"
		else
			progress "resuming: $branch is on origin at ${head:0:7} with no PR"
			validate_bump_pr
			if [ -z "$tagged" ]; then wait_for_experiment "$(git rev-parse "$head^1")"; fi
			write_summary "no PR was opened"
		fi
		open_pr
		;;
	OPEN)
		progress "resuming: $url is open"
		if [ -z "$tagged" ]; then wait_for_experiment "$(git rev-parse "$head^1")"; fi
		;;
	MERGED)
		progress "resuming: $url was merged as ${merge:0:7}"
		if [ -z "$tagged" ]; then wait_for_experiment "$(git rev-parse "$head^1")"; fi
		;;
	*)
		refuse "$url was closed without merging, so there is nothing to resume; reopen it or delete $branch to start over"
		;;
	esac

	published_url=
	if [ -n "$tagged" ]; then recover_failed_release; fi
	if [ -z "$tagged" ]; then
		if [ "$pr_state" != MERGED ]; then wait_for_pr_checks; fi
		git push --quiet origin "$head:refs/tags/$tag"
		progress "tagged $tag on ${head:0:7} and pushed the tag"
	fi
	if [ -z "$published_url" ]; then
		wait_for_release
		published_url=$(gh release view "$tag" --json url --jq .url)
	fi
	if [ "$pr_state" != MERGED ]; then merge_bump_pr; fi
	progress "released $tag at $published_url"
}

# recover_failed_release
# Only a resumed invocation recovers a completed failed workflow. An existing
# GitHub Release is final, even if a later publication job failed.
recover_failed_release() {
	local run id run_url status conclusion base
	published_url=$(gh release view "$tag" --json url --jq .url 2>/dev/null || true)
	if [ -n "$published_url" ]; then return; fi
	run=$(release_run "$tagged")
	read -r id run_url status conclusion <<<"$run"
	if [ "$status" != completed ] || [ "$conclusion" = success ]; then return; fi
	base=$(newest_green_main "$(git rev-parse "$head^1")")
	if [ -n "$base" ] || [ "$head" != "$tagged" ]; then
		if [ -n "$base" ]; then
			progress "re-cutting unpublished $tag from main at ${base:0:7}"
			commit_bump "$base"
		else
			wait_for_experiment "$(git rev-parse "$head^1")"
		fi
		write_summary "the release branch and tag were not moved"
		refuse_if_published
		git push --force --quiet origin "$head:refs/heads/$branch"
		if [ "$pr_state" = MERGED ]; then
			open_pr
			pr_state=OPEN
		else
			edit_pr
		fi
		wait_for_pr_checks
		refuse_if_published
		git push --force --quiet origin "$head:refs/tags/$tag"
		tagged=$head
		progress "moved unpublished $tag to ${head:0:7} and pushed the tag"
		return
	fi
	gh run rerun --failed "$id"
	progress "reran the failed jobs of the release workflow at $run_url"
}

# Publication can finish while the Experiment, summary or PR checks run.
refuse_if_published() {
	if gh release view "$tag" --json url --jq .url >/dev/null 2>&1; then
		refuse "$tag is already published; its branch and tag will not be moved"
	fi
}

# find_earlier_run
# Sets what an earlier run for this version got done: pr_state (OPEN, MERGED,
# CLOSED, or empty with no PR), url, head and merge from the newest PR from
# $branch, or with no PR, head from $branch on origin if it was pushed. A PR
# closed without merging whose branch is gone from origin counts as no PR.
# Refuses a PR from another repository, or an open PR whose head is not
# $branch on origin.
find_earlier_run() {
	local pr cross_repository origin_head
	pr=$(gh pr list --repo "$(git remote get-url origin)" --head "$branch" --base main --state all --limit 1 \
		--json state,url,headRefOid,mergeCommit,isCrossRepository \
		--jq '.[0] // empty | "\(.isCrossRepository) \(.state) \(.url) \(.headRefOid) \(.mergeCommit.oid // "")"')
	read -r cross_repository pr_state url head merge <<<"$pr"
	origin_head=$(git ls-remote origin "refs/heads/$branch" | cut -f 1)
	# A PR closed without merging whose branch was then deleted leaves
	# nothing to resume, whichever repository it came from: start over.
	if [ "$pr_state" = CLOSED ] && [ -z "$origin_head" ]; then
		pr_state='' url='' head='' merge=''
	fi
	if [ -n "$pr_state" ] && [ "$cross_repository" != false ]; then
		refuse "$url has its head in another repository, not origin"
	fi
	if [ "$pr_state" = OPEN ] && { [ -z "$origin_head" ] || [ "$head" != "$origin_head" ]; }; then
		refuse "$url head does not match refs/heads/$branch on origin"
	fi
	if [ -z "$pr_state" ]; then
		head=$origin_head
		if [ -n "$head" ]; then
			git fetch --quiet origin "refs/heads/$branch"
		fi
	fi
}

# validate_bump_pr
# Refuses unless the selected PR's (or pushed branch's) entire tree is the
# bump_version result from its merge base. A merged PR uses main just before
# its merge, since its head is already an ancestor of main now.
validate_bump_pr() {
	local base target expected candidate=${url:-$branch on origin}
	if ! git fetch --quiet origin "$head"; then
		refuse "$candidate head could not be fetched from origin"
	fi
	target=origin/main
	if [ "$pr_state" = MERGED ]; then
		target=$merge^1
	fi
	base=$(git merge-base "$head" "$target")
	expected=$(bump_tree "$base")
	if [ "$expected" != "$(git rev-parse "$head^{tree}")" ]; then
		refuse "$candidate changes more than the version bump to $version in Cargo.toml and Cargo.lock"
	fi
}

# bump_tree <base>
# The tree commit_bump would make, without committing or touching the caller's
# checkout. The subshell owns the temporary worktree and its cleanup trap.
bump_tree() (
	local repo=$PWD work
	work=$(mktemp -d)
	trap 'git -C "$repo" worktree remove --force "$work" 2>/dev/null || rm -rf "$work"' EXIT
	git worktree add --quiet --detach "$work" "$1"
	cd "$work"
	bump_version "$version"
	git add Cargo.toml Cargo.lock
	git write-tree
)

# find_existing_tag
# Recognises an earlier tag at the bump head, or an unpublished failed bump
# left behind by an interrupted recovery. With no earlier run,
# check_can_release refuses an existing tag in its turn.
find_existing_tag() {
	local old_base base run id run_url status conclusion
	tagged=
	if [ -z "$pr_state" ] && [ -z "$head" ]; then
		return
	fi
	tagged=$(git ls-remote origin "refs/tags/$tag" "refs/tags/$tag^{}" | tail -n 1 | cut -f 1)
	if [ -z "$tagged" ]; then
		return
	fi
	if [ "$tagged" = "$head" ]; then
		progress "$tag is already tagged on ${head:0:7}, the head of $branch"
		return
	fi
	# A recovery can stop after pushing the new branch but before moving its
	# tag. Accept only two exact bumps, a strictly newer content base, and an
	# unpublished failed workflow on the old tag; unrelated tags still refuse.
	git fetch --quiet origin "$tagged"
	old_base=$(git rev-parse "$tagged^1")
	base=$(git rev-parse "$head^1")
	run=$(release_run "$tagged")
	read -r id run_url status conclusion <<<"$run"
	if [ "$pr_state" = OPEN ] && [ "$old_base" != "$base" ] &&
		git merge-base --is-ancestor "$old_base" "$base" &&
		[ "$(bump_tree "$old_base")" = "$(git rev-parse "$tagged^{tree}")" ] &&
		[ "$status" = completed ] && [ "$conclusion" != success ] &&
		! gh release view "$tag" --json url --jq .url >/dev/null 2>&1; then
		progress "resuming re-cut of unpublished $tag on ${head:0:7}"
		return
	fi
	refuse "$tag already exists on origin at ${tagged:0:7}, which is not the head of $branch"
}

# commit_bump [content commit]
# Selects and saves main's checked content commit, or reuses that selection
# after interruption, then commits the bump in a worktree and sets head to it.
commit_bump() {
	local base=${1-} candidate_ref=refs/thirdshift/release-candidates/$version
	if [ -z "$base" ]; then
		if is_plain_version "$version" && base=$(git rev-parse --quiet --verify "$candidate_ref^{commit}"); then
			progress "resuming release Experiment from saved content commit ${base:0:7}"
		else
			base=$(check_can_release "$version" "$tag")
			# Keep the selection even if the Experiment is interrupted before a
			# branch exists. The empty old value refuses a competing selection.
			git update-ref "$candidate_ref" "$base" ''
		fi
	fi
	# The content commit is stable across interruptions before the push;
	# the version-only bump made below adds no Eval suite dependencies.
	wait_for_experiment "$base"
	repo=$PWD
	work=$(mktemp -d)
	trap 'git -C "$repo" worktree remove --force "$work" 2>/dev/null || rm -rf "$work"' EXIT
	git worktree add --quiet --detach "$work" "$base"

	progress "bumping the version to $version on $branch from main at ${base:0:7}"
	(
		cd "$work"
		bump_version "$version"
		# An older invocation may already have merged this version onto main.
		git commit --quiet --allow-empty --all --message "Release $version"
	)
	head=$(git -C "$work" rev-parse HEAD)
}

# wait_for_experiment <content commit>
# Use this checkout's runner and private Eval suites, as for summary_agent.
wait_for_experiment() {
	local snapshot status outcome id answer
	if [ -z "$with_experiment" ]; then
		progress "skipping the release Experiment (use --with-experiment to run it) for ${1:0:7}"
		return
	fi
	progress "checking release Experiment for ${1:0:7}"
	snapshot=$(cargo run --quiet --manifest-path "$summary_manifest" -- experiment --release "$1" --json --yes)
	while :; do
		status=$(printf '%s\n' "$snapshot" | experiment_status)
		read -r outcome id <<<"$status"
		case $outcome in
		pass | skip) return ;;
		waiting_on_rechecks)
			sleep "$poll_seconds"
			snapshot=$(cargo run --quiet --manifest-path "$summary_manifest" -- experiments "$id" --json)
			;;
		fail | inconclusive)
			{ stamp && printf 'Experiment %s; release anyway? [y/N] ' "$outcome"; } >&2
			read -r answer || refuse "Experiment $outcome; no release override given"
			case $answer in
			y | yes) return ;;
			*) refuse "Experiment $outcome; release declined" ;;
			esac
			;;
		*) refuse "unexpected release Experiment state: $outcome" ;;
		esac
	done
}

# experiment_status
# Keep the renderer's state on stdout and stamp its progress through the same
# helper as every other release step. Preserve parser diagnostics on failure.
experiment_status() {
	local rendered result=0 line
	rendered=$(python3 "$(dirname "$summary_prompt")/release-experiment.py" 2>&1) || result=$?
	while IFS= read -r line; do
		case $line in
		"release: "*) progress "${line#release: }" ;;
		*)
			if [ "$result" -eq 0 ]; then
				printf '%s\n' "$line"
			else
				printf '%s\n' "$line" >&2
			fi
			;;
		esac
	done <<<"$rendered"
	return "$result"
}

# write_summary <what stopping leaves undone>
# Sets diff to the version diff of $head and summary to the Harness's summary of
# it, then with --review, asks about the summary and exits 1 on no, saying
# <what stopping leaves undone>.
write_summary() {
	local base
	base=$(git merge-base "$head" origin/main)
	progress "writing the summary with the configured Harness"
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

# The subshell owns the temporary PR body even if gh fails.
edit_pr() (
	local body_file
	body_file=$(mktemp)
	trap 'rm -f "$body_file"' EXIT
	pr_body "$summary" "$diff" >"$body_file"
	gh pr edit "$url" --body-file "$body_file" >/dev/null
)

# wait_for_pr_checks
# Leaves the PR open and untagged and exits 1 if its checks failed.
wait_for_pr_checks() {
	local failed tag_state="no tag was pushed"
	progress "waiting for CI on $url"
	failed=$(wait_for_workflow_runs "$head")
	if [ -n "$failed" ]; then
		if [ -n "$tagged" ]; then tag_state="the existing tag is unchanged"; fi
		progress "CI failed on $url ($failed), so the PR is left open and unmerged; $tag_state"
		exit 1
	fi
}

# merge_bump_pr
# The tag is already public. A failed merge leaves the release standing and
# names the PR for the Day shift; publication has already succeeded.
merge_bump_pr() {
	if gh pr merge "$url" --merge --match-head-commit "$head" >/dev/null; then
		progress "merged $url"
	else
		progress "warning: $tag is left in place and the release still stands; the Day shift must merge $url"
	fi
}

# wait_for_release
# Waits for the release workflow run that pushing $tag started on $head to
# be listed, then to complete. If it failed, exits 1 naming the jobs that
# failed, leaving the tag in place and the bump PR unmerged. Recovery waits
# for the next invocation so the operator sees this failure first.
# The run's status is that of its current attempt, so after gh run rerun
# --failed, the rerun's. CI's run on main for the merge is not waited for.
wait_for_release() {
	local run id run_url status conclusion failed
	run=$(release_run)
	if [ -z "$run" ]; then
		progress "waiting for the release workflow on $tag to start"
	fi
	while [ -z "$run" ]; do
		sleep "$poll_seconds"
		run=$(release_run)
	done
	read -r id run_url status conclusion <<<"$run"
	progress "waiting for the release workflow at $run_url"
	while [ "$status" != completed ]; do
		sleep "$poll_seconds"
		read -r id run_url status conclusion <<<"$(release_run)"
	done

	if [ "$conclusion" != success ]; then
		failed=$(gh run view "$id" --json jobs --jq '[.jobs[] | select(.conclusion != "success" and .conclusion != "skipped" and .conclusion != "neutral") | .name] | join(", ")')
		progress "the release workflow ended in $conclusion${failed:+ ($failed)} at $run_url, so $tag is left in place; run scripts/release.sh $version again to recover, or rerun the jobs manually with gh run rerun --failed $id"
		exit 1
	fi
}

# release_run
# The newest release workflow run on the supplied commit (default $head) from a push, as
# "<id> <url> <status> <conclusion>", or nothing if it isn't listed yet.
release_run() {
	gh run list --workflow release.yml --event push --commit "${1:-$head}" --limit 1 \
		--json databaseId,url,status,conclusion \
		--jq '.[] | "\(.databaseId) \(.url) \(.status) \(.conclusion)"'
}

# progress <message>
# Prints <message> on stderr as a progress line.
progress() {
	{ stamp && printf '%s\n' "$*"; } >&2
}

# stamp
# What every progress line starts with: the prefix, then the local time it was
# printed as HH:MM:SS, as thirdshift stamps its own, so a long wait can be told
# from a stalled one.
stamp() {
	printf 'release: %(%H:%M:%S)T ' -1
}

# refuse <reason>
refuse() {
	progress "refusing to release $version: $*"
	exit 1
}

# check_can_release <version> <tag>
# Refuses unless <version> is plain X.Y.Z and higher than the version in the
# manifest on main, and no <tag> exists on origin or locally. Prints the newest
# main commit since the last release whose latest push CI succeeded. When none
# is green yet, polls all candidates so moving main never restarts the wait.
check_can_release() {
	local current base previous waiting=
	if ! is_plain_version "$1"; then
		refuse "it is not a plain X.Y.Z version, such as 0.4.0"
	fi

	fetch_main
	current=$(git show origin/main:Cargo.toml | package_field version)
	if ! is_higher "$1" "$current"; then refuse "it is not higher than $current on main"; fi
	if [ -n "$(git ls-remote --tags origin "refs/tags/$2")" ]; then refuse "$2 already exists on origin"; fi
	if git rev-parse --quiet --verify "refs/tags/$2" >/dev/null; then refuse "$2 already exists locally"; fi
	previous=$(git describe --tags --abbrev=0 --match 'v[0-9]*' origin/main 2>/dev/null || true)
	while :; do
		base=$(newest_green_main "$previous")
		if [ -n "$base" ]; then printf '%s\n' "$base"; return; fi
		if [ -z "$waiting" ]; then
			progress "waiting for CI on main: no successful push run newer than ${previous:-the first commit}"
			waiting=1
		fi
		sleep "$poll_seconds"
		fetch_main
	done
}

# newest_green_main <exclusive ancestor>
# The newest main commit since that ancestor whose latest push CI succeeded,
# or nothing. Recovery does not wait for a newer commit to turn green.
newest_green_main() {
	local base latest sha status conclusion
	for base in $(git rev-list --first-parent origin/main ${1:+^$1}); do
		latest=$(gh run list --branch main --workflow ci.yml --event push --commit "$base" --limit 1 \
			--json headSha,status,conclusion --jq '.[] | "\(.headSha) \(.status) \(.conclusion)"')
		read -r sha status conclusion <<<"$latest"
		if [ "$sha" = "$base" ] && [ "$status" = completed ] && [ "$conclusion" = success ]; then
			printf '%s\n' "$base"
			return
		fi
	done
}

# is_plain_version <version>
# Whether <version> is plain semver X.Y.Z, with no leading zeros.
is_plain_version() {
	[[ $1 =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]
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
		{ stamp && printf 'open the PR with this summary? [y]es / [e]dit / [n]o '; } >&2
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
		summary_agent) &&
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

# Use this checkout's config parser and Harness adapters, not a possibly older
# installed binary. Prompt on stdin, final assistant text on stdout.
summary_agent() {
	cargo run --quiet --manifest-path "$summary_manifest" -- --release-summary
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

# Only when run, so the tests can source the script and call its functions.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
	main "$@"
fi
