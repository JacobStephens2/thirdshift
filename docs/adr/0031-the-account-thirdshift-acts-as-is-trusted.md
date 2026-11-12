# The account thirdshift acts as is a Trusted account

This amends the **Trusted account** rule, which until now was the repository's owner, members of the organization that owns it and its collaborators.

Every host in the fleet authenticated `gh` as one person, so all of them shared that person's 5,000-an-hour GitHub quota (#739, #1993). A GitHub App installation token has its own, larger limit. A host that authenticates as the App acts on GitHub as `<app>[bot]`: the issues, pull requests, comments and `ready-for-agent` labellings it makes come from that bot. GitHub gives the bot no `OWNER`, `MEMBER` or `COLLABORATOR` association, and its collaborator check answers 404. Under the old rule, a Review or Fix Pass would refuse the fleet's own pull requests, Pickup would pass over Specs that thirdshift itself handed back, and sessions would lose the fleet's own comments.

So the login thirdshift's own credentials act as is also a Trusted account. REST spells a bot's login `<app>[bot]` and GraphQL spells it `<app>`, so the two are compared without that suffix. It is the login the **Quota wait** already resolves and caches per credential: `gh api user`, else GraphQL `viewer`, else `gh auth status`. The rule stays fixed, with no **User config** setting. An issue author whose association is not trusted is now checked as an actor is, by repository membership and then that login. Comments and Base CI reports are checked by association and that login only, with no membership read for each one.

## Considered Options

- **A User config list of trusted bot logins.** Rejected: it makes trust configurable on each host, which the rule has refused so far, and a host whose list is wrong would trust or refuse work silently.
- **Trust any GitHub App installed on the repository.** Rejected: any installed App, such as a third-party bot that writes text, would become trusted, and checking installations needs the App's own JWT.

## Consequences

- A host still signed in as a person does not trust the App's bot, and a host signed in as the App does not trust the person unless that person is the owner, a member or a collaborator, which the fleet's owner is. Moving the fleet to the App is therefore all or nothing for pull requests that hosts review for each other.
- The Quota wait stays keyed by login, not by App installation: one App's hosts share one quota, as one person's did. Each installation token is a new credential, so the identity cache replaces a login's earlier credential instead of keeping one entry per token.
- If the login cannot be resolved, a check that reaches it fails instead of leaving the author out, as an unreadable membership already does.
- An untrusted association now costs one more GitHub read, and with an unknown credential, one identity read, before the author is refused.
