import { dismissRequest, dismissResponse, groupLabels, inboxStatus, type DismissRequest, type InboxStatus } from "./schema";

const form = document.querySelector("form");
const input = document.querySelector("input");
const main = document.querySelector("main");
const groups = document.getElementById("groups");
const activeRuns = document.getElementById("active-runs");
const errorMessage = document.getElementById("error");
if (!form || !input || !main || !groups || !activeRuns || !errorMessage) throw new Error("Missing Inbox elements");
const view = { form, input, main, groups, activeRuns, errorMessage };
let token = sessionStorage.getItem("console-token") ?? "";

async function request(path: string, dismissal?: DismissRequest): Promise<unknown> {
  const response = await fetch(path, {
    method: dismissal ? "POST" : "GET",
    headers: { Authorization: `Bearer ${token}`, ...(dismissal ? { "Content-Type": "application/json" } : {}) },
    ...(dismissal ? { body: JSON.stringify(dismissRequest.parse(dismissal)) } : {}),
  });
  if (!response.ok) throw new Error(`Console request failed (${response.status})${response.status === 403 ? ": act scope required" : ""}`);
  const json: unknown = await response.json();
  return json;
}

function render(status: InboxStatus): void {
  view.errorMessage.textContent = "";
  view.form.hidden = true;
  view.main.hidden = false;
  view.activeRuns.textContent = `${status.activeRuns} active run${status.activeRuns === 1 ? "" : "s"}`;
  view.groups.replaceChildren();
  for (const [group, label] of Object.entries(groupLabels)) {
    const section = document.createElement("section");
    const heading = document.createElement("h2");
    heading.textContent = label;
    const list = document.createElement("ul");
    for (const row of status.rows.filter(row => row.group === group)) {
      const item = document.createElement("li");
      item.dataset.rowId = row.id;
      item.dataset.group = row.group;
      const title = document.createElement("span");
      title.textContent = row.title;
      item.append(title);
      if (row.dismissal || row.group === "needs-you" || row.group === "ready-for-review") {
        const action = document.createElement("button");
        action.textContent = row.dismissal ? "Undo" : "Dismiss";
        action.addEventListener("click", async () => {
          action.disabled = true;
          try { render(dismissResponse.parse(await request("/api/v1/dismiss", { id: row.id, undo: row.dismissal !== null }))); }
          catch (error) { showError(error); action.disabled = false; }
        });
        item.append(action);
      }
      list.append(item);
    }
    section.append(heading, list);
    view.groups.append(section);
  }
}

function showError(error: unknown): void {
  view.errorMessage.textContent = error instanceof Error ? error.message : String(error);
}

async function load(): Promise<void> {
  try {
    const status = inboxStatus.parse(await request("/api/v1/status"));
    sessionStorage.setItem("console-token", token);
    render(status);
  } catch (error) { showError(error); }
}
view.form.addEventListener("submit", event => {
  event.preventDefault();
  token = view.input.value;
  void load();
});
if (token) void load();
