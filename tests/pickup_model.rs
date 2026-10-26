//! Bounded model of ADR-0017. Request mapping and assumptions live in
//! .agents/skills/verify-pickup/SKILL.md and its protocol.md reference.

use stateright::{Checker, Model, Property};

const DISPATCH_SAFETY: &str = "one dispatch per Ready generation";
const CLAIM_SAFETY: &str = "one claimed dispatch per Ready generation";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Phase {
    List,
    SelectHistory,
    Reserve,
    RefreshIssue,
    RefreshHistory,
    Dispatch,
    ClaimRead,
    ClaimWrite,
    Running,
    NeedsInfoWrite,
    EndRead,
    EndWrite,
    Release,
    Done,
    Crashed,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
struct Labels {
    ready: bool,
    claimed: bool,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Host {
    phase: Phase,
    selected_generation: usize,
    labels: Labels,
    claim_entry: Labels,
    dispatched: bool,
    claim_completed: bool,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct State {
    hosts: Vec<Host>,
    reservation: Option<usize>,
    day_observed: Option<usize>,
    labels: Labels,
    claim_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Protocol {
    Current,
    WithoutClaimCount,
    UnconditionalCreate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Promise {
    Dispatch,
    ClaimedDispatch,
}

struct Pickup {
    hosts: usize,
    protocol: Protocol,
    promise: Promise,
}

/// Each request action maps to one external read/write. Local control and
/// process death are explicit, rather than bundled with an adjacent request.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Action {
    ListReady(usize),
    SelectHistory(usize),
    CreateReservation(usize),
    RefreshIssue(usize),
    RefreshHistory(usize),
    Dispatch(usize),
    ReadClaimLabels(usize),
    PutClaimLabels(usize),
    FailedRunReadsOrigin(usize),
    KeepClaim(usize),
    ReadNeedsInfoLabels(usize),
    PutNeedsInfoLabels(usize),
    ReadEndingLabels(usize),
    PutEndingLabels(usize),
    ReleaseReservation(usize),
    FailBeforeClaim(usize),
    Crash(usize),
    DayReadsReservation,
    DayDeletesStaleReservation,
    DayHandsBack,
}

impl Model for Pickup {
    type State = State;
    type Action = Action;

    fn init_states(&self) -> Vec<State> {
        vec![State {
            hosts: vec![
                Host {
                    phase: Phase::List,
                    selected_generation: 0,
                    labels: Labels::default(),
                    claim_entry: Labels::default(),
                    dispatched: false,
                    claim_completed: false,
                };
                self.hosts
            ],
            reservation: None,
            day_observed: None,
            labels: Labels {
                ready: true,
                claimed: false,
            },
            claim_count: 0,
        }]
    }

    fn actions(&self, state: &State, actions: &mut Vec<Action>) {
        for (h, host) in state.hosts.iter().enumerate() {
            let action = match host.phase {
                Phase::List => Action::ListReady(h),
                Phase::SelectHistory => Action::SelectHistory(h),
                Phase::Reserve => Action::CreateReservation(h),
                Phase::RefreshIssue => Action::RefreshIssue(h),
                Phase::RefreshHistory => Action::RefreshHistory(h),
                Phase::Dispatch => Action::Dispatch(h),
                Phase::ClaimRead => Action::ReadClaimLabels(h),
                Phase::ClaimWrite => Action::PutClaimLabels(h),
                Phase::Running if !host.claim_entry.claimed || host.claim_entry.ready => {
                    Action::FailedRunReadsOrigin(h)
                }
                Phase::Running => Action::KeepClaim(h),
                Phase::NeedsInfoWrite => Action::PutNeedsInfoLabels(h),
                Phase::EndRead => Action::ReadEndingLabels(h),
                Phase::EndWrite => Action::PutEndingLabels(h),
                Phase::Release => Action::ReleaseReservation(h),
                Phase::Done | Phase::Crashed => continue,
            };
            actions.push(action);
            actions.push(Action::Crash(h));
            // run::start may fail (base preparation, interruption or Claim
            // read/write failure) after pass::dispatch and before the Claim.
            if matches!(host.phase, Phase::ClaimRead | Phase::ClaimWrite) {
                actions.push(Action::FailBeforeClaim(h));
            }
            if host.phase == Phase::Running && (!host.claim_entry.claimed || host.claim_entry.ready)
            {
                actions.push(Action::KeepClaim(h));
            }
            if host.phase == Phase::Running {
                actions.push(Action::ReadNeedsInfoLabels(h));
            }
        }
        if state.reservation.is_some() && state.day_observed != state.reservation {
            actions.push(Action::DayReadsReservation);
        }
        if let Some(observed) = state.day_observed
            && state.hosts[observed].phase == Phase::Crashed
            && state.reservation == Some(observed)
        {
            actions.push(Action::DayDeletesStaleReservation);
        }
        // A hand-back follows confirmation that dispatched work has ended.
        if !state.labels.ready
            && state.hosts.iter().all(|host| {
                !host.dispatched
                    || matches!(host.phase, Phase::Release | Phase::Done | Phase::Crashed)
            })
        {
            actions.push(Action::DayHandsBack);
        }
    }

    fn next_state(&self, state: &State, action: Action) -> Option<State> {
        let mut next = state.clone();
        match action {
            Action::ListReady(h) => {
                next.hosts[h].phase = if state.labels.ready && !state.labels.claimed {
                    Phase::SelectHistory
                } else {
                    Phase::Done
                };
            }
            Action::SelectHistory(h) => {
                next.hosts[h].selected_generation = state.claim_count;
                next.hosts[h].phase = if state.labels.claimed {
                    Phase::Done
                } else {
                    Phase::Reserve
                };
            }
            Action::CreateReservation(h) => {
                if state.reservation.is_none() || self.protocol == Protocol::UnconditionalCreate {
                    next.reservation = Some(h);
                    next.hosts[h].phase = Phase::RefreshIssue;
                } else {
                    next.hosts[h].phase = Phase::Done;
                }
            }
            Action::RefreshIssue(h) => {
                next.hosts[h].phase = if state.labels.ready && !state.labels.claimed {
                    Phase::RefreshHistory
                } else {
                    Phase::Release
                };
            }
            Action::RefreshHistory(h) => {
                next.hosts[h].phase = if !state.labels.claimed
                    && (self.protocol == Protocol::WithoutClaimCount
                        || state.claim_count == state.hosts[h].selected_generation)
                {
                    Phase::Dispatch
                } else {
                    Phase::Release
                };
            }
            Action::Dispatch(h) => {
                next.hosts[h].dispatched = true;
                next.hosts[h].phase = Phase::ClaimRead;
            }
            Action::ReadClaimLabels(h) => {
                next.hosts[h].claim_entry = state.labels;
                if state.labels.claimed && !state.labels.ready {
                    // claim::make accepts an existing Claim without a PUT.
                    next.hosts[h].claim_completed = true;
                    next.hosts[h].phase = Phase::Running;
                } else {
                    next.hosts[h].phase = Phase::ClaimWrite;
                }
            }
            Action::PutClaimLabels(h) => {
                if !state.labels.claimed {
                    next.claim_count += 1;
                }
                next.labels = Labels {
                    ready: false,
                    claimed: true,
                };
                next.hosts[h].claim_completed = true;
                next.hosts[h].phase = Phase::Running;
            }
            Action::FailedRunReadsOrigin(h) => next.hosts[h].phase = Phase::EndRead,
            Action::KeepClaim(h) => next.hosts[h].phase = Phase::Release,
            Action::ReadNeedsInfoLabels(h) => next.hosts[h].phase = Phase::NeedsInfoWrite,
            Action::PutNeedsInfoLabels(h) => {
                // Diagnosis hands the bug back for information, ending the
                // Claim without making another Ready generation available.
                next.labels = Labels {
                    ready: false,
                    claimed: false,
                };
                next.hosts[h].phase = Phase::Release;
            }
            Action::ReadEndingLabels(h) => {
                next.hosts[h].labels = state.labels;
                next.hosts[h].phase = if state.labels.claimed {
                    Phase::EndWrite
                } else {
                    Phase::Release
                };
            }
            Action::PutEndingLabels(h) => {
                let host = &state.hosts[h];
                next.labels = Labels {
                    ready: host.labels.ready || host.claim_entry.ready,
                    claimed: host.claim_entry.claimed,
                };
                next.hosts[h].phase = Phase::Release;
            }
            Action::ReleaseReservation(h) => {
                if state.reservation == Some(h) {
                    next.reservation = None;
                }
                next.hosts[h].phase = Phase::Done;
            }
            Action::FailBeforeClaim(h) => next.hosts[h].phase = Phase::Release,
            Action::Crash(h) => next.hosts[h].phase = Phase::Crashed,
            Action::DayReadsReservation => next.day_observed = state.reservation,
            Action::DayDeletesStaleReservation => next.reservation = None,
            Action::DayHandsBack => {
                next.labels = Labels {
                    ready: true,
                    claimed: false,
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        let mut properties = vec![Property::<Self>::always(
            match self.promise {
                Promise::Dispatch => DISPATCH_SAFETY,
                Promise::ClaimedDispatch => CLAIM_SAFETY,
            },
            |model, state| unique_generation(state, model.promise),
        )];
        if self.protocol == Protocol::Current && self.promise == Promise::ClaimedDispatch {
            // Reachability checks guard against a vacuous safety pass from
            // accidentally removing recovery, requeue or competing hosts.
            properties.extend([
                Property::<Self>::sometimes("three hosts select", |_, s| {
                    s.hosts.iter().all(|h| h.phase == Phase::Reserve)
                }),
                Property::<Self>::sometimes("unreproduced bug ends without Ready", |_, s| {
                    !s.labels.ready
                        && !s.labels.claimed
                        && s.hosts
                            .iter()
                            .any(|h| h.phase == Phase::Release && h.claim_completed)
                }),
                Property::<Self>::sometimes("failed winner restores Ready", |_, s| {
                    s.labels.ready
                        && !s.labels.claimed
                        && s.hosts.iter().any(|h| {
                            h.phase == Phase::Release && h.claim_completed && h.labels.claimed
                        })
                }),
                Property::<Self>::sometimes("Day shift deletes a crashed reservation", |_, s| {
                    s.reservation.is_none()
                        && s.day_observed
                            .is_some_and(|h| s.hosts[h].phase == Phase::Crashed)
                }),
                Property::<Self>::sometimes("Day shift hands back retained Claim", |_, s| {
                    s.labels.ready
                        && !s.labels.claimed
                        && s.hosts.iter().any(|h| {
                            h.phase == Phase::Done && h.claim_completed && !h.labels.claimed
                        })
                }),
            ]);
        }
        properties
    }
}

fn unique_generation(state: &State, promise: Promise) -> bool {
    let claimed_only = promise == Promise::ClaimedDispatch;
    for (h, host) in state.hosts.iter().enumerate() {
        if !host.dispatched || (claimed_only && !host.claim_completed) {
            continue;
        }
        if state.hosts[..h].iter().any(|other| {
            other.dispatched
                && (!claimed_only || other.claim_completed)
                && other.selected_generation == host.selected_generation
        }) {
            return false;
        }
    }
    true
}

fn check(protocol: Protocol, promise: Promise) -> impl Checker<Pickup> {
    let checker = Pickup {
        hosts: 3,
        protocol,
        promise,
    }
    .checker()
    .spawn_bfs()
    .join();
    eprintln!(
        "{protocol:?}, {promise:?}: {} unique states",
        checker.unique_state_count()
    );
    checker
}

#[test]
fn pickup_claimed_dispatch_safety() {
    let checker = check(Protocol::Current, Promise::ClaimedDispatch);
    checker.assert_properties();
}

#[test]
fn current_protocol_reports_pre_claim_dispatch_counterexample() {
    let checker = check(Protocol::Current, Promise::Dispatch);
    let path = checker.assert_any_discovery(DISPATCH_SAFETY);
    eprintln!("KNOWN FINDING: the literal dispatch property fails: {path}");
    assert!(
        path.into_actions()
            .iter()
            .any(|action| matches!(action, Action::FailBeforeClaim(_) | Action::Crash(_)))
    );
}

#[test]
fn removing_claim_count_recheck_is_detected() {
    let checker = check(Protocol::WithoutClaimCount, Promise::ClaimedDispatch);
    let path = checker.assert_any_discovery(CLAIM_SAFETY);
    eprintln!("Claim-count mutation counterexample: {path}");
    assert!(path.into_vec().iter().any(|(s, action)| matches!(action,
        Some(Action::RefreshHistory(h)) if s.claim_count != s.hosts[*h].selected_generation)));
}

#[test]
fn removing_empty_create_lease_is_detected() {
    let checker = check(Protocol::UnconditionalCreate, Promise::ClaimedDispatch);
    let path = checker.assert_any_discovery(CLAIM_SAFETY);
    eprintln!("Empty-lease mutation counterexample: {path}");
    assert!(path.into_vec().iter().any(|(s, action)| matches!(action,
        Some(Action::CreateReservation(h)) if s.reservation.is_some_and(|owner| owner != *h))));
}

/// Literal issue-881 property: kept executable even while the current
/// protocol has a counterexample. The CI regression records that finding.
#[test]
#[ignore = "known pre-Claim dispatch finding; run explicitly to see the failure"]
fn assert_requested_property() {
    let (protocol, promise) = match std::env::var("PICKUP_MODEL_VARIANT").as_deref() {
        Ok("no-claim-count") => (Protocol::WithoutClaimCount, Promise::ClaimedDispatch),
        Ok("no-empty-lease") => (Protocol::UnconditionalCreate, Promise::ClaimedDispatch),
        Ok(value) => panic!("unknown PICKUP_MODEL_VARIANT: {value}"),
        Err(std::env::VarError::NotPresent) => (Protocol::Current, Promise::Dispatch),
        Err(error) => panic!("invalid PICKUP_MODEL_VARIANT: {error}"),
    };
    check(protocol, promise).assert_properties();
}
