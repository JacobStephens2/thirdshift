# A Security finding stays private until it is fixed, and a Security run may fix a reproduced one in public

A public write-up of a vulnerability already on a public repository's **Base branch** discloses it, and a model can turn even a public fix into a working exploit within hours ([security-agent-passes.md](../research/security-agent-passes.md) §7.4). So a **Security run** never files a **Security finding** as an ordinary issue. It records the finding privately: a draft repository security advisory on a public repository, or an issue labelled `security-finding` on a private one. The **Run notification** names the finding without its write-up. GitHub's own private fix path can't run unattended: no CI runs in an advisory's temporary private fork, and only a button in the web UI merges it (§5.2). So when a flag or the **User config** allows it, the Security run fixes a finding that a proof-of-concept test has reproduced through an ordinary **Run** or **Spec run**. It works from terse public issues that link the private record and say only what the fix changes. The fix discloses the vulnerability when it is pushed. That window is the price of fixing without a human, and a finding nothing reproduces is never fixed unattended.

## Considered Options

- **Findings as ordinary issues, the way an Architecture review publishes an Architect plan.** Rejected. On a public repository the issue would publish an unfixed vulnerability, and a failed fix would leave that write-up public until the **Day shift** acted.
- **GitHub's temporary private fork for every fix.** Rejected as the factory's path, since CI can't run there and only a human can merge it. The Day shift can still use it by hand.
- **Report only, never fix unattended.** Kept as the default: fixing is off until a flag or the User config allows it, as a **Base fix** is.
- **Hold every fix for the Day shift's review.** Rejected as the only mode. A fix waiting in a public pull request discloses the vulnerability for as long as it waits, and on a repository that deploys on merge, a quick **Merge run** is the shortest exposure.

## Consequences

- The Day shift triages the private record, not an issue, and its triage ends a finding's wait: closing the draft advisory, publishing it or giving it a severity, or taking `needs-triage` off the private issue.
- GitHub keeps repository security advisories for public repositories, so a private repository's finding is an issue, private along with its repository.
- A Run notification passes through Resend, a third party, so it carries a finding's severity, title and link, never its write-up.
- A **Security review** that meets a vulnerability already on the Base branch sends it to the same private record, out of its Run's pull request.
