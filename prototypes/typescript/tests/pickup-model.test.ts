import { expect, test } from "bun:test";

// Independent request-level model, not a second invocation of product code.
// Mapping, bounds and the deliberate Rust ordering difference: pickup-model.md.
type Protocol = "Current" | "WithoutClaimCount" | "UnconditionalCreate";
type Phase = "List" | "History" | "Reserve" | "RefreshIssue" | "RefreshHistory"
  | "ClaimWrite" | "Dispatch" | "Running" | "EndRead" | "EndWrite" | "Release" | "Done" | "Crashed";
type Labels = { ready: boolean; claimed: boolean };
type Host = { phase: Phase; generation: number; dispatched: boolean; claimedDispatch: boolean };
type State = { hosts: Host[]; labels: Labels; count: number; owner: number | null; observed: number | null };
type Step = { action: string; state: State };
function assertNever(value: never): never { throw new Error(`Unhandled model phase: ${value}`); }
function stopped(host: Host): boolean { return ["Release", "Done", "Crashed"].includes(host.phase); }

function successors(state: State, protocol: Protocol): Step[] {
  const result: Step[] = [];
  function step(action: string, change: (next: State) => void): void {
    const next = structuredClone(state);
    change(next);
    result.push({ action, state: next });
  }
  for (const [id, host] of state.hosts.entries()) {
    const action = (name: string, change: (next: State, host: Host) => void) => step(`H${id}.${name}`, next => {
      const target = next.hosts[id];
      if (!target) throw new Error("Missing host");
      change(next, target);
    });
    switch (host.phase) {
      case "List": action("ListReady", (next, h) => { h.phase = next.labels.ready && !next.labels.claimed ? "History" : "Done"; }); break;
      case "History": action("SelectHistory", (next, h) => {
        h.generation = next.count;
        h.phase = next.labels.claimed ? "Done" : "Reserve";
      }); break;
      case "Reserve": action("CreateReservation", (next, h) => {
        if (next.owner === null || protocol === "UnconditionalCreate") { next.owner = id; h.phase = "RefreshIssue"; }
        else h.phase = "Done";
      }); break;
      case "RefreshIssue": action("RefreshIssue", (next, h) => {
        h.phase = next.labels.ready && !next.labels.claimed ? "RefreshHistory" : "Release";
      }); break;
      case "RefreshHistory": action("RefreshHistory", (next, h) => {
        h.phase = !next.labels.claimed && (protocol === "WithoutClaimCount" || next.count === h.generation) ? "ClaimWrite" : "Release";
      }); break;
      case "ClaimWrite":
        action("PutClaimLabels", (next, h) => {
          if (!next.labels.claimed) next.count++;
          next.labels = { ready: false, claimed: true };
          h.phase = "Dispatch";
        });
        action("FailBeforeClaim", (_next, h) => { h.phase = "Release"; });
        break;
      case "Dispatch":
        action("Dispatch", (_next, h) => { h.dispatched = true; h.claimedDispatch = true; h.phase = "Running"; });
        // Worktree/comment/Harness startup may fail after the Claim write.
        action("FailAfterClaim", (_next, h) => { h.phase = "EndRead"; });
        break;
      case "Running":
        action("FailedRunReadsOrigin", (_next, h) => { h.phase = "EndRead"; });
        action("KeepClaim", (_next, h) => { h.phase = "Release"; });
        break;
      case "EndRead": action("ReadEndingLabels", (next, h) => {
        h.phase = next.labels.claimed ? "EndWrite" : "Release";
      }); break;
      case "EndWrite": action("PutEndingLabels", (next, h) => { next.labels = { ready: true, claimed: false }; h.phase = "Release"; }); break;
      case "Release": action("ReleaseReservation", (next, h) => {
        if (next.owner === id) next.owner = null;
        h.phase = "Done";
      }); break;
      case "Done": case "Crashed": continue;
      default: assertNever(host.phase);
    }
    action("Crash", (_next, h) => { h.phase = "Crashed"; });
  }
  if (state.owner !== null && state.observed !== state.owner) step("DayReadsReservation", next => { next.observed = next.owner; });
  if (state.observed !== null && state.owner === state.observed && state.hosts[state.observed]?.phase === "Crashed") {
    step("DayDeletesStaleReservation", next => { next.owner = null; });
  }
  // Humans only recover dead owners and hand back work confirmed stopped.
  if (!state.labels.ready && state.hosts.every(host => stopped(host) || !["Dispatch", "Running", "EndRead", "EndWrite"].includes(host.phase))) {
    step("DayHandsBack", next => { next.labels = { ready: true, claimed: false }; });
  }
  return result;
}

