# A Trial carries out a Process but is not a Command

An Arm can name a Process, Built-in or Custom, and its Trials then carry out that Process's Steps on an Eval task, from the Ticket up to the first publishing Step. Such a Trial runs the same sessions a Ticket run would, but it stays part of its Experiment, which is the Command, in the same way that a Spec run's child Ticket runs are part of the Spec run and not Commands of their own. It keeps a full record shaped like a Command record, with its Steps, session traces, outcome, usage and times, and the Console's run-detail and trace views can show it. That record is filed under the Experiment and counted only there. A Trial has no Claim, labels or pull request. Its record never enters the Console's run lists, Needs you, the Activity log, a Scoring run's sample or Run metrics, where it would mix test work into factory figures. Grilled in #1484, Q3.2 and Q8.1.

## Considered Options

- **Make each process Trial a Command flagged as part of an Experiment.** Rejected: every view that lists or counts Commands would need a filter for the flag, and one missed filter skews a factory figure without anyone noticing.
- **Keep Trials as single sessions with no record of their own.** Rejected: a process Trial runs many sessions across Steps and providers, and cost per role, Step times and the leak check all need them recorded.
