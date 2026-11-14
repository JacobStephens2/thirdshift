# Custom runners run inside one Trial sandbox

A Custom runner is an outside orchestrator used as either Arm of a process
Experiment. Its scripts and mapped skill trees are frozen before Trials and
verified by SHA-256 before each attempt. Source Git origin, path, HEAD and dirty
state are provenance; dirty local edits are valid inputs. Neither Arm needs a
thirdshift build when both are Custom runners. Decided in #1468; implementation
starts in #2188.

One Trial-wide sandbox and memory scope enclose the orchestrator and all its
children. Fresh HOME holds declared files, Harness logins and Trials-only
provider keys. A provider-only proxy and Harness wrappers enforce the same
network and web-search boundary as native Trials. These multi-provider sandbox
adapters are delivered by #2189; the execution contract in #2188 is their seam.

The runner receives a versioned request, works unattended, commits its change
on HEAD and writes a versioned result. Code Graders and Pair evidence read the
commit, ignoring uncommitted files. Stopping to ask fails its Score; no simulated
user supplies answers. Only session unavailability, usage limits and rate limits
may be self-reported as Void. Outer supervision owns interruption, sandbox
failure, memory-cap and host-kill evidence. A broken runner protocol fails one
Trial rather than aborting the Experiment.

Session evidence is converted to the standard traces and priced from the frozen
Experiment price table (#2191). Declared roles describe configuration; observed
Models decide pricing. Setup identity includes the frozen runner digest, with
no thirdshift commit or single Harness on a Custom-runner Arm.

## Considered Options

- **Run the outside workflow as a top-level Command.** Rejected: it would
  publish factory work and mix Trial evidence into production metrics.
- **Sandbox every child session separately.** Rejected: the orchestrator could
  read the host or network outside those sessions and bypass Trial boundaries.
- **Grade a returned patch or the working tree.** Rejected: committed HEAD
  handles binary files, renames and several commits while excluding scratch.
- **Refuse dirty source repositories.** Rejected: freezing records the actual
  bytes, making local skill experiments reproducible without extra commits.
