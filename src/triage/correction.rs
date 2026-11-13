//! Correct invalid session output once; GitHub checks and publication stay outside this loop.

use anyhow::{Context, Result};

use super::recommendation::{Captured, Recommendation};
use crate::prompt;
use crate::session::{Kind, Purpose, Sessions};

pub(super) fn recommendation<'a>(
    sessions: &Sessions<'_>,
    instructions: &str,
    captured: &Captured<'a>,
) -> Result<Recommendation<'a>> {
    // An execution failure is not invalid recommendation output.
    let message = sessions.run_to_final_message(Purpose::Ordinary, Kind::Triage, instructions)?;
    let error = match Recommendation::interpret(message.as_deref(), captured) {
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
    Recommendation::interpret(corrected.as_deref(), captured).with_context(|| {
        let correction_trace = sessions
            .last_log_of(Kind::TriageCorrection)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "unavailable".into());
        format!(
            "Triage recommendation invalid after one correction. {retained}\nCorrection Session trace: {correction_trace}"
        )
    })
}
