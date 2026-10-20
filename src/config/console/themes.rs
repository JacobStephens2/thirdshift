//! Portable Theme documents, validated before either CLI or page can save them.
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

const CSS: &str = include_str!("../../console/assets/themes.css");
const CONSOLE_CSS: &str = include_str!("../../console/assets/console.css");
const STACKS: &[&str] = &[
    "sans-serif",
    "serif",
    "monospace",
    "system-ui",
    "ui-sans-serif",
    "ui-serif",
    "ui-monospace",
];

#[derive(Debug, Serialize)]
pub struct Theme {
    name: String,
    base: String,
    tokens: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeDocument {
    name: String,
    #[serde(default = "default_base")]
    base: String,
    #[serde(default)]
    tokens: BTreeMap<String, String>,
}

fn built_in_font(family: &str) -> bool {
    let declaration = format!("font-family: \"{family}\";");
    [CSS, CONSOLE_CSS].iter().any(|css| {
        css.split("@font-face").skip(1).any(|face| {
            face.split_once('}')
                .is_some_and(|(block, _)| block.contains(&declaration))
        })
    })
}

fn default_base() -> String {
    "focus-auto".into()
}

pub fn directory() -> Result<PathBuf> {
    Ok(crate::config::home()?.join(".thirdshift/console/themes"))
}

fn identifier(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn defaults(base: &str) -> Result<BTreeMap<String, String>> {
    ensure!(
        super::theme_names().iter().any(|name| name == base),
        "unknown base Theme: {base}"
    );
    let family = base.rsplit_once('-').expect("built-in mode").0;
    let selector = format!(":root[data-theme^=\"{family}-\"] {{");
    let block = CSS
        .split_once(&selector)
        .expect("built-in tokens")
        .1
        .split_once('}')
        .expect("token block")
        .0;
    let mut tokens = BTreeMap::from([("activity-motion".into(), "none".into())]);
    for declaration in block.split(';').filter_map(|s| s.trim().strip_prefix("--")) {
        let (name, value) = declaration.split_once(':').expect("token declaration");
        tokens.insert(name.into(), value.trim().into());
    }
    Ok(tokens)
}

fn hex(value: &str) -> bool {
    value.strip_prefix('#').is_some_and(|digits| {
        matches!(digits.len(), 3 | 4 | 6 | 8) && digits.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

fn color(value: &str) -> bool {
    hex(value)
        || matches!(value, "transparent" | "currentColor")
        || value
            .strip_prefix("light-dark(")
            .and_then(|s| s.strip_suffix(')'))
            .is_some_and(|pair| {
                pair.split_once(',')
                    .is_some_and(|(a, b)| hex(a.trim()) && hex(b.trim()))
            })
}

fn dimension(value: &str, unit: &str, min: f64, max: f64) -> bool {
    value
        .strip_suffix(unit)
        .and_then(|s| s.parse::<f64>().ok())
        .is_some_and(|n| n >= min && n <= max)
}

fn font_parts(value: &str) -> Result<Vec<&str>> {
    let parts: Vec<_> = value
        .split(',')
        .map(|s| s.trim().trim_matches('"'))
        .collect();
    ensure!(
        parts.iter().all(|part| !part.is_empty()
            && part.len() <= 100
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b' ' | b'.' | b'-' | b'_'))),
        "invalid font family or stack"
    );
    Ok(parts)
}

impl Theme {
    pub fn parse(text: &str) -> Result<Self> {
        ensure!(text.len() <= 16 * 1024, "Theme JSON exceeds 16 KiB");
        let document: ThemeDocument = serde_json::from_str(text).context("invalid Theme JSON")?;
        let mut theme = Self {
            name: document.name,
            base: document.base,
            tokens: document.tokens,
        };
        ensure!(
            identifier(&theme.name) && !matches!(theme.name.as_str(), "list" | "import" | "export"),
            "invalid or reserved Theme name: {}",
            theme.name
        );
        let mut tokens = defaults(&theme.base)?;
        for (key, value) in &theme.tokens {
            ensure!(tokens.contains_key(key), "unknown Theme token: {key}");
            // Existing built-in values remain valid, including their effects and
            // activity-only motion. Arbitrary CSS never enters the stylesheet.
            let builtin = super::THEME_FAMILIES.iter().any(|family| {
                defaults(&format!("{family}-auto"))
                    .expect("built-in")
                    .get(key)
                    == Some(value)
            });
            let valid = builtin
                || match key.as_str() {
                    "font-head" | "font-body" | "font-mono" => font_parts(value).is_ok(),
                    "body-size" => dimension(value, "px", 10.0, 32.0),
                    "radius" => dimension(value, "px", 0.0, 32.0),
                    "head-weight" => value.parse::<u16>().is_ok_and(|n| (100..=900).contains(&n)),
                    "head-case" => matches!(
                        value.as_str(),
                        "none" | "uppercase" | "lowercase" | "capitalize"
                    ),
                    "activity-speed" => dimension(value, "s", 0.1, 60.0),
                    "activity-decoration" | "activity-motion" | "panel-shadow" | "title-shadow" => {
                        false
                    }
                    _ => color(value),
                };
            ensure!(valid, "invalid Theme token {key}: {value}");
            tokens.insert(key.clone(), value.clone());
        }
        theme.tokens = tokens;
        if super::theme_names().contains(&theme.name) {
            ensure!(
                theme.base == theme.name && theme.tokens == defaults(&theme.name)?,
                "built-in Theme {} cannot be replaced; choose a new name",
                theme.name
            );
        }
        Ok(theme)
    }

    pub fn export(name: &str) -> Result<Self> {
        if super::theme_names().iter().any(|builtin| builtin == name) {
            return Ok(Self {
                name: name.into(),
                base: name.into(),
                tokens: defaults(name)?,
            });
        }
        ensure!(identifier(name), "invalid Theme name: {name}");
        let theme = Self::parse(&fs::read_to_string(
            directory()?.join(format!("{name}.json")),
        )?)?;
        ensure!(
            theme.name == name,
            "Theme name does not match its filename: {name}"
        );
        Ok(theme)
    }

    pub fn save(&self) -> Result<()> {
        // An unchanged built-in already exists on every installation. Accept
        // its export without creating a file that could shadow it.
        if super::theme_names().contains(&self.name) {
            return Ok(());
        }
        let root = directory()?;
        fs::create_dir_all(&root)?;
        let mut file = tempfile::NamedTempFile::new_in(&root)?;
        file.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
        file.persist(root.join(format!("{}.json", self.name)))?;
        Ok(())
    }

    pub fn stylesheet(&self) -> String {
        let mode = self.base.rsplit_once('-').expect("validated base").1;
        let scheme = if mode == "auto" { "light dark" } else { mode };
        let mut faces = String::new();
        let mut declarations = format!(
            ":root[data-theme=\"{}\"] {{ color-scheme: {scheme};",
            self.name
        );
        for (key, value) in &self.tokens {
            let value = if key.starts_with("font-") {
                let mut families = Vec::new();
                for family in font_parts(value).expect("validated font") {
                    if built_in_font(family) {
                        families.push(format!("\"{family}\""));
                    } else if STACKS.contains(&family) {
                        families.push(family.into());
                    } else if font_type(family).is_some() && font(family).is_ok() {
                        faces.push_str(&format!("@font-face {{font-family: \"user-{family}\"; src: url(\"/assets/user-fonts/{family}\"); font-display: swap;}}\n"));
                        families.push(format!("\"user-{family}\""));
                    }
                }
                families.push(
                    if key == "font-mono" {
                        "monospace"
                    } else {
                        "sans-serif"
                    }
                    .into(),
                );
                families.join(", ")
            } else {
                value.clone()
            };
            declarations.push_str(&format!("--{key}: {value};"));
        }
        format!("{faces}{declarations}}}\n")
    }
}

pub fn names() -> Result<Vec<String>> {
    let mut names = super::theme_names();
    let entries = match fs::read_dir(directory()?) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(names),
        Err(error) => return Err(error.into()),
    };
    let mut custom = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_some_and(|s| s == "json")
            && let Some(name) = path.file_stem().and_then(|s| s.to_str())
            && Theme::export(name).is_ok()
        {
            custom.push(name.to_owned());
        }
    }
    custom.sort();
    names.extend(custom);
    Ok(names)
}

pub fn font_type(name: &str) -> Option<&'static str> {
    if name.len() > 100
        || name.starts_with('.')
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        return None;
    }
    match name.rsplit_once('.')?.1 {
        "woff2" => Some("font/woff2"),
        "woff" => Some("font/woff"),
        "otf" => Some("font/otf"),
        "ttf" => Some("font/ttf"),
        _ => None,
    }
}

pub fn font(name: &str) -> Result<Vec<u8>> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::OpenOptionsExt;
    ensure!(font_type(name).is_some(), "unsupported font filename");
    // Pin the folder and refuse symlinks at open time, including replacement
    // races. No pathname resolution can escape through a link or a slash.
    let folder = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(directory()?.join("fonts"))?;
    let name = std::ffi::CString::new(name)?;
    let fd = unsafe {
        libc::openat(
            folder.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut file = unsafe { fs::File::from_raw_fd(fd) };
    ensure!(file.metadata()?.is_file(), "font is not a regular file");
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}
