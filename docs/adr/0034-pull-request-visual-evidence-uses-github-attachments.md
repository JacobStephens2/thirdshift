# Pull request visual evidence uses GitHub attachments

Issue #2023 replaces #1092's secret-Gist hosting for pull request **Visual evidence** (`Before`/`After` `.png` screenshots and H.264 `.mp4` walkthrough recordings) with GitHub CLI attachments (`gh pr create --attach` and `gh pr edit --attach`, requiring `gh >= 2.103.2` so repository-scoped GitHub App installation tokens work too). GitHub uploads each validated local capture to `uploads.github.com/user-attachments/assets`, renders native inline images and `<video>` players in the pull request body, and enforces the repository's own read permissions automatically across private and public repositories without adding media blobs to git history.

## Considered Options

- **Commit captures on the branch (`docs/screenshots/**` or `site/screenshots/**`):** Bloats repository clones; in #1603 `docs/screenshots/**` triggered conservative 34-minute full Rust runs in `scripts/affected-tests`, and in #1624 `site/screenshots/**` deployed PR evidence to the public site.
- **Secret git-pushed Gists (#1092):** Unlisted capability URLs with no viewer login (unsuitable for private team repositories), owned by a personal `gh` account, incompatible with repository-scoped GitHub App installation tokens (`gist` is user-only), and `gist.githubusercontent.com` serves `.mp4`/`.webm` with `X-Content-Type-Options: nosniff` rather than an inline player.
- **Unmerged custom git refs (`refs/thirdshift/evidence/*`):** Inherits repository permissions and stays out of default `git clone`/`git fetch` refspecs, but `raw.githubusercontent.com` serves `.mp4`/`.webm` as `application/octet-stream` / `audio/webm` with `Content-Disposition: attachment` and `nosniff` (forcing file downloads instead of in-browser playback) while growing remote packfiles.
- **Serve recordings from `thirdshift console`:** Requires adding HTTP `Range` (`206 Partial Content`) streaming, larger upload limits and timeouts, disk pruning, per-teammate `allowed_emails` onboarding, and a separate story for external reviewers on public repositories.

## Consequences

- Uploads to `user-attachments` have no delete API (`cli/cli` `attach.go`): sessions must validate regular-file ownership, non-symlink paths, size caps (`<= 8 MB`, inside GitHub's `10 MB` free-plan limit), and faux-only test data before calling `gh --attach`, never uploading exploratory scratch captures.
- Hosts running UI verification need `gh >= 2.103.2`, headless Chromium, and `ffmpeg` (`-c:v libx264 -pix_fmt yuv420p -movflags +faststart`), checked by `thirdshift setup`, and #1486's `bwrap` Run sandbox proxy allowlist must permit `uploads.github.com` alongside `github.com` and `api.github.com`.
- GitHub Enterprise Server is unsupported by `gh --attach`; private-repository `user-attachments` URLs also require authenticated browser sessions rather than unauthenticated `curl`.
