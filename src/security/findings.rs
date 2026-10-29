//! Private recording and reproduction shared by Security runs and catalogue Steps.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{Recorded, RecordedFinding, audit, listed_severity, reproduction};
use crate::base_fix::Advice;
use crate::github::{DraftAdvisory, SecurityRecord, SecurityRecords};

/// Session ownership and the check repeated before a session and its Resume.
pub(crate) struct Session<'a> {
    pub logs: &'a crate::session::Logs,
    pub before_each: &'a dyn Fn(&Path) -> Result<()>,
}

/// The effects needed to record and reproduce findings, independent of dispatch.
pub(crate) trait Outside {
    fn security_records(&mut self) -> Result<SecurityRecords>;
    fn create_security_record(
        &mut self,
        records: &SecurityRecords,
        finding: &DraftAdvisory,
    ) -> Result<SecurityRecord>;
    fn reproduce(
        &mut self,
        record: &SecurityRecord,
        number: usize,
    ) -> (Result<reproduction::Reproduction>, Option<PathBuf>);
    fn update_security_record(
        &mut self,
        record: &SecurityRecord,
        reproduction: &reproduction::Reproduction,
    ) -> Result<()>;
    fn step(&mut self, line: String);
}

impl<T: crate::pass::Outside> Outside for T {
    fn security_records(&mut self) -> Result<SecurityRecords> {
        crate::pass::Outside::security_records(self)
    }

    fn create_security_record(
        &mut self,
        records: &SecurityRecords,
        finding: &DraftAdvisory,
    ) -> Result<SecurityRecord> {
        crate::pass::Outside::create_security_record(self, records, finding)
    }

    fn reproduce(
        &mut self,
        record: &SecurityRecord,
        number: usize,
    ) -> (Result<reproduction::Reproduction>, Option<PathBuf>) {
        crate::pass::Outside::reproduce(self, record, number)
    }

    fn update_security_record(
        &mut self,
        record: &SecurityRecord,
        reproduction: &reproduction::Reproduction,
    ) -> Result<()> {
        crate::pass::Outside::update_security_record(self, record, reproduction)
    }

    fn step(&mut self, line: String) {
        crate::pass::Outside::step(self, line);
    }
}

#[derive(Default)]
pub(crate) struct Findings {
    pub recorded: Recorded,
    pub findings: Vec<RecordedFinding>,
    pub waiting: Vec<String>,
    pub advice: Vec<Advice>,
    pub log: Option<PathBuf>,
    pub failed_reproduction: Option<usize>,
    pending: Vec<(SecurityRecord, String)>,
}

impl Findings {
    pub fn record(
        &mut self,
        outside: &mut impl Outside,
        audited: audit::Audited,
        offer_command: Option<&str>,
    ) -> Result<()> {
        if audited.findings.is_empty() {
            return Ok(());
        }
        let mut known = outside.security_records()?;
        let mut seen = std::collections::HashSet::new();
        for finding in audited.findings {
            let resolved = known.record_or_reuse(&finding, |records, draft| {
                outside.create_security_record(records, draft)
            })?;
            if resolved.created {
                self.recorded.created += 1;
                outside.step("recorded a Security finding privately".to_string());
            } else {
                self.recorded.existing += 1;
            }
            let record = resolved.record;
            if record.untriaged() {
                self.waiting.push(record.metadata().url.clone());
            }
            let mut metadata = record.metadata().clone();
            let url = metadata.url.clone();
            let first = seen.insert(finding.fingerprint);
            // A new candidate's write-up may quote an outcome no reproduction wrote.
            let kept = (!resolved.created)
                .then(|| record.completed_reproduction())
                .flatten();
            match (record.untriaged(), kept) {
                (false, _) => outside.step(format!(
                    "keeping the Day shift's grade for {}",
                    record.name()
                )),
                (true, Some(outcome)) => {
                    metadata.severity = listed_severity(&outcome, offer_command, &mut self.advice);
                    if first {
                        outside.step(format!(
                            "keeping the completed Security reproduction of {}: {outcome}",
                            record.name()
                        ));
                    }
                }
                (true, None) if first => self.pending.push((record, url)),
                (true, None) => {}
            }
            self.findings.push(metadata);
        }
        Ok(())
    }

    pub fn reproduce(
        &mut self,
        outside: &mut impl Outside,
        offer_command: Option<&str>,
    ) -> Result<()> {
        for (index, (record, url)) in self.pending.iter().enumerate() {
            let number = index + 1;
            outside.step(format!(
                "starting Security reproduction {number} of {}",
                record.name()
            ));
            self.failed_reproduction = Some(number);
            let (reproduced, session_log) = outside.reproduce(record, number);
            if session_log.is_some() {
                self.log = session_log;
            }
            let reproduced = reproduced?;
            outside.update_security_record(record, &reproduced)?;
            let severity = listed_severity(&reproduced.outcome, offer_command, &mut self.advice);
            for finding in self
                .findings
                .iter_mut()
                .filter(|finding| finding.url == *url)
            {
                finding.severity = severity.clone();
            }
            outside.step(format!(
                "Security reproduction {number}: {}",
                reproduced.outcome
            ));
        }
        self.pending.clear();
        self.failed_reproduction = None;
        Ok(())
    }
}
