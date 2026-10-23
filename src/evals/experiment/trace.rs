//! Trial ancestry lives with private Eval artifacts, including older Arms'
//! Session traces. No Trial gets a factory Command record of its own.
use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;

use crate::logs::trace::{Context, TRIAL_CONTEXT, id};
use crate::otlp_jsonl::{
    AnyValue, KeyValue, Logs, Reader, Resource, ResourceSpans, ScopeSpans, Span, Spans, Status,
    Writer,
};

pub(super) struct Attempt {
    directory: PathBuf,
    context: Context,
    spans: Vec<Span>,
}

impl Attempt {
    pub fn start(directory: &Path, name: &str, context: &Context) -> Result<Self> {
        let span_id = id(&[&context.trace.0, directory.as_os_str().as_encoded_bytes()]);
        let span = Span {
            trace_id: Some(context.trace),
            span_id: Some(span_id),
            parent_span_id: Some(context.stage),
            name: format!("Trial {name}"),
            kind: 1,
            start_time_unix_nano: now(),
            ..Default::default()
        };
        let mut context = context.clone();
        context.stage = span_id;
        super::store::write(&directory.join("trace-context.json"), &context)?;
        Ok(Self {
            directory: directory.into(),
            context,
            spans: vec![span],
        })
    }

    pub fn inherit(&self, command: &mut Command) -> Result<()> {
        command.env(TRIAL_CONTEXT, serde_json::to_string(&self.context)?);
        Ok(())
    }

    pub fn grade(
        &mut self,
        steps: impl FnOnce() -> Result<super::grading::Score>,
    ) -> Result<super::grading::Score> {
        let started = now();
        let result = steps();
        self.spans.push(Span {
            trace_id: Some(self.context.trace),
            span_id: Some(id(&[&self.context.stage.0, b"grader"])),
            parent_span_id: Some(self.context.stage),
            name: "Grader".into(),
            kind: 1,
            start_time_unix_nano: started,
            end_time_unix_nano: now(),
            status: Some(Status {
                code: if result.as_ref().is_ok_and(|score| score.passed) {
                    1
                } else {
                    2
                },
                ..Default::default()
            }),
            ..Default::default()
        });
        result
    }

    pub fn outcome(&mut self, outcome: crate::trial::Outcome) {
        self.spans[0].status = Some(Status {
            code: if outcome == crate::trial::Outcome::Succeeded {
                1
            } else {
                2
            },
            message: outcome.to_string(),
        });
    }

    fn finish(&mut self) -> Result<()> {
        self.spans[0].end_time_unix_nano = now();
        if self.spans[0].status.is_none() {
            self.spans[0].status = Some(Status {
                code: 2,
                message: "Trial interrupted".into(),
            });
        }
        // Older version-1 Arms have no context inheritance. Join their local
        // traces too, preserving span IDs and all within-Session ancestry.
        for session in sessions(&self.directory)? {
            rewrite::<Spans>(&session.with_extension("spans.jsonl"), |request| {
                for resource in &mut request.resource_spans {
                    self.resource(&mut resource.resource);
                    for span in resource
                        .scope_spans
                        .iter_mut()
                        .flat_map(|scope| &mut scope.spans)
                    {
                        span.trace_id = Some(self.context.trace);
                        if span.name.starts_with("invoke_agent ") {
                            span.parent_span_id = Some(self.context.stage);
                        }
                    }
                }
            })?;
            rewrite::<Logs>(&session.with_extension("logs.jsonl"), |request| {
                for resource in &mut request.resource_logs {
                    self.resource(&mut resource.resource);
                    for log in resource
                        .scope_logs
                        .iter_mut()
                        .flat_map(|scope| &mut scope.log_records)
                    {
                        log.trace_id = Some(self.context.trace);
                    }
                }
            })?;
        }
        let request = Spans {
            resource_spans: vec![ResourceSpans {
                scope_spans: vec![ScopeSpans {
                    spans: self.spans.clone(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let mut file = tempfile::NamedTempFile::new_in(&self.directory)?;
        Writer::new(file.as_file_mut()).write(&request)?;
        file.persist(self.directory.join("trace.spans.jsonl"))?;
        File::create(self.directory.join("trace.logs.jsonl"))?;
        Ok(())
    }

    fn resource(&self, resource: &mut Option<Resource>) {
        let resource = resource.get_or_insert_default();
        for (key, value) in [
            ("thirdshift.command.identity", &self.context.identity),
            ("thirdshift.repository", &self.context.repository),
        ] {
            resource.attributes.retain(|attr| attr.key != key);
            resource.attributes.push(KeyValue {
                key: key.into(),
                value: Some(AnyValue::String(value.clone())),
            });
        }
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            crate::progress::step(format!("warning: could not finish Trial trace: {error:#}"));
        }
    }
}

fn rewrite<R: crate::otlp_jsonl::Request>(
    path: &Path,
    mut update: impl FnMut(&mut R),
) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    for request in Reader::<_, R>::new(BufReader::new(File::open(path)?)) {
        let mut request = request?;
        update(&mut request);
        Writer::new(file.as_file_mut()).write(&request)?;
    }
    file.persist(path)?;
    Ok(())
}

pub(super) fn attempts(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut attempts = Vec::new();
    let trials = directory.join("trials");
    if trials.is_dir() {
        for trial in fs::read_dir(trials)? {
            for entry in fs::read_dir(trial?.path())? {
                let entry = entry?;
                if entry.file_type()?.is_dir()
                    && entry.file_name().to_string_lossy().starts_with("attempt-")
                {
                    attempts.push(entry.path());
                }
            }
        }
    }
    attempts.sort();
    Ok(attempts)
}

pub(super) fn sessions(attempt: &Path) -> Result<Vec<PathBuf>> {
    let directory = attempt.join("session");
    let mut sessions = Vec::new();
    if directory.is_dir() {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if let Some(stem) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".spans.jsonl"))
            {
                sessions.push(path.with_file_name(format!("{stem}.jsonl")));
            }
        }
    }
    sessions.sort();
    Ok(sessions)
}

impl super::Description {
    pub(crate) fn trace_sessions(&self, context: &Context) -> Result<Vec<PathBuf>> {
        let directory = crate::config::home()?
            .join(".thirdshift/evals/experiments")
            .join(&self.id);
        let mut paths = Vec::new();
        for attempt in attempts(&directory)? {
            let path = attempt.join("trace-context.json");
            if !path.exists() {
                continue;
            }
            let recorded: Context = serde_json::from_slice(&fs::read(path)?)?;
            if recorded.trace == context.trace {
                paths.push(attempt.join("trace.jsonl"));
                paths.extend(sessions(&attempt)?);
            }
        }
        Ok(paths)
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .try_into()
        .unwrap_or(u64::MAX)
}
