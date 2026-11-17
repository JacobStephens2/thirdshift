# Owning hosts release reservations on proof of death

Issue #1959 extends [ADR 0017](0017-pickup-reserves-issues-on-origin.md).
An orphaned CI reservation stopped Base branch repair across the fleet twice,
for eleven days and five days. Scheduled Pickup, Fix, Review, Triage, Architect,
Security and Release Passes inspect origin reservations before choosing work.
The owning host may release one only on local proof that its process stopped:
the PID does not exist, its process started after the reservation, or the host
booted after the reservation. Host means the same OS hostname recorded in the
reservation; this contract assumes distinct hostnames and a shared PID namespace
between that host's Commands. Linux and macOS supply the native clocks. Missing
or ambiguous owner fields and failed probes establish no death.

This is the one automatic recovery exception to ADR 0017's human recovery rule.
It is evidence of death, not expiry: age alone never authorizes deletion.
Deletion expects the exact observed SHA with `--force-with-lease`; a replaced
owner survives a lost lease. Releases and failed leases name the ref, process,
start time and blocked work on stderr and in the Activity log. Recovery changes
no Claim or pull-request label.

Another host's reservation, an unknown hostname and legacy or incomplete owner
records remain for the Day shift. Once one has blocked work and has been held
over 24 hours, Needs you shows its owner, age, blocked work and exact release
command. The 24 hours decides reporting only. Console and status refresh these
refs without deleting them; deletion or a different SHA clears the old row,
while unavailable origins retain the observation. Dismissal can be undone; it
never releases the reservation and applies only to that exact ref and SHA.
