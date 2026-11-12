//! Which Commands and buckets a metrics trend covers: its filters and window.

use chrono::{DateTime, Datelike, Days, NaiveDate, Utc};
use serde::Serialize;

use super::super::reading::Command;
use crate::args::{MetricsArgs, MetricsGrain, MetricsWindow};

/// The buckets a trend shows. A bounded window also has the preceding window
/// of equal length to compare against.
#[derive(Clone, Copy, Serialize)]
pub(crate) struct Window {
    pub(crate) preset: Option<&'static str>,
    pub(crate) grain: &'static str,
    pub(crate) from: Option<NaiveDate>,
    pub(crate) through: Option<NaiveDate>,
    pub(crate) prior: Option<Span>,
    #[serde(skip)]
    unit: MetricsGrain,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct Span {
    pub(crate) from: NaiveDate,
    pub(crate) through: NaiveDate,
}

impl Span {
    pub(super) fn contains(self, bucket: NaiveDate) -> bool {
        self.from <= bucket && bucket <= self.through
    }
}

impl Window {
    /// Presets end with the bucket holding `today`, a UTC date like the buckets.
    pub(super) fn of(asked: &MetricsArgs, today: NaiveDate) -> Self {
        let monday = start(MetricsGrain::Week, today);
        let (unit, from, through) = match asked.window {
            Some(MetricsWindow::Days7) => {
                (MetricsGrain::Day, Some(today - Days::new(6)), Some(today))
            }
            Some(MetricsWindow::Days30) => {
                (MetricsGrain::Day, Some(today - Days::new(29)), Some(today))
            }
            Some(MetricsWindow::Weeks12) => (
                MetricsGrain::Week,
                Some(monday - Days::new(7 * 11)),
                Some(monday),
            ),
            Some(MetricsWindow::All) | None => {
                (asked.grain.unwrap_or_default(), asked.from, asked.through)
            }
        };
        let window = Self {
            preset: asked.window.map(MetricsWindow::name),
            grain: unit.name(),
            from,
            through,
            prior: None,
            unit,
        };
        Self {
            prior: window.current().map(|current| window.preceding(current)),
            ..window
        }
    }

    pub(crate) fn current(&self) -> Option<Span> {
        Some(Span {
            from: self.from?,
            through: self.through?,
        })
    }

    fn step(&self) -> u64 {
        match self.unit {
            MetricsGrain::Day => 1,
            MetricsGrain::Week => 7,
        }
    }

    /// The window of the same number of buckets that ends just before this one.
    fn preceding(&self, current: Span) -> Span {
        let length = (current.through - current.from).num_days().unsigned_abs() + self.step();
        Span {
            from: current.from - Days::new(length),
            through: current.from - Days::new(self.step()),
        }
    }

    /// The bucket holding a Command's start, by its UTC date.
    pub(super) fn bucket(&self, command: &Command) -> Option<NaiveDate> {
        let started = DateTime::parse_from_rfc3339(&command.record.started_at).ok()?;
        Some(self.bucket_on(started.with_timezone(&Utc).date_naive()))
    }

    /// The bucket holding a UTC date, such as a merge or a Scoring week's start.
    pub(super) fn bucket_on(&self, date: NaiveDate) -> NaiveDate {
        start(self.unit, date)
    }

    pub(super) fn shows(&self, bucket: NaiveDate) -> bool {
        self.from.is_none_or(|from| from <= bucket)
            && self.through.is_none_or(|through| bucket <= through)
    }

