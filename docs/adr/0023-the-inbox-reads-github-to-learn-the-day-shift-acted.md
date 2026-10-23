# The Inbox reads GitHub to learn the Day shift has acted

Until now the **Inbox** read only what each **Command** writes about itself, as ADR 0016 says of the **Console**, and #711 (Q37) turned down reading GitHub on every refresh. So it couldn't tell when the **Day shift** had acted on GitHub. A pull request merged by hand stayed in Ready for review, and a failure whose issue was closed stayed in **Needs you** until it aged out. On 2026-10-23, GitHub already showed 36 of the 58 rows in the two machines' Needs you and Ready for review finished (#919).

Now each machine's Console reads GitHub once a minute. One batched GraphQL query looks up every row's issue and pull request, including those of the Run an **Architect run** or **Security run** dispatched, and one REST list per repository reads the **Security findings** of each Security run in the Inbox whose findings were waiting. The Console keeps the answer in a file on the machine. `thirdshift status` reads the same file, and asks GitHub itself only when no Console has refreshed it lately, so the two still agree on every row's group. GitHub's latest state then decides whether a row still waits:
- An ended row leaves Needs you once GitHub shows its issue closed or its pull request merged. A Triage row also leaves once `needs-triage` is off its issue, and a Finding once none of its findings still waits.
- A ready Command leaves Ready for review once its pull request is merged or closed.
- Age moves only a row GitHub can't speak for: one with no issue or pull request, one with no **Command record**, or one not yet read. Every other row waits until GitHub shows it finished, a later Command starts on its issue, or someone makes a **Dismissal**.

## Considered Options

- **Each Pickup run's Sweep reads GitHub and writes what it finds beside each record.** Rejected: a **Pickup run** is skipped while a Run holds its repository, which is when a pull request merged by hand is likeliest, and a repository with no Pickup line would never be read.
- **Whatever shows the Inbox asks GitHub once its answer is a minute old.** Rejected: `thirdshift status` would wait on GitHub whenever it ran more than a minute after the last read.
- **GitHub webhooks.** Rejected: each of 19 repositories would need a webhook with a shared secret, the Console would need a route open to anyone, and deliveries missed during a restart would still need a poll.
- **Nothing from GitHub, only Dismissal and age.** Rejected: the Day shift would have to say again in the Console what it had just done on GitHub, which is the chore #919 is about.

## Consequences

- The Console runs `gh`, and its reads come out of the same quotas as the factory's (#739): about 60 GraphQL points an hour per machine, plus the REST lists.
- Needs you and Ready for review keep a row until it is dealt with, not until it is old (#919 Q2.5). #1083's age limit applies only to rows GitHub can't speak for.
- The reads never write to GitHub. Taking `in-progress` off a closed issue stays the Sweep's job.
- GitHub moves only ended rows. A **Silent** or **Slow** row stays whatever GitHub shows, and GitHub's facts apply once it ends (#919 Q3.5).
- When GitHub can't be read, the Inbox keeps the last answer. A row keeps the group its last-read facts give it, and one GitHub hasn't been read for, or can't find, goes by age. After 5 minutes without a good read, the Inbox's warning says when GitHub last answered and why the latest read failed (#919 Q3.1).
- The Console leaves the last tenth of each quota to the factory. With fewer than 500 GraphQL points or 500 REST requests left, it skips its reads until that quota resets, and says so (#919 Q3.2).
- GitHub is read for a row only until it reaches the History page, so each read grows with the Inbox, not with the machine's history, and a reopened issue brings back only a row still in Done or Resolved (#919 Q3.3).
- Resolved counts from when a row was dealt with, so one dealt with after a long wait stays in view. A row that ages out still goes straight to History (#919 Q3.4, Q4.2).
- Nothing turns the reads off, and none of their timings is a setting (#919 Q3.7).
- The Inbox can be up to a minute behind GitHub, and further while GitHub can't be read or the quota is low, when it says so.
