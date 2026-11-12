//! Correct invalid session output once; GitHub checks and publication stay outside this loop.

use anyhow::{Context, Result, bail};

use super::{
    Recommendation, existing, issue_category, read_recommendation, resolve_recommendation,
};
use crate::issue::IssueUrl;
use crate::prompt;
use crate::session::{Kind, Purpose, Sessions};

pub(super) fn recommendation(
    sessions: &Sessions<'_>,
    instructions: &str,
    target: Option<&IssueUrl>,
    categories: &[String],
) -> Result<Recommendation> {
    // An execution failure is not invalid recommendation output.
    let message = sessions.run_to_final_message(Purpose::Ordinary, Kind::Triage, instructions)?;
    let error = match validate(message.as_deref(), target, categories) {
        Ok(recommendation) => return Ok(recommendation),
        Err(error) => error,
    };
    let error = format!("{error:#}");
    let initial_trace = sessions
        .last_log_of(Kind::Triage)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "unavailable".into());
    let retained = format!(
        "Initial validation error: {error}\nTriage analysis retained in initial Session trace: {initial_trace}"
    );
    let correction = prompt::triage_correction(instructions, message.as_deref(), &error);
    let corrected = sessions
        .run_to_final_message(Purpose::Ordinary, Kind::TriageCorrection, &correction)
        .with_context(|| format!("Triage correction could not complete. {retained}"))?;
    validate(corrected.as_deref(), target, categories).with_context(|| {
        let correction_trace = sessions
            .last_log_of(Kind::TriageCorrection)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "unavailable".into());
        format!(
            "Triage recommendation invalid after one correction. {retained}\nCorrection Session trace: {correction_trace}"
        )
    })
}

fn validate(
    message: Option<&str>,
    target: Option<&IssueUrl>,
    categories: &[String],
) -> Result<Recommendation> {
    let mut recommendation = read_recommendation(message)?;
    // An invalid category is reported with any other error, so one
    // correction turn hears about both.
    let category = recommendation
        .issues
        .iter()
        .find_map(|issue| issue_category(categories, &recommendation, issue).err());
    let checked = resolve_recommendation(&mut recommendation).and_then(|()| match target {
        Some(target) => existing::check_recommendation(&mut recommendation, target),
        None => Ok(()),
    });
    match (checked, category) {
        (Ok(()), None) => Ok(recommendation),
        (Err(error), None) | (Ok(()), Some(error)) => Err(error),
        (Err(error), Some(category)) => bail!("{error:#}; {category:#}"),
    }
}
