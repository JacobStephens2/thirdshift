//! Escaped Experiment markup. The same facts feed the list, grid and Peeks.
use crate::console::filters::escaped as esc;
use crate::evals::experiment::{
    Arm, Conclusion, Flip, Pending, State, TrialResult, View, setup_changes, trial_key,
};
use crate::trial::Outcome;
use crate::verdict::{Decision, PassRule};

/// An instant as people read it, escaped: this machine's zone, to the minute.
fn when(text: &str) -> String {
    esc(&crate::local_time::readable(text))
}
fn short(text: &str) -> String {
    text.chars().take(10).collect()
}
fn active(view: &View) -> bool {
    matches!(view.state, State::Running | State::Prepared)
}

struct Status {
    label: &'static str,
    tone: &'static str,
    group: usize,
}

fn status(view: &View) -> Status {
    if active(view) && !view.details.waiting_for.is_empty() {
        return Status {
            label: "Waiting for Commands",
            tone: "working",
            group: 0,
        };
    }
    match view.verdict.as_ref().map(|v| &v.verdict) {
        Some(Conclusion::WaitingOnLeaks) => Status {
            label: "Waiting on leak decisions",
            tone: "look",
            group: 1,
        },
        Some(Conclusion::Pass) => Status {
            label: "Pass",
            tone: "done",
            group: 2,
        },
        Some(Conclusion::Fail) => Status {
            label: "Fail",
            tone: "urgent",
            group: 1,
        },
        Some(Conclusion::Inconclusive) => Status {
            label: "Inconclusive",
            tone: "look",
            group: 1,
        },
        Some(Conclusion::WaitingOnRechecks) => Status {
            label: "Waiting on rechecks",
            tone: "look",
            group: 1,
        },
        Some(Conclusion::WaitingOnPeople) => Status {
            label: "Waiting on people",
            tone: "look",
            group: 1,
        },
        None if active(view) => Status {
            label: "Running",
            tone: "working",
            group: 0,
        },
        None => Status {
            label: "Paused",
            tone: "look",
            group: 1,
        },
    }
}
fn tone(view: &View) -> &str {
    status(view).tone
}
fn tag(view: &View) -> String {
    let status = status(view);
    format!(
        "<span class=\"experiment-tag\" data-tone=\"{}\">{}</span>",
        status.tone, status.label
    )
}
fn group(view: &View) -> usize {
    status(view).group
}
fn total(view: &View) -> usize {
    view.tasks
        .iter()
        .map(|task| 2 * view.description.bar.trials_per_task[&task.suite])
        .sum()
}
fn now(view: &View) -> String {
    if active(view) && !view.details.waiting_for.is_empty() {
        return format!(
            "Waiting for Commands: {}",
            view.details.waiting_for.join(", ")
        );
    }
    let Some(verdict) = &view.verdict else {
        return format!(
            "Trials {} of {}, {} running{}",
            view.results.len(),
            total(view),
            view.running_trials.len(),
            if active(view) { "" } else { "; paused" }
        );
    };
    let mut reasons = Vec::new();
    let pending_leaks = view
        .leak_hits
        .iter()
        .filter(|hit| hit.decision.is_none())
        .count();
    if pending_leaks > 0 {
        reasons.push(format!("{pending_leaks} leak decisions waiting"));
    }
    if !view.pending_rechecks.is_empty() {
        reasons.push(format!(
            "{} rechecks waiting, provisionally {}",
            view.pending_rechecks.len(),
            decision(
                verdict
                    .provisional_verdict
                    .decision()
                    .unwrap_or(Decision::Inconclusive)
            )
        ));
    }
    for (name, suite) in &verdict.suites {
        let suite = &suite.statistics;
        if suite.dropped_tasks > (suite.retained_tasks + suite.dropped_tasks) / 10 {
            reasons.push(format!(
                "{name}: {} Eval tasks dropped",
                suite.dropped_tasks
            ));
        } else {
            for (metric, assessment) in [
                ("accuracy", &suite.accuracy),
                ("wrong findings", &suite.wrong_findings),
            ] {
                if assessment.verdict != Decision::Pass {
                    reasons.push(format!("{name} {metric}: {}", decision(assessment.verdict)));
                }
            }
        }
    }
    if reasons.is_empty() {
        "Every suite passed under the Bar.".into()
    } else {
        reasons.join(" · ")
    }
}
fn decision(value: Decision) -> &'static str {
    match value {
        Decision::Pass => "pass",
        Decision::Fail => "fail",
        Decision::Inconclusive => "inconclusive",
    }
}

fn command_links(view: &View) -> String {
    view.details.command_log.as_ref().map_or_else(
        || "<span class=\"experiment-dim\">Command log unavailable</span>".into(),
        |path| {
            let command = path.to_string_lossy();
            format!(
                "<a href=\"{}\" class=\"peek-link\">Details</a> · <a href=\"{}\">Full log</a>",
                esc(&crate::logs::status::inspect::url("/peek", &command, None)),
                esc(&crate::logs::status::inspect::url("/log", &command, None))
            )
        },
    )
}

fn builds(view: &View) -> String {
    format!(
        "Baseline {} <code>{}</code> → Candidate {} <code>{}</code>",
        esc(view
            .details
            .baseline_ref
            .as_deref()
            .unwrap_or("ref unknown")),
        esc(&short(&view.description.baseline.commit)),
        esc(view
            .details
            .candidate_ref
            .as_deref()
            .unwrap_or("ref unknown")),
        esc(&short(&view.description.candidate.commit))
    )
}

pub(super) fn list(views: &[View]) -> String {
    let mut html = String::from(
        "<div class=\"page-title\"><h1>Experiments</h1><code>thirdshift experiments --json</code></div><p>Compare a Baseline arm with a Candidate arm on the Eval suites a change touches. Start an Experiment from the CLI.</p>",
    );
    for (index, title) in ["Running", "Needs you", "Done"].iter().enumerate() {
        let rows: Vec<_> = views.iter().filter(|view| group(view) == index).collect();
        html.push_str(&format!("<details class=\"group experiment-group\" id=\"experiment-group-{index}\" open><summary><span>{title}</span><span class=\"count\">{}</span></summary><ul class=\"commands\">", rows.len()));
        if rows.is_empty() {
            html.push_str("<li class=\"empty\">Nothing here.</li>");
        }
        for view in rows {
            let id = esc(&view.description.id);
            let command = view
                .details
                .command_log
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            html.push_str(&format!("<li class=\"command experiment-row\" tabindex=\"0\" id=\"experiment-{id}\" data-tone=\"{}\" data-command=\"{}\" data-peek=\"{}\"><div class=\"row-head\">{} <span class=\"subject\"><a href=\"/experiments/{id}\">{}</a></span><span class=\"issue-title\">{}</span><time class=\"age\">{}</time></div><p class=\"now\">{}</p>{}{}<p class=\"harness\">{}</p><p><a href=\"/experiments/{id}\">Experiment</a> · {}</p></li>", tone(view), esc(&command), esc(&crate::logs::status::inspect::url("/peek", &command, None)), tag(view), esc(&view.name), esc(view.details.title.as_deref().unwrap_or("")), when(view.details.ended_at.as_deref().unwrap_or(&view.created_at)), esc(&now(view)), contention(view), list_intervals(view), builds(view), command_links(view)));
        }
        html.push_str("</ul></details>");
    }
    html
}

