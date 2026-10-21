//! Console settings and narrowly scoped, comment-preserving look writes.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub mod themes;

pub const THEME_FAMILIES: &[&str] = &["focus", "plain", "press", "signal", "garden", "factory"];
pub const SECTIONS: &[&str] = &[
    "needs-you",
    "ready-for-review",
    "landing",
    "working",
    "done",
    "resolved",
];

pub fn theme_names() -> Vec<String> {
    THEME_FAMILIES
        .iter()
        .flat_map(|family| ["auto", "light", "dark"].map(|mode| format!("{family}-{mode}")))
        .collect()
}

pub fn validate_theme(theme: &str) -> Result<()> {
    themes::Theme::export(theme)
        .map_err(|error| anyhow::anyhow!("unknown or invalid Console Theme {theme}: {error}"))?;
    Ok(())
}

pub fn validate_sections(sections: &[String]) -> Result<()> {
    ensure!(
        sections
            .iter()
            .all(|section| SECTIONS.contains(&section.as_str())),
        "unknown Console section; choose {}",
        SECTIONS.join(", ")
    );
    Ok(())
}

/// The Console settings Setup asks about; other Console settings stay put.
#[derive(Clone)]
pub struct ConsoleAccess {
    pub address: String,
    pub allowed_emails: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConsoleSettings {
    pub address: String,
    pub port: u16,
    pub allowed_emails: Vec<String>,
    pub other_consoles: Vec<String>,
    pub theme: String,
    pub hidden_sections: Vec<String>,
}

impl Default for ConsoleSettings {
    fn default() -> Self {
        Self {
            address: String::new(),
            port: 8711,
            allowed_emails: Vec::new(),
            other_consoles: Vec::new(),
            theme: "focus-auto".into(),
            hidden_sections: Vec::new(),
        }
    }
}

impl ConsoleSettings {
    /// The HTTPS site's DNS name, accepted with or without `https://`.
    pub fn hostname(&self) -> Result<&str> {
        let host = self
            .address
            .strip_prefix("https://")
            .unwrap_or(&self.address);
        let host = host.strip_suffix('/').unwrap_or(host);
        ensure!(
            !host.is_empty()
                && host.len() <= 253
                && host.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && !label.starts_with('-')
                        && !label.ends_with('-')
                        && label
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                }),
            "console.address must be an HTTPS DNS name, such as https://console.example.com, without a port or path"
        );
        Ok(host)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if !self.address.is_empty() {
            self.hostname()?;
        }
        for email in &self.allowed_emails {
            ensure!(
                email.split_once('@').is_some_and(|(local, domain)| {
                    !local.is_empty()
                        && !domain.contains('@')
                        && domain.contains('.')
                        && !domain.starts_with('.')
                        && !domain.ends_with('.')
                }) && !email.chars().any(char::is_whitespace),
                "console.allowed_emails: {email:?} is not an email address"
            );
        }
        validate_theme(&self.theme)?;
        validate_sections(&self.hidden_sections)?;
        for address in &self.other_consoles {
            let url = reqwest::Url::parse(address)?;
            ensure!(
                matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
                "console.other_consoles must contain http or https addresses"
            );
        }
        Ok(())
    }
}

/// The caller can change only the two look keys. Lock the read/edit/replace
/// across CLI processes and HTTP requests so independent look writes compose.
pub fn write_look(
    theme: Option<&str>,
    hide: &[String],
    show: &[String],
) -> Result<ConsoleSettings> {
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use toml_edit::{DocumentMut, Item, Table, Value};

    if let Some(theme) = theme {
        validate_theme(theme)?;
    }
    validate_sections(hide)?;
    validate_sections(show)?;
    let (home, path) = super::home_and_path()?;
    let root = path.parent().expect("User config directory");
    fs::create_dir_all(root)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("config.lock"))?;
    lock.lock()?;
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let mut settings = super::UserConfig::parse(&text, &path, &home)?.console;
    let mut document: DocumentMut = text.parse()?;
    if let Some(theme) = theme {
        settings.theme = theme.into();
    }
    settings
        .hidden_sections
        .retain(|section| !show.contains(section));
    for section in hide {
        if !settings.hidden_sections.contains(section) {
            settings.hidden_sections.push(section.clone());
        }
    }
    if !document.contains_key("console") {
        document["console"] = Item::Table(Table::new());
    }
    let mut set = |key: &str, value: Value| {
        let table = document["console"]
            .as_table_like_mut()
            .expect("validated console table");
        if table.contains_key(key) {
            super::set(&mut document, "console", key, value);
        } else {
            table.insert(key, Item::Value(value));
        }
    };
    if theme.is_some() {
        set("theme", settings.theme.clone().into());
    }
    if !hide.is_empty() || !show.is_empty() {
        set(
            "hidden_sections",
            settings
                .hidden_sections
                .iter()
                .collect::<toml_edit::Array>()
                .into(),
        );
    }
    let rendered = document.to_string();
    if path.exists() {
        super::replace(&path, &rendered)?;
    } else {
        let mut file = tempfile::NamedTempFile::new_in(root)?;
        file.write_all(rendered.as_bytes())?;
        file.persist(&path)?;
    }
    Ok(settings)
}
