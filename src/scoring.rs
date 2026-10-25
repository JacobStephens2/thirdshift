//! The weekly Scoring sample. A single machine-wide lock protects window
//! advancement; an atomic private sample file is the successful checkpoint.
use std::fs::{self, File, TryLockError};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, TimeDelta, Utc};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};

use crate::config::{self, UserConfig};
use crate::github::GitHub;
use crate::issue::Repo;
use crate::{interrupt, logs};

mod calibration;
mod grader;
pub(crate) mod origins;
mod sample;
mod store;
pub(crate) use sample::PullRequest;
pub(crate) mod packet;
pub(crate) use grader::{Criterion, Score};

pub fn lock() -> Result<Option<File>> {
    let dir = config::home()?.join(".thirdshift");
    fs::create_dir_all(&dir)?;
    let file = File::create(dir.join("scoring.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(error.into()),
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Week {
    pub week: String,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    seed: u64,
    pub sample: Vec<PullRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    packets: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bug_origins: Option<Vec<origins::Proposal>>,
    #[serde(default)]
    initial_calibration: bool,
}

pub fn sample(config: &UserConfig, end: DateTime<Utc>) -> Result<usize> {
    let repo = Repo {
        owner: "JacobStephens2".into(),
        name: "thirdshift".into(),
    };
    logs::started(logs::Work::ScoringRun(&repo), &config.scoring.grader());
    if config.scoring.owners.is_empty() {
        bail!("configure scoring.owners in the User config before starting a Scoring run");
    }
    let store = store::Store::open()?;
    let mut previous = store.load()?;
    let mut ordinal = 0;
    for retained in &mut previous {
        if !grade(config, retained, &mut ordinal)? {
            return Ok(0);
        }
    }
    let start = previous
        .last()
        .map(|sample| sample.week.window_end)
        .unwrap_or(end - TimeDelta::days(7));
    if start > end {
        bail!("the previous Scoring window ends in the future");
    }
    let candidates = GitHub::new().scoring_pull_requests(&config.scoring.owners, start, end)?;
    let seed = OsRng.next_u64();
    let mut sample = sample::draw(candidates, config.scoring.sample_size.get(), seed);
    let initial_calibration = previous.is_empty();
    if initial_calibration {
        let history = GitHub::new().scoring_pull_requests(
            &config.scoring.owners,
            DateTime::UNIX_EPOCH,
            end,
        )?;
        for mut pr in sample::draw(history, 20, seed) {
            if let Some(existing) = sample.iter_mut().find(|existing| existing.url == pr.url) {
                existing.for_you = true;
            } else {
                pr.for_you = true;
                sample.push(pr);
            }
        }
    } else {
        let week = end.format("%G-W%V").to_string();
        let marked = previous
            .iter()
            .map(|sample| &sample.week)
            .filter(|saved| saved.week == week && !saved.initial_calibration)
            .flat_map(|saved| &saved.sample)
            .filter(|pr| pr.for_you)
            .count();
        for pr in sample.iter_mut().take(2usize.saturating_sub(marked)) {
            pr.for_you = true;
        }
    }
    let mut week = Week {
        week: end.format("%G-W%V").to_string(),
        window_start: start,
        window_end: end,
        seed,
        sample,
        packets: None,
        bug_origins: None,
        initial_calibration,
    };
    let packets = store.stage_packets()?;
    for (index, pr) in week.sample.iter().enumerate() {
        packet::write(&GitHub::new(), pr, &packets.path().join(index.to_string()))
            .with_context(|| format!("can't build Scoring packet for {}", pr.url))?;
    }
    let origin_start = previous
        .iter()
        .rev()
        .map(|sample| &sample.week)
        .find(|week| week.bug_origins.is_some())
        .map(|week| week.window_end);
    week.bug_origins = Some(origins::propose(
        config,
        origin_start,
        end,
        &packets.path().join("origins"),
    )?);
    for pr in &mut week.sample {
        pr.command_record = logs::record_for_pull_request(&config.logs_dir, &pr.url)?;
    }
    let mut retained = packets.publish(week)?;
    grade(config, &mut retained, &mut ordinal)?;
    Ok(retained.week.sample.len())
}

fn grade(config: &UserConfig, retained: &mut store::Retained, ordinal: &mut usize) -> Result<bool> {
    for index in 0..retained.week.sample.len() {
        if retained.week.sample[index].assessment.is_some() {
            continue;
        }
        interrupt::check()?;
        let packet = retained.packet(index)?;
        *ordinal += 1;
        let assessment = match grader::run(
            &config.scoring,
            &retained.week.sample[index],
            &packet,
            *ordinal,
        ) {
            Ok(assessment) => assessment,
            Err(error) if error.is::<crate::session::limits::Reached>() => {
                crate::progress::step(error);
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        let week = &mut retained.week;
        week.sample[index].assessment = Some(assessment);
        if week.sample[index].command_record.is_none() {
            week.sample[index].command_record =
                logs::record_for_pull_request(&config.logs_dir, &week.sample[index].url)?;
        }
        retained.commit()?;
    }
    Ok(true)
}

pub fn show(pull_request: Option<&str>, mine: bool, json: bool) -> Result<String> {
    if mine {
        interrupt::install()?;
        write_mine(
            pull_request.context("human scoring requires a pull request")?,
            calibration::enter,
        )?;
    }
    let view = view(pull_request)?;
    display(&view.weeks, &view.calibration, json)
}

pub(crate) struct View {
    pub weeks: Vec<Week>,
    pub calibration: calibration::Calibration,
}

pub(crate) fn view(pull_request: Option<&str>) -> Result<View> {
    let mut weeks: Vec<_> = store::Store::open()?
        .load()?
        .into_iter()
        .map(store::Retained::into_week)
        .collect();
    // Calibration always uses the full history, even for a single PR's view.
    let calibration = calibration::measure(&weeks);
    if let Some(target) = pull_request {
        let url = resolve(&weeks, target)?;
        for week in &mut weeks {
            week.sample.retain(|pr| pr.url == url);
        }
        weeks.retain(|week| !week.sample.is_empty());
    }
    // CLI and Console reads share the same blind view. Stored assessments
    // remain available for calibration and are revealed after human entry.
    for pr in weeks.iter_mut().flat_map(|week| &mut week.sample) {
        if pr.mine.is_none() {
            pr.assessment = None;
        }
    }
    Ok(View { weeks, calibration })
}

fn resolve<'a>(weeks: impl IntoIterator<Item = &'a Week>, target: &str) -> Result<String> {
    let urls: std::collections::BTreeSet<_> = weeks
        .into_iter()
        .flat_map(|week| &week.sample)
        .filter(|pr| {
            pr.url.trim_end_matches('/') == target.trim_end_matches('/')
                || target.parse::<u64>().ok() == Some(pr.number)
        })
        .map(|pr| pr.url.clone())
        .collect();
    if urls.len() != 1 {
        bail!("pull request is unknown or ambiguous in the Scoring samples: {target}");
    }
    Ok(urls.into_iter().next().unwrap())
}

pub(crate) fn save_mine(target: &str, scores: Vec<Score>) -> Result<()> {
    write_mine(target, || calibration::HumanScores::new(scores))
}

fn write_mine(
    target: &str,
    enter: impl FnOnce() -> Result<calibration::HumanScores>,
) -> Result<()> {
    let _lock = lock()?.context("another Scoring run or human scoring session is running")?;
    let mut weeks = store::Store::open()?.load()?;
    let url = resolve(weeks.iter().map(|sample| &sample.week), target)?;
    let scores = enter()?;
    for retained in &mut weeks {
        if let Some(pr) = retained.week.sample.iter_mut().find(|pr| pr.url == url) {
            pr.mine = Some(scores);
            retained.commit()?;
            break;
        }
    }
    Ok(())
}

fn display(weeks: &[Week], calibration: &calibration::Calibration, json: bool) -> Result<String> {
    if json {
        let mut view = serde_json::to_value(weeks)?;
        for week in view.as_array_mut().unwrap() {
            for pr in week["sample"].as_array_mut().unwrap() {
                if let Some(scores) = pr
                    .get_mut("assessment")
                    .and_then(|assessment| assessment.get_mut("scores"))
                    .and_then(|scores| scores.as_array_mut())
                {
                    for score in scores {
                        let criterion = serde_json::from_value(score["criterion"].clone())?;
                        score["calibration"] = serde_json::to_value(&calibration[&criterion])?;
                    }
                }
            }
        }
        return Ok(serde_json::to_string_pretty(&view)?);
    }
    let mut text = String::new();
    for week in weeks {
        text.push_str(&format!(
            "{}  {} → {}  seed {}\n",
            week.week, week.window_start, week.window_end, week.seed
        ));
        for pr in &week.sample {
            text.push_str(&format!("  {}  {} · {}\n", pr.url, pr.harness, pr.model));
            if pr.for_you {
                text.push_str("    For you to score\n");
            }
            if let Some(mine) = &pr.mine {
                for score in &mine.scores {
                    text.push_str(&format!(
                        "    Yours {:?}: {} — {}\n",
                        score.criterion,
                        if score.passed { "pass" } else { "fail" },
                        score.reason
                    ));
                }
            }
            if pr.mine.is_none() {
                text.push_str("    Model Scores hidden until all five of yours are saved.\n");
            } else if let Some(assessment) = &pr.assessment {
                text.push_str(&format!(
                    "    Grader: {} · {}\n",
                    assessment.grader.harness, assessment.grader.model
                ));
                for score in &assessment.scores {
                    let state = &calibration[&score.criterion];
                    text.push_str(&format!(
                        "    {:?}: {} — {} [{}: {}/{}]\n",
                        score.criterion,
                        if score.passed { "pass" } else { "fail" },
                        score.reason,
                        if state.calibrated {
                            "calibrated"
                        } else {
                            "uncalibrated"
                        },
                        state.matches,
                        state.compared
                    ));
                }
            } else {
                text.push_str("    unscored\n");
            }
        }
    }
    Ok(text)
}