function safe(state: State, claimedOnly: boolean): boolean {
  const generations = new Set<number>();
  for (const host of state.hosts) {
    if (!host.dispatched || (claimedOnly && !host.claimedDispatch)) continue;
    if (generations.has(host.generation)) return false;
    generations.add(host.generation);
  }
  return true;
}

// Hosts have identical code and unique tokens. Quotienting host permutations
// preserves every action up to renaming while avoiding six copies per state.
function key(state: State): string {
  const hosts = state.hosts.map((host, id) => JSON.stringify([host, state.owner === id, state.observed === id])).sort();
  return JSON.stringify([state.labels, state.count, hosts]);
}
type Node = { state: State; parent: number; action: string };
function check(protocol: Protocol) {
  const initial: State = {
    hosts: Array.from({ length: 3 }, () => ({ phase: "List", generation: 0, dispatched: false, claimedDispatch: false })),
    labels: { ready: true, claimed: false }, count: 0, owner: null, observed: null,
  };
  const queue: Node[] = [{ state: initial, parent: -1, action: "Initial" }];
  const seen = new Set([key(initial)]);
  const reached = new Set<string>();
  let dispatchTrace: string[] | undefined;
  let claimedTrace: string[] | undefined;
  function trace(index: number): string[] {
    const path: string[] = [];
    while (index >= 0) {
      const node = queue[index];
      if (!node) throw new Error("Missing BFS predecessor");
      path.push(`${node.action} (generation=${node.state.count}, owner=${node.state.owner}, Ready=${node.state.labels.ready}, Claim=${node.state.labels.claimed})`);
      index = node.parent;
    }
    return path.reverse();
  }
  for (let cursor = 0; cursor < queue.length; cursor++) {
    const node = queue[cursor];
    if (!node) throw new Error("Missing BFS node");
    if (!safe(node.state, false) && !dispatchTrace) dispatchTrace = trace(cursor);
    if (!safe(node.state, true) && !claimedTrace) claimedTrace = trace(cursor);
    if (dispatchTrace && claimedTrace) break;
    if (node.state.hosts.every(host => host.phase === "Reserve")) reached.add("three hosts select");
    for (const next of successors(node.state, protocol)) {
      reached.add(next.action.replace(/^H\d\./, ""));
      const identity = key(next.state);
      if (seen.has(identity)) continue;
      seen.add(identity);
      queue.push({ ...next, parent: cursor });
    }
  }
  return { states: seen.size, reached, dispatchTrace, claimedTrace };
}

test("ADR-0017 BFS exhausts three-host interleavings with both safety properties", () => {
  const result = check("Current");
  console.log(`Current: ${result.states} unique states; exhausted BFS`);
  expect(result.dispatchTrace).toBeUndefined();
  expect(result.claimedTrace).toBeUndefined();
  for (const action of ["three hosts select", "Crash", "DayDeletesStaleReservation", "DayHandsBack", "PutEndingLabels", "FailBeforeClaim", "FailAfterClaim", "KeepClaim"]) {
    expect(result.reached.has(action)).toBe(true);
  }
}, 120_000);

test.each(["WithoutClaimCount", "UnconditionalCreate"] satisfies Protocol[])("BFS finds minimal counterexamples for %s", protocol => {
  const result = check(protocol);
  expect(result.dispatchTrace).toBeDefined();
  expect(result.claimedTrace).toBeDefined();
  console.log(`${protocol}: shortest dispatch counterexample\n${result.dispatchTrace?.join("\n")}`);
  console.log(`${protocol}: shortest claimed-dispatch counterexample\n${result.claimedTrace?.join("\n")}`);
  if (protocol === "WithoutClaimCount") expect(result.claimedTrace?.join("\n")).toMatch(/PutEndingLabels|DayHandsBack/);
  if (protocol === "UnconditionalCreate") expect(result.claimedTrace?.filter(step => step.includes("CreateReservation")).length).toBe(2);
}, 120_000);