fn contention(view: &View) -> String {
    view.details.contention().map_or_else(String::new, |line| {
        format!("<p class=\"experiment-contention\">{}</p>", esc(&line))
    })
}

/// What the Experiment asks, in words: the change between its arms, the suites
/// it is judged on and what counts as worse, or that it tests no change at all.
fn question(view: &View) -> String {
    let d = &view.description;
    let suites = match d.suites.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    let reference = |name: &Option<String>, commit: &str| match name {
        Some(name) if !commit.starts_with(name.as_str()) => format!("{name} ({})", short(commit)),
        _ => short(commit),
    };
    let mut changes = Vec::new();
    if d.baseline.commit != d.candidate.commit {
        let mut build = format!(
            "Build: {} → {}",
            reference(&view.details.baseline_ref, &d.baseline.commit),
            reference(&view.details.candidate_ref, &d.candidate.commit)
        );
        if let Some(title) = view.details.title.as_deref().filter(|t| !t.is_empty()) {
            build.push_str(&format!(", “{title}”"));
        }
        changes.push(build);
    }
    changes.extend(setup_changes(d));
    let body = if changes.is_empty() {
        format!(
            "<p>Both arms are the same build, Harness, Model and Effort, so this Experiment tests no change: it shows how much {} results vary between runs of one setup.</p>",
            esc(&suites)
        )
    } else {
        format!(
            "<p>Does the Candidate do worse than the Baseline on {}? The Candidate changes:</p><ul>{}</ul><p>Worse means a pass share more than {} points below the Baseline’s.</p>",
            esc(&suites),
            changes
                .iter()
                .map(|change| format!("<li>{}</li>", esc(change)))
                .collect::<String>(),
            (-d.bar.margin * 100.0).round()
        )
    };
    format!(
        "<section class=\"experiment-question\" aria-label=\"What this Experiment tests\"><h2>What it tests</h2>{body}</section>"
    )
}

pub(super) fn detail(view: &View, csrf: &str, can_act: bool) -> String {
    let id = esc(&view.description.id);
    let command_log = view.details.command_log.as_ref().map_or_else(
        || "Command log unavailable".into(),
        |path| {
            format!(
                "<a href=\"{}\">Command log</a>",
                esc(&crate::logs::status::inspect::url(
                    "/log",
                    &path.to_string_lossy(),
                    None
                ))
            )
        },
    );
    let mut html = format!(
        "<p><a href=\"/experiments\">← Experiments</a></p><div class=\"page-title\"><h1>{}</h1><code>thirdshift experiments {id} --json</code></div>{}<p>Started <time>{}</time>{} · {}</p><section class=\"experiment-lead\" aria-label=\"Experiment status\">{} <p>{}</p></section>",
        esc(&view.name),
        question(view),
        when(&view.created_at),
        view.details
            .ended_at
            .as_ref()
            .map(|time| format!(" · Ended <time>{}</time>", when(time)))
            .unwrap_or_default(),
        command_log,
        tag(view),
        esc(&lead(view))
    );
    if view.verdict.is_none() {
        if let Some(reason) = &view.details.pause_reason {
            html.push_str(&format!("<p>{}</p>", esc(reason)));
        }
        if !active(view) {
            if let Some(command) = &view.details.resume_command {
                html.push_str(&format!(
                    "<p>Run the same command to resume: <code>{}</code></p>",
                    esc(command)
                ));
            }
        } else {
            html.push_str("<p>Trials fill in as they finish. Model Graders then score review tasks' blind packets; process tasks are scored on their outcome.</p>");
        }
    }
    html.push_str(&leaks(view, csrf, can_act));
    html.push_str(&contention(view));
    html.push_str(&verdict(view));
    let timing_note = view.accounting.timing_note();
    if !timing_note.is_empty() {
        html.push_str(&format!("<p>{}</p>", esc(&timing_note)));
    }
    html.push_str(&people(view));
    if view.description.baseline.process.is_some() || view.description.candidate.process.is_some() {
        html.push_str("<section aria-label=\"Process Trial outcomes\"><h2>Process Trial outcomes</h2><p>Stopped-to-ask counts as failed in the Verdict.</p><dl>");
        for (label, counts) in [
            ("Baseline", &view.outcomes.baseline),
            ("Candidate", &view.outcomes.candidate),
        ] {
            html.push_str(&format!(
                "<dt>{label}</dt><dd>{} delivered · {} stopped-to-ask · {} failed</dd>",
                counts.delivered, counts.stopped_to_ask, counts.failed
            ));
        }
        html.push_str("</dl></section>");
    }
    html.push_str(&pairs(view));
    for finding in &view.findings {
        html.push_str(&format!(
            "<p class=\"experiment-callout\">Finding: {}</p>",
            esc(finding)
        ));
    }
    html.push_str(&rechecks(view));
    if view.state == State::Completed {
        html.push_str(&arms(view));
        html.push_str(&grid(view));
    } else {
        html.push_str(&grid(view));
        html.push_str(&arms(view));
    }
    html.push_str(&bar(view));
    html
}

/// People's blind pairwise calls, with each reader's queue or links to read
/// every pair blind. The Bar says whether they count in the Verdict.
fn people(view: &View) -> String {
    use crate::evals::experiment::{deciders, pairwise};
    if view.pairs.is_empty() {
        return String::new();
    }
    let id = esc(&view.description.id);
    let people = &view.people;
    let mut html = String::from("<section aria-label=\"People's calls\"><h2>People's calls</h2>");
    html.push_str(match deciders::people_role(&view.description.bar) {
        deciders::Role::Off => {
            html.push_str("<p>The Bar has people off: no one grades these pairs.</p></section>");
            return html;
        }
        deciders::Role::Beside => {
            "<p>Blind pairwise calls, reported beside the Verdict. They change no Score.</p>"
        }
        deciders::Role::Counted => {
            "<p>Blind pairwise calls, counted in the Verdict: a Trial passes only if the code Grader passes and its pair is called ready. On a shared pair, every reader must call it ready.</p>"
        }
    });
    let link = |key: &str| {
        view.pairs
            .iter()
            .find(|pair| pair.key() == key)
            .map_or_else(
                || format!("{} (not finished)", esc(key)),
                |pair| {
                    format!(
                        "<a href=\"/experiments/{id}/pairs/{}\">{}/{} #{}</a>",
                        esc(key),
                        esc(&pair.suite),
                        esc(&pair.task),
                        pair.trial + 1
                    )
                },
            )
    };
    if people.queues.is_empty() {
        html.push_str("<p>Read a pair blind:</p><ul>");
        for pair in &view.pairs {
            html.push_str(&format!("<li>{}</li>", link(&pair.key())));
        }
        html.push_str("</ul>");
    } else {
        html.push_str("<h3>Queues</h3><p>The pairs the Bar's seed gives each reader.</p>");
        for queue in &people.queues {
            html.push_str(&format!("<h4>{}</h4><ul>", esc(&queue.reader)));
            for key in &queue.pairs {
                let called = people.reviews.iter().any(|review| {
                    review.pair == *key && review.blind && review.reader.who() == queue.reader
                });
                html.push_str(&format!(
                    "<li>{} · {}</li>",
                    link(key),
                    if called { "called" } else { "to read" }
                ));
            }
            html.push_str("</ul>");
        }
    }
    if people.reviews.is_empty() {
        html.push_str("<p>No calls yet.</p></section>");
        return html;
    }
    html.push_str("<table><thead><tr><th scope=\"col\">Pair</th><th scope=\"col\">Reader</th><th scope=\"col\">Recorded</th><th scope=\"col\">Baseline</th><th scope=\"col\">Candidate</th><th scope=\"col\">Preference</th><th scope=\"col\">Blind</th></tr></thead><tbody>");
    for review in &people.reviews {
        html.push_str(&format!(
            "<tr><td>{}/{} #{}</td><td>{}</td><td><time>{}</time></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            esc(&review.suite),
            esc(&review.task),
            review.trial + 1,
            esc(review.reader.who()),
            esc(&review.recorded_at),
            pairwise::ready(review.baseline_ready),
            pairwise::ready(review.candidate_ready),
            review
                .preference
                .map_or("—", pairwise::arm_preference),
            if review.blind {
                "yes"
            } else {
                "no, after the reveal"
            }
        ));
    }
    html.push_str(&format!(
        "</tbody></table><p>{}</p><p>{}</p></section>",
        esc(&pairwise::agreement(&people.agreement)),
        esc(&pairwise::people_agreement(&people.people_agreement))
    ));
    html
}

