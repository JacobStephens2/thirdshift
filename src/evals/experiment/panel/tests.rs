use super::*;

fn seat(harness: &str, model: &str) -> Seat {
    Seat {
        harness: harness.into(),
        model: model.into(),
        effort: None,
        provider_family: None,
    }
}

fn side(ready: bool) -> SideCall {
    SideCall {
        rubric: Vec::new(),
        merge_reason: "Reason.".into(),
        ready_to_merge: ready,
    }
}

/// One seat's two orders, each preferring `first` then `second`.
fn orders(first: Preference, second: Preference, ready: [bool; 2]) -> Vec<Call> {
    [first, second]
        .into_iter()
        .map(|preference| Call {
            shown_as_a: Arm::Baseline,
            baseline: side(ready[0]),
            candidate: side(ready[1]),
            reason: "Reason.".into(),
            preference,
        })
        .collect()
}

#[test]
fn a_family_split_is_its_own_tie_before_the_panel_votes() {
    let panel = Panel {
        seats: vec![
            seat("claude", "a"),
            seat("claude", "b"),
            seat("codex", "c"),
            seat("codex", "d"),
            seat("grok", "e"),
        ],
        vote: Vote::Family,
        arm_family_rule: true,
        role: Role::Beside,
        families: 2,
        agreement: 0.8,
        spending_limit: None,
    };
    let candidate = Preference::Candidate;
    let (_, votes, preference, _) = tally(
        &panel,
        &BTreeSet::new(),
        vec![
            orders(candidate, candidate, [false, true]),
            orders(Preference::Baseline, Preference::Baseline, [true, false]),
            orders(candidate, candidate, [false, true]),
            orders(candidate, candidate, [false, true]),
            orders(candidate, candidate, [false, true]),
        ],
    );
    let named: Vec<_> = votes
        .iter()
        .map(|unit| (unit.name.as_str(), unit.preference))
        .collect();
    assert_eq!(
        named,
        [
            ("anthropic", Preference::Tie),
            ("openai", candidate),
            ("xai", candidate)
        ]
    );
    assert_eq!(preference, candidate);
}

#[test]
fn opencode_seats_take_the_family_of_the_provider_they_name() {
    assert_eq!(
        seat("opencode", "fireworks-ai/accounts/fireworks/models/kimi").family(),
        "fireworks-ai"
    );
    assert_eq!(seat("opencode", "anthropic/claude").family(), "anthropic");
    assert_eq!(seat("claude", "anything").family(), "anthropic");
}

#[test]
fn seats_are_named_fields() {
    let parsed = Seat::parse("harness=opencode,model=baseten/deepseek:v4,effort=high").unwrap();
    assert_eq!(parsed.model, "baseten/deepseek:v4");
    assert_eq!(parsed.effort.as_deref(), Some("high"));
    for invalid in [
        "claude",
        "harness=claude",
        "harness=nope,model=x",
        "harness=claude,model=x,model=y",
        "harness=claude,model=x,colour=red",
    ] {
        assert!(Seat::parse(invalid).is_err(), "{invalid}");
    }
}
