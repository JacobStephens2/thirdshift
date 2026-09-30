<!-- Generated from src/prompt.rs by src/prompts_page.rs; don't edit. Regenerate with UPDATE_PROMPTS=1 cargo test prompts_page -->

# CI-fix Repair

Starts a Repair session when CI fails on the pull request's head commit, listing each failed check.

```
CI failed on pull request <pull request URL> (branch <branch>, implementing <Issue URL>).

Failed checks:
- <failing check>: <check URL>

Read the failure logs (e.g. `gh run view <run-id> --log-failed`), find the root cause, and fix it. Do not skip, disable, or weaken tests or checks to make them pass.
Run the affected checks locally, commit, and push <branch>.

If a failure is not caused by this branch (it is flaky, or also fails on <base>), do not change code for it. Instead, add it to a "CI notes" section of the pull request body with a one-line explanation.

You run headless: nobody is watching, and ending your turn ends the session. Run tests and other long commands in the foreground, raising the Bash timeout if needed. Never end your turn while a background task you depend on is still running: ending the turn kills it.
```
