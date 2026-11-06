import { z } from "zod";

const waitingGroup = z.enum(["needs-you", "ready-for-review"]);
export const inboxStatus = z.strictObject({
  activeRuns: z.number().int().nonnegative(),
  rows: z.array(z.strictObject({
    id: z.string().min(1),
    title: z.string(),
    group: z.enum(["needs-you", "ready-for-review", "landing", "working", "done", "resolved"]),
    active: z.boolean(),
    dismissal: z.strictObject({ from: waitingGroup }).nullable(),
  })),
});
export const dismissRequest = z.strictObject({ id: z.string().min(1), undo: z.boolean() });
// Dismissal and Undo return the fresh status, so every caller renders the same view.
export const dismissResponse = inboxStatus;
export type InboxStatus = z.infer<typeof inboxStatus>;
export type DismissRequest = z.infer<typeof dismissRequest>;
export type DismissResponse = z.infer<typeof dismissResponse>;

export const groupLabels = {
  "needs-you": "Needs you", "ready-for-review": "Ready for review", landing: "Landing",
  working: "Working", done: "Done", resolved: "Resolved",
} satisfies Record<InboxStatus["rows"][number]["group"], string>;
