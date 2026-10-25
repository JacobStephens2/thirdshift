//! Trial report references and the shared, uncached read-and-blind operation.

use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(crate) struct Reports {
    pub standards: Option<PathBuf>,
    pub spec: Option<PathBuf>,
}

pub(crate) struct Blinded {
    pub standards: Option<String>,
    pub spec: Option<String>,
}

impl Reports {
    pub(crate) fn read_and_blind(&self) -> Result<Blinded> {
        let read = |path: &Option<PathBuf>| -> Result<Option<String>> {
            path.as_ref()
                .map(|path| fs::read_to_string(path).map(|text| blind(&text)))
                .transpose()
                .map_err(Into::into)
        };
        Ok(Blinded {
            standards: read(&self.standards)?,
            spec: read(&self.spec)?,
        })
    }
}

fn blind(text: &str) -> String {
    enum Table {
        Identities,
        KeyValue,
        Findings,
    }

    let fields = ["harness", "model", "effort", "arm"];
    let mut table = None;
    let mut blinded = String::new();
    for line in text.lines() {
        let normalized = line.to_ascii_lowercase().replace(['*', '_', '`'], "");
        let normalized = normalized
            .trim()
            .trim_start_matches(['#', '>', '-', ' '])
            .trim();
        if normalized.contains('|') {
            let cells: Vec<_> = normalized
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect();
            // Findings can name a Model or Harness as evidence. Metadata
            // tables instead identify Trials through columns or key/value rows.
            let kind = table.get_or_insert_with(|| {
                if cells.iter().any(|cell| fields.contains(cell)) {
                    Table::Identities
                } else if cells.len() == 2
                    && (cells[1] == "value"
                        || ["field", "key", "attribute", "property", "setting"].contains(&cells[0]))
                {
                    Table::KeyValue
                } else {
                    Table::Findings
                }
            });
            // A Findings block can still smuggle metadata: a two-column row
            // naming a field with a bare token value (| Harness | claude |)
            // identifies the Trial, while genuine finding evidence is a
            // sentence. Drop the former even when the block latched as
            // Findings.
            let metadata_row = cells.len() == 2
                && cells.first().is_some_and(|cell| fields.contains(cell))
                && cells[1].split_whitespace().count() <= 1;
            if matches!(kind, Table::Identities)
                || (matches!(kind, Table::KeyValue)
                    && cells.first().is_some_and(|cell| fields.contains(cell)))
                || metadata_row
            {
                continue;
            }
        } else {
            table = None;
        }
        let identifying = fields.iter().any(|key| {
            normalized == *key
                || normalized
                    .strip_prefix(key)
                    .is_some_and(|rest| rest.trim_start().starts_with([':', '=']))
        });
        if identifying
            || ["baseline", "candidate", "baseline arm", "candidate arm"].contains(&normalized)
            || ["built with", "generated with", "co-authored-by:"]
                .iter()
                .any(|prefix| normalized.starts_with(prefix))
        {
            continue;
        }
        blinded.push_str(line);
        blinded.push('\n');
    }
    blinded
}