/// Base-tests and cross-tests per pair, numbered from 1: evidence, not Scores.
fn pairs(view: &View) -> String {
    if view.pairs.is_empty() {
        return String::new();
    }
    let mut html = String::from(
        "<section aria-label=\"Pair evidence\"><h2>Pair evidence</h2><p>Each Arm's new tests, run offline on the unchanged base and on the other Arm's final commit. A cross-test failure is a disagreement to look into, not a defect, and changes no Score.</p><table><thead><tr><th scope=\"col\">Pair</th><th scope=\"col\">Arm</th><th scope=\"col\">New tests</th><th scope=\"col\">On the base</th><th scope=\"col\">On the other Arm</th></tr></thead><tbody>",
    );
    for pair in &view.pairs {
        for (arm, side) in [("Baseline", &pair.baseline), ("Candidate", &pair.candidate)] {
            let tests = if side.test_files.is_empty() {
                "None".to_owned()
            } else {
                side.test_files
                    .iter()
                    .map(|file| format!("<code>{}</code>", esc(file)))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            let unknown = |checks: &[crate::sandbox::code_grader::Check]| match &side.error {
                Some(error) => format!("Unavailable: {}", esc(error)),
                None if checks.is_empty() => "—".to_owned(),
                None => "Did not finish".to_owned(),
            };
            let base = match side.fails_on_base {
                Some(true) => "Fails".to_owned(),
                Some(false) => "Passes".to_owned(),
                None => unknown(&side.base),
            };
            let other = match side.passes_on_other {
                Some(true) => "Passes".to_owned(),
                Some(false) => "Fails: disagreement".to_owned(),
                None => unknown(&side.cross),
            };
            html.push_str(&format!(
                "<tr><td>{}/{} #{}</td><td>{arm}</td><td>{tests}</td><td>{base}</td><td>{other}</td></tr>",
                esc(&pair.suite),
                esc(&pair.task),
                pair.trial + 1
            ));
        }
    }
    html.push_str("</tbody></table></section>");
    html
}

fn lead(view: &View) -> String {
    match view.verdict.as_ref().map(|v| &v.verdict) {
        Some(Conclusion::WaitingOnLeaks) => {
            "Review the pending Session evidence leak hits before the Verdict is final.".into()
        }
        Some(Conclusion::WaitingOnRechecks) => format!(
            "Provisionally {}, until you recheck {} Scores.",
            decision(
                view.verdict
                    .as_ref()
                    .unwrap()
                    .provisional_verdict
                    .decision()
                    .unwrap_or(Decision::Inconclusive)
            ),
            view.pending_rechecks.len()
        ),
        Some(Conclusion::Pass) => {
            "Pass: the Candidate is no worse than the Baseline under the Bar.".into()
        }
        Some(Conclusion::Fail) => {
            "Fail: the Candidate is worse than the Baseline under the Bar.".into()
        }
        Some(Conclusion::WaitingOnPeople) => format!(
            "Provisionally {}, until each reader makes their blind merge calls on {} pairs.",
            decision(
                view.verdict
                    .as_ref()
                    .unwrap()
                    .provisional_verdict
                    .decision()
                    .unwrap_or(Decision::Inconclusive)
            ),
            view.verdict
                .as_ref()
                .and_then(|verdict| verdict.deciders.as_ref())
                .map_or(0, |deciders| deciders.waiting_pairs)
        ),
        Some(Conclusion::Inconclusive) => match view
            .verdict
            .as_ref()
            .and_then(|verdict| verdict.deciders.as_ref())
            .and_then(|deciders| deciders.inconclusive.as_ref())
        {
            Some(reason) => format!("Inconclusive: {reason}."),
            None => {
                "Inconclusive: these Trials cannot settle whether the Candidate is worse.".into()
            }
        },
        None => now(view),
    }
}

fn leaks(view: &View, csrf: &str, can_act: bool) -> String {
    if view.leak_hits.is_empty() {
        return String::new();
    }
    let mut html = String::from(
        "<section class=\"experiment-callout\" id=\"experiment-leaks\" aria-label=\"Session evidence leak check\"><h2>Session evidence leak check</h2><p>Review each match to the real fix. Dismiss false positives to keep the Trial; confirm a leak to make it Void. Decisions retain the reviewer and reason.</p><ul>",
    );
    for hit in &view.leak_hits {
        html.push_str(&format!("<li><p><strong>{}</strong> · {} · <code>{}</code></p><p>Trial <code>{}</code> · attempt {} · <a href=\"/experiments/{}/sessions/{}\">Session evidence</a> · Session evidence line {}</p>", hit.status(), hit.kind, esc(&hit.identifier), esc(&hit.trial_id), hit.attempt + 1, esc(&view.description.id), esc(&hit.session), hit.line));
        if let Some(decision) = &hit.decision {
            html.push_str(&format!(
                "<p>{} · {} · {}</p>",
                esc(&decision.reviewer.to_string()),
                when(&decision.recorded_at),
                esc(&decision.reason)
            ));
        } else if can_act && !active(view) {
            html.push_str(&format!("<form method=\"post\" action=\"/experiments/{}/leaks/{}\"><input type=\"hidden\" name=\"csrf\" value=\"{}\"><label>Reason <input name=\"reason\" required></label><button name=\"decision\" value=\"dismiss\">Dismiss false positive</button><button name=\"decision\" value=\"confirm\">Confirm leak</button></form>", esc(&view.description.id), esc(&hit.id), esc(csrf)));
        } else if active(view) {
            html.push_str("<p>Decide after the Experiment stops.</p>");
        }
        html.push_str("</li>");
    }
    html.push_str("</ul></section>");
    html
}

fn points(value: f64) -> String {
    format!("{:+.0}", value * 100.0)
}

fn interval(
    bounds: crate::verdict::Interval,
    verdict: Decision,
    margin: f64,
    label: &str,
) -> String {
    let lower = bounds.lower;
    let upper = bounds.upper;
    let pos = |value: f64| ((value + 0.5) * 100.0).clamp(0.0, 100.0);
    let range = format!("{} to {} points", points(lower), points(upper));
    format!(
        "<span class=\"experiment-interval\" data-tone=\"{}\" role=\"img\" aria-label=\"{}: {}; margin {} points; axis −50 to +50\"><svg viewBox=\"0 0 100 16\" preserveAspectRatio=\"none\" aria-hidden=\"true\"><line class=\"interval-axis\" x1=\"0\" x2=\"100\" y1=\"8\" y2=\"8\"/><line class=\"interval-margin\" x1=\"{}\" x2=\"{}\" y1=\"0\" y2=\"16\"/><line class=\"interval-zero\" x1=\"50\" x2=\"50\" y1=\"0\" y2=\"16\"/><rect class=\"interval-bar\" x=\"{}\" y=\"4\" width=\"{}\" height=\"8\"/>{}{}</svg></span><span class=\"interval-range\">{}</span>",
        decision(verdict),
        esc(label),
        esc(&range),
        points(margin),
        pos(margin),
        pos(margin),
        pos(lower),
        pos(upper) - pos(lower),
        if lower < -0.5 {
            "<path class=\"interval-arrow\" d=\"M3 4L0 8L3 12\"/>"
        } else {
            ""
        },
        if upper > 0.5 {
            "<path class=\"interval-arrow\" d=\"M97 4L100 8L97 12\"/>"
        } else {
            ""
        },
        range
    )
}

fn list_intervals(view: &View) -> String {
    view.verdict.as_ref().map_or(String::new(), |v| {
        v.suites
            .iter()
            .map(|(name, suite)| {
                let suite = &suite.statistics;
                let (bounds, level, kind) = if suite.accuracy.verdict == Decision::Fail {
                    (
                        suite.accuracy.intervals.fail(),
                        suite.bar.levels.fail(),
                        "fail",
                    )
                } else {
                    (
                        suite.accuracy.intervals.pass(),
                        suite.bar.levels.pass(),
                        "pass",
                    )
                };
                let label = format!("{name} accuracy · {kind} ({:.0}%)", level * 100.0);
                format!(
                    "<p class=\"experiment-list-interval\"><span>{}</span>{}</p>",
                    esc(&label),
                    interval(
                        bounds,
                        suite.accuracy.verdict,
                        view.description.bar.margin,
                        &label
                    )
                )
            })
            .collect()
    })
}

fn verdict(view: &View) -> String {
    let Some(verdict) = &view.verdict else {
        return String::new();
    };
    let mut html =
        String::from("<section class=\"experiment-verdict\" aria-label=\"Verdict by suite\">");
    if let Some(line) = &verdict.line {
        html.push_str(&format!(
            "<p class=\"experiment-resource-verdict\">{}</p>",
            esc(line)
        ));
    }
    if let Some(deciders) = &verdict.deciders {
        html.push_str(&format!("<p>{}</p>", esc(&deciders.line())));
    }
    // A Verdict on the people's sample alone counts pairs, not Eval tasks.
    let units = if crate::evals::experiment::deciders::sample_only(&view.description.bar) {
        "pairs"
    } else {
        "Eval tasks"
    };
    for (name, suite) in &verdict.suites {
        let advisory = suite.advisory == Some(true);
        let counts = suite.counts.as_ref();
        let suite = &suite.statistics;
        // The gist shows the interval that decided each metric, as the list does.
        let decided = |assessment: &crate::verdict::Assessment| {
            if assessment.verdict == Decision::Fail {
                assessment.intervals.fail()
            } else {
                assessment.intervals.pass()
            }
        };
        let (accuracy, wrong_findings) = (decided(&suite.accuracy), decided(&suite.wrong_findings));
        // Folded to its verdict and intervals; open it for the figures and why.
        html.push_str(&format!(
            "<details class=\"experiment-suite\" data-suite=\"{}\"><summary><h2>{} <span class=\"experiment-tag\" data-tone=\"{}\">{}</span>{}</h2> <span class=\"experiment-gist\">Accuracy {} to {} · Wrong findings {} to {} points</span></summary>",
            esc(name),
            esc(name),
            decision(suite.verdict),
            decision(suite.verdict),
            if advisory { " <span class=\"experiment-tag\">Advisory suite</span>" } else { "" },
            points(accuracy.lower),
            points(accuracy.upper),
            points(wrong_findings.lower),
            points(wrong_findings.upper)
        ));
        if let Some(counts) = counts {
            html.push_str(&format!(
                "<p>{} judged · {} measurement-only · {} void-dropped · {} other-setup.</p>",
                counts.judged, counts.measurement_only, counts.void_dropped, counts.other_setup
            ));
        }
        if advisory {
            html.push_str("<p>This Advisory suite's Verdict cannot pass or block a release.</p>");
        }
        for (metric, assessment) in [
            ("Accuracy", &suite.accuracy),
            ("Wrong findings", &suite.wrong_findings),
        ] {
            let intervals = match assessment.intervals {
                crate::verdict::Intervals::Separate {
                    pass_interval,
                    fail_interval,
                } => vec![
                    ("pass", suite.bar.levels.pass(), pass_interval),
                    ("fail", suite.bar.levels.fail(), fail_interval),
                ],
                crate::verdict::Intervals::Legacy { interval } => {
                    vec![("credible", suite.bar.levels.pass(), interval)]
                }
            };
            for (kind, level, bounds) in intervals {
                let label = format!("{metric} · {kind} ({:.0}%)", level * 100.0);
                html.push_str(&format!("<div class=\"experiment-figure\"><span>{} <span class=\"experiment-tag\" data-tone=\"{}\">{}</span></span>{}</div>", esc(&label), decision(assessment.verdict), decision(assessment.verdict), interval(bounds, assessment.verdict, view.description.bar.margin, &label)));
            }
            let why = match assessment.verdict {
                Decision::Pass if suite.bar.levels.pass_rule() == PassRule::Margin => format!(
                    "At the pass level, the {metric} interval sits above the {}-point margin. The interval at the fail level reaches zero or above.",
                    points(view.description.bar.margin)
                ),
                Decision::Pass => format!(
                    "At the pass level, the {metric} interval sits above the {}-point margin and reaches zero or above. The interval at the fail level reaches zero or above too.",
                    points(view.description.bar.margin)
                ),
                Decision::Fail => {
                    format!("At the fail level, the whole {metric} interval is below zero.")
                }
                Decision::Inconclusive => format!(
                    "The {metric} intervals establish neither a pass nor a fail under the Bar."
                ),
            };
            html.push_str(&format!("<p class=\"experiment-why\">{why}"));
            if assessment.worse_within_margin {
                html.push_str(&format!(" <span class=\"experiment-tag\" data-tone=\"inconclusive\">worse, within the margin</span> The whole {metric} interval at the pass level is below zero; the Bar's margin tolerates it."));
            }
            if let Some(more) = &assessment.more_tasks {
                if let Some(count) = more.additional_tasks {
                    html.push_str(&format!(
                        " About {count} more {units} would settle it ({:.0}% chance).",
                        more.target_probability * 100.0
                    ));
                } else {
                    html.push_str(&format!(" The forecast's search budget did not find how many more {units} would settle it."));
                }
            }
            html.push_str("</p>");
        }
        html.push_str("<p class=\"experiment-axis\" aria-hidden=\"true\">−50 ··············· 0 ··············· +50 points</p>");
        html.push_str(&format!("<p>{} retained {units}; {} dropped{}.</p>", suite.retained_tasks, suite.dropped_tasks, if suite.dropped_tasks > (suite.retained_tasks + suite.dropped_tasks) / 10 { "; more than a tenth dropped, so the suite is inconclusive whatever the intervals say" } else { "" }));
        if let Some(pass) = suite.pass_cubed {
            html.push_str(&format!(
                "<p>pass³ (all three Trials passed): Baseline {:.0}%, Candidate {:.0}%.</p>",
                pass.baseline * 100.0,
                pass.candidate * 100.0
            ));
        }
        html.push_str("</details>");
    }
    html.push_str(&format!("</section><p class=\"experiment-dim\">Points of pass share, Candidate minus Baseline. Each interval is labelled with its credible level. The axis runs from −50 to +50; the dashed line is the {}-point margin. Arrows mark intervals continuing beyond the axis. For wrong findings, plus means fewer in the Candidate.</p>", points(view.description.bar.margin)));
    html
}

fn recheck_command(view: &View, pending: &Pending) -> String {
    let passed = view
        .results
        .iter()
        .find(|r| r.key() == pending.id)
        .and_then(|r| r.score.as_ref())
        .is_some_and(|score| score.effective().passed);
    format!(
        "thirdshift recheck {} {} {} --reason '…'",
        view.description.id,
        pending.id,
        if passed { "pass" } else { "fail" }
    )
}

fn recheck_why(pending: &Pending) -> String {
    let mut reasons: Vec<String> = Vec::new();
    if pending.borderline {
        reasons.push("the Grader marked it borderline".into());
    }
    for flip in &pending.decisive_flips {
        reasons.push(
            match flip {
                Flip::Passed => "flipping pass or fail would change the Verdict",
                Flip::WrongFinding => "flipping the wrong-finding flag would change the Verdict",
            }
            .into(),
        );
    }
    if pending.decisive && pending.decisive_flips.is_empty() {
        reasons.push(
            "a Score flip would change the Verdict (the older record does not name the flag)"
                .into(),
        );
    }
    reasons.join("; ")
}

fn rechecks(view: &View) -> String {
    if view.pending_rechecks.is_empty() {
        return String::new();
    }
    let mut html = format!(
        "<section class=\"experiment-callout\" id=\"experiment-rechecks\"><h2>Waiting on you</h2><p>{} rechecks ({} borderline, {} decisive; some are both). The Verdict is final once you have rechecked them all. Record yours from the CLI; it stays beside the Grader's Score.</p><button type=\"button\" data-pending-only aria-pressed=\"false\">Show only their Eval tasks below</button><ol data-rechecks>",
        view.pending_rechecks.len(),
        view.pending_rechecks
            .iter()
            .filter(|p| p.borderline)
            .count(),
        view.pending_rechecks.iter().filter(|p| p.decisive).count()
    );
    for (index, pending) in view.pending_rechecks.iter().enumerate() {
        html.push_str(&format!("<li{}><button type=\"button\" class=\"experiment-link\" data-experiment-peek=\"trial-{}\">{} · {} Trial {}</button><span>{}</span><button type=\"button\" data-copy=\"{}\">Copy command</button></li>", if index >= 5 { " data-recheck-extra hidden" } else { "" }, esc(&pending.id), esc(&pending.task), arm_title(pending.arm), pending.trial+1, esc(&recheck_why(pending)), esc(&recheck_command(view,pending))));
    }
    html.push_str("</ol>");
    if view.pending_rechecks.len() > 5 {
        html.push_str(&format!("<button type=\"button\" data-all-rechecks=\"{}\" aria-expanded=\"false\">Show all {}</button>", view.pending_rechecks.len(), view.pending_rechecks.len()));
    }
    html.push_str("<p data-copy-status role=\"status\"></p></section>");
    html
}

fn grid(view: &View) -> String {
    let mut html = String::from(
        "<section id=\"experiment-trials\"><h2>Trials</h2><p class=\"experiment-legend\" aria-label=\"Legend\">✓ passed · ✗ failed · ! wrong finding · ? borderline · ◇ recheck waiting · ↺ rechecked · ⊘ void · – not graded · ◌ running · · queued</p>",
    );
    for suite in &view.description.suites {
        let n = view.description.bar.trials_per_task[suite];
        let tasks = view
            .tasks
            .iter()
            .filter(|task| &task.suite == suite)
            .count();
        html.push_str(&format!("<details class=\"trial-suite\" data-suite=\"{}\"><summary><h3>{}</h3> <span class=\"experiment-gist\">{tasks} Eval tasks · {n} {} per Arm each</span></summary><div role=\"table\" aria-label=\"{} Trials\" class=\"trial-grid\" data-trials=\"{n}\"><div role=\"row\" class=\"trial-row\"><span role=\"columnheader\">Eval task</span><span role=\"columnheader\">Baseline</span><span role=\"columnheader\">Candidate</span><span role=\"columnheader\">Diff.</span></div>", esc(suite), esc(suite), if n == 1 { "Trial" } else { "Trials" }, esc(suite)));
        for task in view.tasks.iter().filter(|task| &task.suite == suite) {
            let cells: Vec<_> = Cell::for_task(view, suite, &task.name).collect();
            let dropped = cells.iter().any(|cell| {
                cell.terminal && cell.result.is_some_and(|r| r.outcome == Outcome::Void)
            });
            let complete = cells.iter().all(Cell::graded);
            let difference: i32 = cells
                .iter()
                .filter(|c| c.passed())
                .map(|c| if c.arm.name() == "candidate" { 1 } else { -1 })
                .sum();
            let pending = view
                .pending_rechecks
                .iter()
                .any(|p| p.suite == *suite && p.task == task.name);
            let task_id = format!("task-{}", trial_key(suite, &task.name, Arm::Baseline, 0));
            html.push_str(&format!("<div role=\"row\" class=\"trial-row{}\" data-task=\"{}\" data-pending=\"{pending}\"><span role=\"rowheader\"><button type=\"button\" class=\"experiment-link trial-task\" data-experiment-peek=\"{task_id}\">{}</button></span>", if dropped { " trial-dropped" } else { "" }, esc(&task.name), esc(&task.name)));
            for arm in [Arm::Baseline, Arm::Candidate] {
                html.push_str("<span role=\"cell\" class=\"trial-cells\">");
                for cell in cells.iter().filter(|cell| cell.arm.name() == arm.name()) {
                    html.push_str(&cell.button());
                }
                html.push_str("</span>");
            }
            let diff = if dropped {
                "dropped".into()
            } else if complete {
                format!("{:+.0}", difference as f64 / n as f64 * 100.0)
            } else {
                "…".into()
            };
            html.push_str(&format!("<span role=\"cell\">{diff}</span></div>"));
            html.push_str(&format!("<template id=\"{task_id}\"><section aria-label=\"Eval task details\"><h2>{}</h2><p>{}</p>", esc(&task.name), if dropped { "Dropped: a Void Trial removes this Eval task from both Arms." } else if complete { "Both Arms' graded Trials." } else { "Not every Trial is finished and graded yet." }));
            for arm in [Arm::Baseline, Arm::Candidate] {
                html.push_str(&format!("<h3>{}</h3><ul>", arm_title(arm)));
                for cell in cells.iter().filter(|c| c.arm.name() == arm.name()) {
                    html.push_str(&format!(
                        "<li>{} {}</li>",
                        cell.button(),
                        cell.result
                            .and_then(|r| r.score.as_ref())
                            .and_then(|s| s.model.as_ref())
                            .map(|s| esc(&s.reason))
                            .unwrap_or_default()
                    ));
                }
                html.push_str("</ul>");
            }
            html.push_str("</section></template>");
            for cell in &cells {
                html.push_str(&format!(
                    "<template id=\"trial-{}\">{}</template>",
                    cell.key,
                    cell.peek()
                ));
            }
        }
        html.push_str("</div></details>");
    }
    html.push_str("</section>");
    html
}

fn arm_title(arm: Arm) -> &'static str {
    if arm.name() == "baseline" {
        "Baseline"
    } else {
        "Candidate"
    }
}

