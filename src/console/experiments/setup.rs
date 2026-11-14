//! Arm identity for the comparison, Trial Peek and the post-call reveal.
use crate::console::filters::escaped as esc;
use crate::evals::experiment::{CustomRunner, PreparedArm, Setup};

pub(super) fn fields(
    setup: &Setup,
    prepared: Option<&PreparedArm>,
    reference: Option<&str>,
) -> Vec<(&'static str, String)> {
    if setup.runner.is_some() {
        return runner_fields(setup, prepared.and_then(|arm| arm.runner.as_ref()));
    }
    vec![
        (
            "Build",
            format!(
                "{} · {}",
                reference.unwrap_or("ref unknown"),
                setup.reference().chars().take(10).collect::<String>()
            ),
        ),
        ("Harness", setup.executor().into()),
        (
            "Model",
            setup.model.clone().unwrap_or_else(|| "default".into()),
        ),
        (
            "Effort",
            setup.effort.clone().unwrap_or_else(|| "default".into()),
        ),
        (
            "Harness version",
            prepared
                .and_then(|arm| arm.harness.as_ref())
                .map(|h| h.version.clone())
                .unwrap_or_else(|| "unknown".into()),
        ),
    ]
}

fn runner_fields(setup: &Setup, runner: Option<&CustomRunner>) -> Vec<(&'static str, String)> {
    vec![
        ("Custom runner", setup.runner.clone().unwrap_or_default()),
        (
            "Runner digest",
            setup
                .runner_digest
                .clone()
                .unwrap_or_else(|| "not recorded".into()),
        ),
        (
            "Git provenance",
            runner
                .map(|runner| {
                    runner
                        .provenance
                        .iter()
                        .map(|(source, git)| {
                            format!(
                                "{source}: HEAD {} · {}{}",
                                git.commit,
                                if git.dirty { "dirty" } else { "clean" },
                                git.origin
                                    .as_ref()
                                    .map(|origin| format!(" · {origin}"))
                                    .unwrap_or_default()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                })
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| "not recorded".into()),
        ),
        (
            "Roles",
            runner
                .map(|runner| {
                    runner
                        .manifest
                        .roles
                        .iter()
                        .map(|(role, choice)| {
                            format!(
                                "{role}: Model {}{}",
                                choice.model,
                                choice
                                    .effort
                                    .as_ref()
                                    .map(|effort| format!(" · Effort {effort}"))
                                    .unwrap_or_default()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                })
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| "none declared".into()),
        ),
    ]
}

pub(super) fn runner_details(setup: &Setup, runner: Option<&CustomRunner>) -> String {
    let mut html = String::from("<dl>");
    for (label, value) in runner_fields(setup, runner) {
        html.push_str(&format!("<dt>{label}</dt><dd>{}</dd>", esc(&value)));
    }
    html.push_str("</dl>");
    html
}
