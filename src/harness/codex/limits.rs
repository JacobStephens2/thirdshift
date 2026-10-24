//! Live token-count reports in Codex's exact native session file. Exec's
//! JSON stdout does not currently expose these quota facts.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;

pub(super) struct Reader {
    root: Option<PathBuf>,
    since: DateTime<Utc>,
    file: Option<BufReader<File>>,
    partial: Vec<u8>,
    verified: bool,
    rejected: bool,
}

impl Default for Reader {
    fn default() -> Self {
        Self {
            root: std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex"))
                }),
            since: Utc::now(),
            file: None,
            partial: Vec::new(),
            verified: false,
            rejected: false,
        }
    }
}

impl Reader {
    pub(super) fn set_home(&mut self, home: &Path) {
        let root = home.join(".codex");
        if self.root.as_ref() == Some(&root) {
            return;
        }
        self.root = Some(root);
        self.file = None;
        self.partial.clear();
        self.verified = false;
        self.rejected = false;
    }

    pub(super) fn records(&mut self, id: Option<&str>, cwd: &str) -> Vec<String> {
        let Some(id) = id.filter(|id| !id.is_empty() && !id.contains(['/', '\\']) && *id != "..")
        else {
            return Vec::new();
        };
        if self.rejected {
            return Vec::new();
        }
        if self.file.is_none() {
            self.file = self
                .root
                .as_ref()
                .and_then(|root| super::patches::find(&root.join("sessions"), id, 3))
                .and_then(|path| File::open(path).ok())
                .map(BufReader::new);
        }
        let Some(file) = &mut self.file else {
            return Vec::new();
        };
        let mut records = Vec::new();
        loop {
            match file.read_until(b'\n', &mut self.partial) {
                Ok(0) | Err(_) => break,
                Ok(_) if !self.partial.ends_with(b"\n") => break,
                Ok(_) => (),
            }
            let raw = String::from_utf8_lossy(&self.partial).into_owned();
            self.partial.clear();
            let Ok(record) = serde_json::from_str::<Value>(&raw) else {
                continue;
            };
            if !self.verified {
                if record["type"] != "session_meta"
                    || record["payload"]["id"] != id
                    || record["payload"]["cwd"] != cwd
                {
                    self.rejected = true;
                    break;
                }
                self.verified = true;
            } else if record["type"] == "event_msg"
                && record["payload"]["type"] == "token_count"
                && record["timestamp"]
                    .as_str()
                    .and_then(|time| DateTime::parse_from_rfc3339(time).ok())
                    .is_some_and(|time| time >= self.since)
            {
                records.push(raw);
            }
        }
        records
    }
}
