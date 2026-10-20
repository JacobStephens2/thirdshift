# The Console has one layout, and its looks are Themes made of tokens

The #711 prototypes tried eighteen looks for the **Console**. Two layouts drew the most interest, and they were the same inbox under different looks: one plain, one in thirdshift.app's press style. So the Console has one layout, the **Inbox** and its pages, and every look is a **Theme**. A Theme is a set of tokens: colours, fonts, shape, effects and motion. It may not change what the page shows, its order, or where things are. The default is Focus, which follows the system's light or dark setting. Plain, Press, Signal, Garden and Factory ship beside it, and a user can make, export and import their own.

## Considered Options

- **Several layouts, one per look** (a garden bed, a press line, a grid). Rejected. Every layout would need its own tests, phone checks and keyboard model. A layout tied to the garden metaphor would also have to be redrawn whenever the metaphor moves.
- **One look and no Themes.** Rejected. The plain and press looks were each wanted for different moods, and a user with their own style would have no way to have it.

## Consequences

- Every built-in Theme is checked for AA text contrast, and Focus is also checked for APCA. A user's Theme is theirs to check.
- Motion belongs to the layout, not the Theme. It is reserved for thirdshift's activity: a Theme can change how a live Command moves, but it can't make anything else move.
- Once people have exported Themes, the token set becomes a format to keep: new tokens need defaults, so older Themes keep importing.
