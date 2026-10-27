//! Process files: compiled-in Built-in processes and local Custom processes.

use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::{Context, Result};
use include_dir::{Dir, include_dir};

use super::{parse, rule};

static BUILT_IN: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/processes");

const RESERVED: &[&str] = &[
    "check",
    "list",
    "show",
    "ticket-run",
    "spec-run",
    "architect-run",
    "pickup-run",
    "security-run",
];

pub(super) struct Catalog<'a> {
    built_in: &'a Dir<'static>,
    custom_dir: PathBuf,
}

impl Catalog<'_> {
    pub(super) fn installed() -> Result<Self> {
        Ok(Self {
            built_in: &BUILT_IN,
            custom_dir: crate::config::home()?.join(".thirdshift/processes"),
        })
    }

    pub(super) fn show(&self, name: &str) -> Result<String> {
        self.source(name).map(|(_, text)| text)
    }

    pub(super) fn source(&self, name: &str) -> Result<(PathBuf, String)> {
        if let Some(file) = self.built_in.get_file(format!("{name}.toml")) {
            let text = file
                .contents_utf8()
                .map(str::to_owned)
                .with_context(|| format!("built-in process {name} is not UTF-8"))?;
            return Ok((PathBuf::from("processes").join(file.path()), text));
        }
        self.custom(name)
    }

    pub(super) fn custom(&self, name: &str) -> Result<(PathBuf, String)> {
        let path = self.custom_dir.join(format!("{name}.toml"));
        if self.reserved(name) {
            return Err(rule(
                &path,
                1,
                "reserved name",
                format!("{name} is reserved"),
            ));
        }
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(rule(
                    &path,
                    1,
                    "missing file",
                    format!("unknown process {name}: the process file does not exist"),
                ));
            }
            Err(error) => {
                return Err(rule(
                    &path,
                    1,
                    "file",
                    format!("could not read the file: {error}"),
                ));
            }
        };
        Ok((path, text))
    }

    pub(super) fn list(&self) -> Result<String> {
        let mut lines = Vec::new();
        for file in self.built_in.files() {
            if file.path().extension().is_none_or(|ext| ext != "toml") {
                continue;
            }
            let name = file.path().file_stem().unwrap().to_str().unwrap();
            let path = PathBuf::from("built-in processes").join(file.path());
            let text = file
                .contents_utf8()
                .with_context(|| format!("built-in process {name} is not UTF-8"))?;
            let file = parse(&path, text)?;
            lines.push(format!("{name} (built-in): {}", file.description));
        }
        let entries = match fs::read_dir(&self.custom_dir) {
            Ok(entries) => Some(entries),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("could not list {}", self.custom_dir.display()));
            }
        };
        for entry in entries.into_iter().flatten() {
            let entry =
                entry.with_context(|| format!("could not list {}", self.custom_dir.display()))?;
            let file_name = entry.file_name();
            let Some(name) = file_name
                .to_str()
                .and_then(|name| name.strip_suffix(".toml"))
            else {
                continue;
            };
            if !crate::args::one_path_component(name) || self.reserved(name) {
                continue;
            }
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(file) = parse(&path, &text) else {
                continue;
            };
            lines.push(format!("{name}: {}", file.description));
        }
        lines.sort();
        Ok(lines.join("\n"))
    }

    fn reserved(&self, name: &str) -> bool {
        RESERVED.contains(&name) || self.built_in.get_file(format!("{name}.toml")).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static FIXTURES: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/tests/fixtures/processes");

    #[test]
    fn compiled_processes_are_listed_and_shown_without_a_custom_directory() {
        let home = tempfile::tempdir().unwrap();
        let catalog = Catalog {
            built_in: &FIXTURES,
            custom_dir: home.path().join("processes"),
        };
        assert_eq!(
            catalog.list().unwrap(),
            "inspection (built-in): Inspect a compiled Process"
        );
        assert_eq!(
            catalog.show("inspection").unwrap(),
            "version = 1\ndescription = \"Inspect a compiled Process\"\n\n[[steps]]\nname = \"inspect\"\ncommand = \"printf inspected\"\n"
        );
    }

    #[test]
    fn custom_processes_cannot_shadow_a_compiled_name() {
        let home = tempfile::tempdir().unwrap();
        let catalog = Catalog {
            built_in: &FIXTURES,
            custom_dir: home.path().to_path_buf(),
        };
        let custom = "version = 1\ndescription = \"Custom inspection\"\n[[steps]]\nname = \"inspect\"\ncommand = \"printf custom\"\n";
        fs::write(home.path().join("inspection.toml"), custom).unwrap();
        fs::write(home.path().join("notes.toml"), custom).unwrap();
        assert_eq!(catalog.show("notes").unwrap(), custom);
        assert!(
            catalog
                .show("inspection")
                .unwrap()
                .contains("printf inspected")
        );
        assert_eq!(
            catalog.list().unwrap(),
            "inspection (built-in): Inspect a compiled Process\nnotes: Custom inspection"
        );
        assert!(
            catalog
                .custom("inspection")
                .unwrap_err()
                .to_string()
                .contains("reserved name"),
            "a Custom process must not take a compiled Built-in process name"
        );
    }
}