    pub(crate) fn weekly(&self) -> bool {
        self.unit == MetricsGrain::Week
    }
}

fn start(unit: MetricsGrain, date: NaiveDate) -> NaiveDate {
    match unit {
        MetricsGrain::Day => date,
        MetricsGrain::Week => date - Days::new(date.weekday().num_days_from_monday().into()),
    }
}

/// The filters asked for, applied to each Command's own record.
#[derive(Serialize)]
pub(crate) struct Filters {
    pub(crate) repo: Vec<String>,
    pub(crate) kind: Vec<String>,
    pub(crate) harness: Vec<String>,
    pub(crate) model: Vec<String>,
    pub(crate) account: Vec<String>,
}

impl Filters {
    pub(super) fn of(asked: &MetricsArgs) -> Self {
        Self {
            repo: asked.repositories.clone(),
            kind: asked.kinds.clone(),
            harness: asked.harnesses.clone(),
            model: asked.models.clone(),
            account: asked.accounts.clone(),
        }
    }

    /// `account` is the Command's OS account, the serving one included.
    pub(super) fn admit(&self, command: &Command, account: Option<&str>) -> bool {
        let record = &command.record;
        let any = |values: &[String], value: Option<&str>| {
            values.is_empty() || value.is_some_and(|value| values.iter().any(|v| v == value))
        };
        any(&self.repo, Some(&record.repository))
            && (any(&self.kind, Some(&record.kind))
                || any(&self.kind, Some(crate::logs::display_name(&record.kind))))
            && any(&self.harness, Some(&record.harness))
            && any(&self.model, record.model.as_deref())
            && any(&self.account, account)
    }

    /// A Scoring sample's pull request, by its repository and Built-with
    /// Harness and Model. Kind and account name Commands, not samples, so
    /// they leave Scores unfiltered.
    pub(super) fn admit_scored(&self, repository: &str, harness: &str, model: &str) -> bool {
        let any = |values: &[String], value: &str| {
            values.is_empty() || values.iter().any(|v| v.eq_ignore_ascii_case(value))
        };
        any(&self.repo, repository) && any(&self.harness, harness) && any(&self.model, model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(window: Option<MetricsWindow>) -> MetricsArgs {
        MetricsArgs {
            window,
            ..MetricsArgs::default()
        }
    }

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    fn span(from: &str, through: &str) -> Option<Span> {
        Some(Span {
            from: date(from),
            through: date(through),
        })
    }

    #[test]
    fn presets_end_today_and_compare_with_the_equal_window_before() {
        // A Wednesday.
        let today = date("2026-11-11");
        for (window, grain, current, prior) in [
            (
                MetricsWindow::Days7,
                "day",
                span("2026-11-05", "2026-11-11"),
                span("2026-10-29", "2026-11-04"),
            ),
            (
                MetricsWindow::Days30,
                "day",
                span("2026-10-13", "2026-11-11"),
                span("2026-09-13", "2026-10-12"),
            ),
            (
                MetricsWindow::Weeks12,
                "week",
                span("2026-08-24", "2026-11-09"),
                span("2026-06-01", "2026-08-17"),
            ),
        ] {
            let resolved = Window::of(&asked(Some(window)), today);
            assert_eq!(resolved.grain, grain, "{window:?}");
            assert!(resolved.current() == current, "{window:?}");
            assert!(resolved.prior == prior, "{window:?}");
        }
        let all = Window::of(&asked(Some(MetricsWindow::All)), today);
        assert_eq!((all.preset, all.grain), (Some("all"), "week"));
        assert!(all.current().is_none() && all.prior.is_none());
    }

    #[test]
    fn explicit_bounds_compare_only_when_both_are_given() {
        let today = date("2026-11-11");
        let bounded = Window::of(
            &MetricsArgs {
                grain: Some(MetricsGrain::Day),
                from: Some(date("2026-11-01")),
                through: Some(date("2026-11-03")),
                ..MetricsArgs::default()
            },
            today,
        );
        assert!(bounded.prior == span("2026-10-29", "2026-10-31"));
        let open = Window::of(
            &MetricsArgs {
                from: Some(date("2026-11-02")),
                ..MetricsArgs::default()
            },
            today,
        );
        assert_eq!(open.grain, "week");
        assert!(open.prior.is_none());
        assert!(open.shows(date("2026-11-09")) && !open.shows(date("2026-10-26")));
    }
}
