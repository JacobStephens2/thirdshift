//! The Factory skills, embedded at compile time and written out as the
//! `thirdshift` plugin (ADR-0001).

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use include_dir::{Dir, include_dir};
use tempfile::TempDir;

/// The Factory skills, as the plugin writes them out and the Prompts and skills
/// page shows them.
pub static SKILLS: Dir = include_dir!("$CARGO_MANIFEST_DIR/skills");

/// A temp directory holding the Factory skills and a generated plugin
/// manifest, removed when dropped.
pub struct Plugin {
    dir: TempDir,
}

impl Plugin {
    pub fn write() -> Result<Self> {
        let dir = tempfile::Builder::new()
            .prefix("thirdshift-plugin-")
            .tempdir()?;
        let skills = dir.path().join("skills");
        fs::create_dir_all(&skills)?;
        SKILLS
            .extract(&skills)
            .context("could not write the Factory skills")?;

        let manifest_dir = dir.path().join(".claude-plugin");
        fs::create_dir_all(&manifest_dir)?;
        let manifest = serde_json::json!({
            "name": "thirdshift",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Factory skills for thirdshift's headless sessions",
        });
        fs::write(
            manifest_dir.join("plugin.json"),
            serde_json::to_string_pretty(&manifest)?,
        )?;
        Ok(Plugin { dir })
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}