struct Cell<'a> {
    view: &'a View,
    task: &'a str,
    suite: &'a str,
    arm: Arm,
    index: usize,
    key: String,
    result: Option<&'a TrialResult>,
    terminal: bool,
}

struct Mark {
    label: String,
    symbol: &'static str,
    tone: &'static str,
    decorations: String,
}

impl Mark {
    fn plain(label: &str, symbol: &'static str, tone: &'static str) -> Self {
        Self {
            label: label.into(),
            symbol,
            tone,
            decorations: String::new(),
        }
    }
}

impl<'a> Cell<'a> {
    fn for_task(view: &'a View, suite: &'a str, task: &'a str) -> impl Iterator<Item = Self> {
        let n = view.description.bar.trials_per_task[suite];
        [Arm::Baseline, Arm::Candidate]
            .into_iter()
            .flat_map(move |arm| (0..n).map(move |index| Self::new(view, suite, task, arm, index)))
    }

    fn new(view: &'a View, suite: &'a str, task: &'a str, arm: Arm, index: usize) -> Self {
        let key = trial_key(suite, task, arm, index);
        let result = view.results.iter().find(|r| r.key() == key);
        let terminal = result.is_some();
        let result = result.or_else(|| view.unfinished_trials.iter().find(|r| r.key() == key));
        Self {
            view,
            task,
            suite,
            arm,
            index,
            key,
            result,
            terminal,
        }
    }
    fn graded(&self) -> bool {
        self.terminal
            && self.result.is_some_and(|r| {
                r.outcome != Outcome::Void
                    && r.score.as_ref().is_some_and(|s| {
                        r.outcome == Outcome::Delivered
                            || s.model.is_some()
                            || s.checks.iter().any(|c| !c.passed)
                    })
            })
    }
    fn passed(&self) -> bool {
        self.graded()
            && self
                .result
                .and_then(|r| r.score.as_ref())
                .is_some_and(|s| s.effective().passed)
    }
    fn mark(&self) -> Mark {
        if !self.terminal && self.view.running_trials.iter().any(|r| r.id == self.key) {
            return Mark::plain("Running", "◌", "working");
        }
        let Some(result) = self.result else {
            return Mark::plain("Queued", "·", "queued");
        };
        if result.outcome == Outcome::Void {
            return Mark::plain(
                if self.terminal {
                    "Void"
                } else {
                    "Void, awaiting retry"
                },
                "⊘",
                "look",
            );
        }
        if !self.graded() {
            return Mark::plain("Not graded", "–", "queued");
        }
        let score = result.score.as_ref().unwrap();
        let passed = score.effective().passed;
        let mut label = if passed { "Passed" } else { "Failed" }.to_owned();
        let mut decorations = String::new();
        if score.effective().wrong_finding {
            label.push_str(", wrong finding");
            decorations.push('!');
        }
        if score.borderline {
            label.push_str(", borderline");
            decorations.push('?');
        }
        if !score.rechecks.is_empty() {
            label.push_str(", rechecked");
            decorations.push('↺');
        }
        if self.view.pending_rechecks.iter().any(|p| p.id == self.key) {
            label.push_str(", recheck waiting");
            decorations.push('◇');
        }
        Mark {
            label,
            symbol: if passed { "✓" } else { "✗" },
            tone: if passed { "pass" } else { "fail" },
            decorations,
        }
    }
    fn button(&self) -> String {
        let Mark {
            label,
            symbol: mark,
            tone,
            decorations,
        } = self.mark();
        format!(
            "<button type=\"button\" class=\"trial-cell\" data-tone=\"{tone}\" data-experiment-peek=\"trial-{}\" title=\"{}\" aria-label=\"{} Trial {}: {}\">{mark}<sup aria-hidden=\"true\">{decorations}</sup></button>",
            self.key,
            esc(&label),
            arm_title(self.arm),
            self.index + 1,
            esc(&label)
        )
    }
    fn sessions(&self, index: usize) -> String {
        self.view
            .sessions
            .iter()
            .filter(|log| log.trial_id == self.key && log.attempt == index)
            .map(|log| {
                format!(
                    "<a href=\"/experiments/{}/sessions/{}\">{}</a>",
                    self.view.description.id,
                    log.id,
                    if log.trace {
                        "Session trace"
                    } else {
                        "Session log"
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }
    fn peek(&self) -> String {
        let label = self.mark().label;
        let mut html = format!(
            "<section aria-label=\"Trial details\"><h2>{} · Trial {}</h2><p>{} · {}</p><p><strong>{}</strong></p>",
            arm_title(self.arm),
            self.index + 1,
            esc(self.suite),
            esc(self.task),
            esc(&label)
        );
        if let Some(pending) = self.view.pending_rechecks.iter().find(|p| p.id == self.key) {
            let command = recheck_command(self.view, pending);
            html.push_str(&format!("<div class=\"experiment-callout\"><p>Waiting on your recheck: {}.</p><code>{}</code><button type=\"button\" data-copy=\"{}\">Copy command</button><p data-copy-status role=\"status\"></p></div>", esc(&recheck_why(pending)), esc(&command), esc(&command)));
        }
        if let Some(result) = self.result {
            if result.suspended() {
                html.push_str("<p><strong>Suspended</strong> · left out of timing; outcome and cost still count.</p>");
            }
            if let Some(score) = &result.score {
                html.push_str("<dl><dt>Model Grader</dt><dd>");
                if let Some(model) = &score.model {
                    html.push_str(&format!(
                        "{}{}. {} <p>Blind-packet label: <code>{}</code></p>",
                        if model.passed { "Pass" } else { "Fail" },
                        if model.borderline { ", borderline" } else { "" },
                        esc(&model.reason),
                        esc(&model.label)
                    ));
                } else {
                    html.push_str("No Model Grader assessment.");
                }
                html.push_str("</dd>");
                for recheck in &score.rechecks {
                    html.push_str(&format!(
                        "<dt>Your recheck</dt><dd>{}: {} · {} · wrong finding: {}</dd>",
                        if recheck.passed { "Pass" } else { "Fail" },
                        esc(&recheck.reason),
                        when(&recheck.recorded_at),
                        recheck.wrong_finding.map_or("unchanged", |wrong| if wrong {
                            "yes"
                        } else {
                            "no"
                        })
                    ));
                }
                html.push_str(&format!(
                    "<dt>Wrong finding</dt><dd>{}</dd><dt>Code checks</dt><dd><ul>",
                    if score.effective().wrong_finding {
                        "Yes"
                    } else {
                        "No"
                    }
                ));
                for check in &score.checks {
                    html.push_str(&format!(
                        "<li>{} {}: {}</li>",
                        if check.passed { "✓" } else { "✗" },
                        esc(&check
                            .command
                            .as_ref()
                            .map(|c| c.join(" "))
                            .unwrap_or_default()),
                        esc(&check.reason)
                    ));
                }
                html.push_str("</ul></dd></dl>");
            }
            html.push_str("<h3>Attempts</h3><ol>");
            for (index, attempt) in result.attempts.iter().enumerate() {
                html.push_str(&format!(
                    "<li>{}{}{} · {:.1} minutes · {}",
                    attempt.outcome,
                    attempt
                        .void_cause
                        .map(|c| format!(": {}", esc(&c.to_string())))
                        .unwrap_or_default(),
                    attempt
                        .cause
                        .as_ref()
                        .map(|c| format!(" · {}", esc(c)))
                        .unwrap_or_default(),
                    attempt.timings.total_ms as f64 / 60_000.0,
                    self.sessions(index)
                ));
                if !attempt.steps.is_empty() {
                    html.push_str("<h4>Steps</h4><ol>");
                    for step in &attempt.steps {
                        html.push_str(&format!(
                            "<li>{} · {} · {:.1}s · {} to {}</li>",
                            esc(&step.name),
                            step.outcome,
                            step.elapsed_ms as f64 / 1000.0,
                            when(&step.started_at),
                            when(&step.ended_at)
                        ));
                    }
                    html.push_str("</ol>");
                }
                html.push_str("</li>");
            }
            html.push_str("</ol>");
        }
        if let Some(running) = self.view.running_trials.iter().find(|r| r.id == self.key) {
            html.push_str(&format!(
                "<p>Started {}</p><p>{}</p>",
                when(&running.started_at),
                self.view
                    .sessions
                    .iter()
                    .filter(|log| log.trial_id == self.key
                        && self.result.is_none_or(|r| log.attempt >= r.attempts.len()))
                    .map(|log| self.sessions(log.attempt))
                    .collect::<Vec<_>>()
                    .join(" · ")
            ));
        } else if self.result.is_none() {
            html.push_str("<p>Queued. It starts when a worker is free, or after the paused Experiment resumes.</p>");
            let logs: std::collections::BTreeSet<_> = self
                .view
                .sessions
                .iter()
                .filter(|log| log.trial_id == self.key)
                .map(|log| log.attempt)
                .collect();
            for attempt in logs {
                html.push_str(&format!("<p>{}</p>", self.sessions(attempt)));
            }
        }
        let setup = if self.arm.name() == "baseline" {
            &self.view.description.baseline
        } else {
            &self.view.description.candidate
        };
        html.push_str(&format!("<dl><dt>Arm</dt><dd>{} · {} · Model {} · Effort {} · <code>{}</code></dd><dt>Trial ID</dt><dd><code>{}</code></dd></dl></section>", arm_title(self.arm), esc(&setup.harness), esc(setup.model.as_deref().unwrap_or("default")), esc(setup.effort.as_deref().unwrap_or("default")), esc(&setup.commit), self.key));
        html
    }
}

fn table_row(label: &str, baseline: &str, candidate: &str) -> String {
    format!(
        "<tr><th scope=\"row\">{}</th><td>{}</td><td{}>{}</td></tr>",
        esc(label),
        esc(baseline),
        if baseline != candidate {
            " class=\"experiment-differs\""
        } else {
            ""
        },
        esc(candidate)
    )
}

struct ArmStats {
    passed: usize,
    wrong_findings: usize,
    all_passed_tasks: usize,
    trials: usize,
}

impl ArmStats {
    fn share(&self) -> String {
        if self.trials == 0 {
            "not graded".into()
        } else {
            format!(
                "{}/{} ({:.0}%)",
                self.passed,
                self.trials,
                self.passed as f64 / self.trials as f64 * 100.0
            )
        }
    }
}

fn arms(view: &View) -> String {
    let d = &view.description;
    let mut html = String::from(
        "<section id=\"experiment-arms\"><details><summary><h2>What was compared</h2></summary><table class=\"experiment-arms\"><thead><tr><td></td><th scope=\"col\">Baseline</th><th scope=\"col\">Candidate</th></tr></thead><tbody>",
    );
    for (label, b, c) in [
        (
            "Build",
            format!(
                "{} · {}",
                view.details
                    .baseline_ref
                    .as_deref()
                    .unwrap_or("ref unknown"),
                short(&d.baseline.commit)
            ),
            format!(
                "{} · {}",
                view.details
                    .candidate_ref
                    .as_deref()
                    .unwrap_or("ref unknown"),
                short(&d.candidate.commit)
            ),
        ),
        (
            "Harness",
            d.baseline.harness.clone(),
            d.candidate.harness.clone(),
        ),
        (
            "Model",
            d.baseline.model.clone().unwrap_or_else(|| "default".into()),
            d.candidate
                .model
                .clone()
                .unwrap_or_else(|| "default".into()),
        ),
        (
            "Effort",
            d.baseline
                .effort
                .clone()
                .unwrap_or_else(|| "default".into()),
            d.candidate
                .effort
                .clone()
                .unwrap_or_else(|| "default".into()),
        ),
        (
            "Harness version",
            d.prepared
                .as_ref()
                .map(|p| p.baseline.harness.version.clone())
                .unwrap_or_else(|| "unknown".into()),
            d.prepared
                .as_ref()
                .map(|p| p.candidate.harness.version.clone())
                .unwrap_or_else(|| "unknown".into()),
        ),
    ] {
        html.push_str(&table_row(label, &b, &c));
    }
    for suite in &d.suites {
        let n = d.bar.trials_per_task[suite];
        let retained: Vec<_> = view
            .tasks
            .iter()
            .filter(|task| &task.suite == suite)
            .filter(|task| Cell::for_task(view, suite, &task.name).all(|cell| cell.graded()))
            .collect();
        let stats = |arm| {
            let cells: Vec<_> = retained
                .iter()
                .flat_map(|task| {
                    (0..n).map(move |index| Cell::new(view, suite, &task.name, arm, index))
                })
                .collect();
            let passed = cells.iter().filter(|c| c.passed()).count();
            let wrong = cells
                .iter()
                .filter(|c| {
                    c.result
                        .and_then(|r| r.score.as_ref())
                        .is_some_and(|s| s.effective().wrong_finding)
                })
                .count();
            let all = retained
                .iter()
                .filter(|task| (0..n).all(|i| Cell::new(view, suite, &task.name, arm, i).passed()))
                .count();
            ArmStats {
                passed,
                wrong_findings: wrong,
                all_passed_tasks: all,
                trials: cells.len(),
            }
        };
        let b = stats(Arm::Baseline);
        let c = stats(Arm::Candidate);
        html.push_str(&table_row(
            &format!("{suite} pass share"),
            &b.share(),
            &c.share(),
        ));
        if n == 3 {
            html.push_str(&table_row(
                &format!("{suite} pass³"),
                &format!("{}/{}", b.all_passed_tasks, retained.len()),
                &format!("{}/{}", c.all_passed_tasks, retained.len()),
            ));
        }
        html.push_str(&table_row(
            &format!("{suite} wrong findings"),
            &format!("{} of {}", b.wrong_findings, b.trials),
            &format!("{} of {}", c.wrong_findings, c.trials),
        ));
    }
    html.push_str(&table_row(
        "Minutes",
        &format!("{:.2}", view.minutes.baseline),
        &format!(
            "{:.2} {}",
            view.minutes.candidate,
            change(Some(view.minutes.baseline), Some(view.minutes.candidate))
        ),
    ));
    let b_tokens = view.usage.baseline.tokens();
    let c_tokens = view.usage.candidate.tokens();
    html.push_str(&table_row(
        "Tokens in + out",
        &b_tokens.map_or("unknown".into(), |n| n.to_string()),
        &format!(
            "{} {}",
            c_tokens.map_or("unknown".into(), |n| n.to_string()),
            change(b_tokens.map(|n| n as f64), c_tokens.map(|n| n as f64))
        ),
    ));
    html.push_str(&table_row(
        "Cost (USD)",
        &view
            .usage
            .baseline
            .cost_usd
            .map_or("unknown".into(), |n| format!("${n:.4}")),
        &format!(
            "{} {}",
            view.usage
                .candidate
                .cost_usd
                .map_or("unknown".into(), |n| format!("${n:.4}")),
            change(view.usage.baseline.cost_usd, view.usage.candidate.cost_usd)
        ),
    ));
    html.push_str(&table_row(
        "Void Trials",
        &view.voids.baseline.to_string(),
        &view.voids.candidate.to_string(),
    ));
    html.push_str("</tbody></table><p>The Bar's optional cost and time ceilings apply to frozen-price accounting. This table shows Harness-reported usage; unknown usage was not reported by the Harness.</p></details></section>");
    html
}

fn change(baseline: Option<f64>, candidate: Option<f64>) -> String {
    match (baseline, candidate) {
        (Some(b), Some(c)) if b > 0.0 => format!("({:+.0}%)", (c - b) / b * 100.0),
        (Some(0.0), Some(0.0)) => "(0%)".into(),
        _ => String::new(),
    }
}

fn bar(view: &View) -> String {
    let b = &view.description.bar;
    let ceiling = |ratio: Option<f64>| {
        ratio.map_or_else(
            || "Off".into(),
            |ratio| format!("{ratio:.3}× Candidate/Baseline"),
        )
    };
    let ceilings = format!(
        "<dt>Cost ceiling</dt><dd>{}</dd><dt>Time ceiling</dt><dd>{}</dd>",
        ceiling(b.cost_ceiling),
        ceiling(b.time_ceiling)
    );
    let levels = match b.levels {
        crate::verdict::Levels::Separate {
            pass_level,
            fail_level,
            pass_rule,
        } => format!(
            "<dt>Pass level</dt><dd>{:.0}%</dd><dt>Fail level</dt><dd>{:.0}%</dd><dt>Pass rule</dt><dd>{}</dd>",
            pass_level * 100.0,
            fail_level * 100.0,
            match pass_rule {
                crate::verdict::PassRule::Margin =>
                    "The pass-level interval's lower bound is above the margin.",
                crate::verdict::PassRule::ReachesZero =>
                    "The pass-level interval's lower bound is above the margin and its upper bound reaches zero.",
            }
        ),
        crate::verdict::Levels::Legacy { credible_level } => format!(
            "<dt>Credible level</dt><dd>{:.0}%</dd>",
            credible_level * 100.0
        ),
    };
    let minimum = b
        .min_discriminating_tasks
        .map_or_else(String::new, |minimum| {
            format!(
                "<dt>Advisory minimum</dt><dd>{minimum} discriminating Eval tasks per suite</dd>"
            )
        });
    let method = view
        .verdict
        .as_ref()
        .map(|v| {
            v.suites
                .iter()
                .map(|(suite, v)| {
                    format!(
                        "<dt>{} method and version</dt><dd>{} · thirdshift {}</dd>",
                        esc(suite),
                        esc(&v.statistics.method),
                        esc(&v.statistics.thirdshift_version)
                    )
                })
                .collect::<String>()
        })
        .unwrap_or_default();
    let deciders = {
        use crate::evals::experiment::deciders;
        let panel = b.panel.as_ref().map_or_else(
            || "off".to_owned(),
            |panel| {
                format!(
                    "{} · {} seats · stands in for people only with {} voting families and people–panel agreement of at least {}",
                    deciders::panel_role(b).name(),
                    panel.seats.len(),
                    panel.families,
                    panel.agreement
                )
            },
        );
        let people = b.people.as_ref().map_or_else(
            || "beside · anyone may read any pair".to_owned(),
            |people| {
                format!(
                    "{} · readers: {} · {} · {} shared",
                    people.role.name(),
                    people.readers.join(", "),
                    people.sample.map_or_else(
                        || "every pair".to_owned(),
                        |sample| format!("{sample} pairs each, drawn from the seed")
                    ),
                    people.overlap
                )
            },
        );
        format!(
            "<dt>Grader panel</dt><dd>{}</dd><dt>People</dt><dd>{}</dd>",
            esc(&panel),
            esc(&people)
        )
    };
    format!(
        "<details id=\"experiment-bar\"><summary>The Bar</summary><dl><dt>Margin</dt><dd>{:.0} points</dd>{levels}{minimum}{ceilings}{deciders}<dt>Prior</dt><dd>{} pseudo Eval tasks, spread evenly over possible differences</dd><dt>Seed and draws</dt><dd>{}, {}</dd><dt>Trials per Eval task</dt><dd>{}</dd><dt>Timeout</dt><dd>{} seconds</dd><dt>Void rules</dt><dd>Up to {} fresh retries. Causes: {}. An exhausted Eval task drops out of both Arms. More than a tenth of judged tasks dropped in suites that are not advisory makes the overall Verdict inconclusive.</dd><dt>Recorded</dt><dd>{}, before the first Trial</dd>{method}</dl></details>",
        b.margin * 100.0,
        b.prior,
        b.seed,
        b.draws,
        esc(&b
            .trials_per_task
            .iter()
            .map(|(suite, n)| format!("{suite}: {n} per Arm"))
            .collect::<Vec<_>>()
            .join(" · ")),
        b.timeout_seconds,
        b.void_rules.retries,
        esc(&b
            .void_rules
            .causes
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")),
        when(&view.created_at)
    )
}
