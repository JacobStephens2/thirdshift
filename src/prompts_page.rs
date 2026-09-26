//! The Prompts and skills page's generated fragment: the `claude` command
//! line, every prompt a Run sends, and every Factory skill, rendered from the
//! same functions and embedded files the binary uses, so the page can't drift
//! from what it does. Test-only: the golden-file test below keeps the
//! checked-in fragment in `site/prompts/index.html` up to date.

use std::ffi::OsStr;
use std::fmt::Write;

use include_dir::{Dir, File};

use crate::github::{Check, CheckState};
use crate::issue::IssueUrl;
use crate::plugin::SKILLS;
use crate::prompt;
use crate::session::claude_args;

/// A press unit on the home page, which each prompt and skill links back to.
struct Unit {
    anchor: &'static str,
    name: &'static str,
}

const IMPLEMENT: Unit = Unit {
    anchor: "unit-2",
    name: "2 · M · Implement",
};
const REVIEW: Unit = Unit {
    anchor: "unit-3",
    name: "3 · Y · Review",
};
const FINISH: Unit = Unit {
    anchor: "unit-4",
    name: "4 · K · Finish",
};

/// The placeholder values the prompts are rendered with, highlighted on the
/// page. The issue number can't be text, so a sentinel stands in for it.
const ISSUE_URL: &str = "<Issue URL>";
const ISSUE_NUMBER: u64 = u64::MAX;
const NUMBER: &str = "<n>";
const BASE: &str = "<base>";
const BRANCH: &str = "<branch>";
const PR_URL: &str = "<pull request URL>";
const CHECK: &str = "<failing check>";
const CHECK_URL: &str = "<check URL>";
const BACKGROUND_WORK: &str = "<background work>";
const PLUGIN_DIR: &str = "<plugin dir>";
const SESSION_ID: &str = "<session id>";
const PROMPT: &str = "<prompt>";
const PLACEHOLDERS: [&str; 11] = [
    ISSUE_URL,
    NUMBER,
    BASE,
    BRANCH,
    PR_URL,
    CHECK,
    CHECK_URL,
    BACKGROUND_WORK,
    PLUGIN_DIR,
    SESSION_ID,
    PROMPT,
];

/// A prompt as the page shows it: when a Run sends it, from which units, and
/// its text with placeholders.
struct Prompt {
    id: &'static str,
    title: &'static str,
    when: &'static str,
    units: &'static [Unit],
    text: String,
}

fn prompts() -> Vec<Prompt> {
    let issue = IssueUrl {
        url: ISSUE_URL.to_string(),
        owner: "<owner>".to_string(),
        repo: "<repo>".to_string(),
        number: ISSUE_NUMBER,
    };
    let failed = [Check {
        name: CHECK.to_string(),
        state: CheckState::Failed,
        url: Some(CHECK_URL.to_string()),
    }];
    vec![
        Prompt {
            id: "prompt-fresh",
            title: "Fresh",
            when: "Starts the implement session when the Run starts a new Issue branch. The same session goes on to review its work and open the pull request.",
            units: &[IMPLEMENT],
            text: prompt::fresh(&issue, BASE, BRANCH),
        },
        Prompt {
            id: "prompt-continuation",
            title: "Continuation, with no pull request",
            when: "Starts the implement session when the Run is a Continuation of an Issue branch that has no pull request.",
            units: &[IMPLEMENT],
            text: prompt::continuation(&issue, BASE, BRANCH, None),
        },
        Prompt {
            id: "prompt-continuation-pr",
            title: "Continuation, with an open pull request",
            when: "Starts the implement session when the Run is a Continuation of an Issue branch whose pull request is open.",
            units: &[IMPLEMENT],
            text: prompt::continuation(&issue, BASE, BRANCH, Some(PR_URL)),
        },
        Prompt {
            id: "prompt-conflict-repair",
            title: "Conflict Repair",
            when: "Starts a Repair session when merging the Base branch into the Issue branch leaves conflicts.",
            units: &[FINISH],
            text: prompt::conflict_repair(&issue, BASE, BRANCH, PR_URL),
        },
        Prompt {
            id: "prompt-ci-fix-repair",
            title: "CI-fix Repair",
            when: "Starts a Repair session when CI fails on the pull request's head commit, listing each failed check.",
            units: &[FINISH],
            text: prompt::ci_fix_repair(&issue, BASE, BRANCH, PR_URL, &failed),
        },
        Prompt {
            id: "prompt-resume",
            title: "Resume",
            when: "Continues a session, once, when it ended its turn while waiting on background work, which was killed with it. Any session can get one, and it isn't a Repair.",
            units: &[IMPLEMENT, FINISH],
            text: prompt::resume(&[BACKGROUND_WORK]),
        },
    ]
}

/// The unit whose session uses each Factory skill, and when, if not always.
fn skill_unit(skill: &str) -> (Unit, Option<&'static str>) {
    match skill {
        "implement" | "tdd" => (IMPLEMENT, None),
        "code-review" | "pr" => (REVIEW, None),
        "resolving-merge-conflicts" => (FINISH, Some("in a conflict Repair")),
        _ => panic!("the Factory skill {skill} has no unit: add it to skill_unit"),
    }
}

/// The fragment: the command line, the prompts, the skills, and the skills'
/// licence.
pub fn render() -> String {
    let mut html = String::new();
    command_line(&mut html);
    prompt_section(&mut html);
    skills_section(&mut html);
    licence_section(&mut html);
    html
}

fn command_line(html: &mut String) {
    let line = |resume| {
        let args = claude_args(OsStr::new(PLUGIN_DIR), resume, PROMPT);
        let args: Vec<_> = args.iter().map(|arg| arg.to_string_lossy()).collect();
        format!("claude {}", args.join(" "))
    };
    let _ = write!(
        html,
        r##"<section class="sec" aria-labelledby="command-title">
  <div class="wrap">
    <div class="sec-head"><p class="eyebrow">The command line</p><h2 id="command-title">How each session starts</h2></div>
    <p class="lede">Every session runs <code>claude</code> headless in the Run's worktree, in auto permission mode, with the Factory skills loaded as a plugin from a temporary directory. The prompt is one of the templates below.</p>
    <pre class="job-text" aria-label="A new session"><code>{new}</code></pre>
    <p class="lede">A <a href="#prompt-resume">Resume</a> continues the session that ended:</p>
    <pre class="job-text" aria-label="A Resume"><code>{resume}</code></pre>
  </div>
</section>
"##,
        new = highlighted(&line(None)),
        resume = highlighted(&line(Some(SESSION_ID))),
    );
}

fn prompt_section(html: &mut String) {
    html.push_str(
        r##"<section class="sec" aria-labelledby="prompts-title">
  <div class="wrap">
    <div class="sec-head"><p class="eyebrow">Prompts</p><h2 id="prompts-title">What a Run tells the agent</h2></div>
    <p class="lede">Every prompt a Run can send, with its placeholders highlighted: they change for each Run.</p>
    <div class="job-sheets">
"##,
    );
    for prompt in prompts() {
        let text = prompt
            .text
            .replace(&format!("#{ISSUE_NUMBER}"), &format!("#{NUMBER}"));
        let _ = write!(
            html,
            r##"      <article class="job-sheet" id="{id}" aria-labelledby="{id}-title">
        <h3 id="{id}-title">{title}</h3>
        <p>{when}</p>
        <p class="sent-by">Sent by {units}</p>
        <pre class="job-text"><code>{text}</code></pre>
      </article>
"##,
            id = prompt.id,
            title = prompt.title,
            when = prompt.when,
            units = unit_links(prompt.units),
            text = highlighted(&text),
        );
    }
    html.push_str("    </div>\n  </div>\n</section>\n");
}

fn skills_section(html: &mut String) {
    html.push_str(
        r##"<section class="sec" aria-labelledby="skills-title">
  <div class="wrap">
    <div class="sec-head"><p class="eyebrow">Factory skills</p><h2 id="skills-title">The skills, as the agent reads them</h2></div>
    <p class="lede">Every Factory skill and supporting file, raw, exactly as the plugin writes it out.</p>
    <div class="job-sheets">
"##,
    );
    let mut skills: Vec<&Dir> = SKILLS.dirs().collect();
    skills.sort_by_key(|dir| dir.path());
    for skill in skills {
        let name = skill.path().to_string_lossy();
        let (unit, note) = skill_unit(&name);
        let note = note.map_or(String::new(), |note| format!(", {note}"));
        let _ = write!(
            html,
            r##"      <article class="job-sheet" id="skill-{name}" aria-labelledby="skill-{name}-title">
        <h3 id="skill-{name}-title">{name}</h3>
        <p class="sent-by">Used by {unit}{note}</p>
"##,
            unit = unit_links(&[unit]),
        );
        let mut files = Vec::new();
        collect_files(skill, &mut files);
        // SKILL.md first, as the agent reads it first; then the rest by path.
        files.sort_by_key(|file| {
            (
                file.path().file_name() != Some(OsStr::new("SKILL.md")),
                file.path(),
            )
        });
        for file in files {
            file_block(html, file, "        ");
        }
        html.push_str("      </article>\n");
    }
    html.push_str("    </div>\n  </div>\n</section>\n");
}

fn licence_section(html: &mut String) {
    html.push_str(
        r##"<section class="sec" aria-labelledby="licence-title">
  <div class="wrap">
    <div class="sec-head"><p class="eyebrow">Licence and credits</p><h2 id="licence-title">Whose skills these are</h2></div>
    <p class="lede">The Factory skills are adapted from Matt Pocock's skills, under this licence. The <a href="#skill-pr">pr</a> skill credits Dex Horthy's <code>show-me</code> skill in its <a href="#file-pr-credits-md">CREDITS.md</a>.</p>
"##,
    );
    let mut files: Vec<&File> = SKILLS.files().collect();
    files.sort_by_key(|file| file.path());
    for file in files {
        assert_eq!(
            file.path(),
            OsStr::new("LICENSE"),
            "a file outside any skill needs a place on the page"
        );
        file_block(html, file, "    ");
    }
    html.push_str("  </div>\n</section>\n");
}

/// Every file under `dir`, at any depth.
fn collect_files<'a>(dir: &'a Dir<'a>, files: &mut Vec<&'a File<'a>>) {
    files.extend(dir.files());
    for dir in dir.dirs() {
        collect_files(dir, files);
    }
}

/// A file's path as a heading, then its raw text.
fn file_block(html: &mut String, file: &File, indent: &str) {
    let path = file.path().to_string_lossy();
    let text = file
        .contents_utf8()
        .unwrap_or_else(|| panic!("{path} is not UTF-8"));
    let id: String = path
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let _ = write!(
        html,
        "{indent}<h4 id=\"file-{id}\"><code>{path}</code></h4>\n{indent}<pre class=\"job-text\"><code>{text}</code></pre>\n",
        path = escape(&path),
        text = escape(text),
    );
}

/// Links to `units` on the home page.
fn unit_links(units: &[Unit]) -> String {
    units
        .iter()
        .map(|unit| format!(r##"<a href="/#{}">{}</a>"##, unit.anchor, unit.name))
        .collect::<Vec<_>>()
        .join(" or ")
}

/// `text` escaped, with each placeholder marked up.
fn highlighted(text: &str) -> String {
    let mut html = escape(text);
    for placeholder in PLACEHOLDERS {
        let escaped = escape(placeholder);
        html = html.replace(&escaped, &format!(r##"<mark class="ph">{escaped}</mark>"##));
    }
    html
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    const UPDATE: &str = "UPDATE_PROMPTS_PAGE=1 cargo test prompts_page";
    const BEGIN: &str = "<!-- Generated from the source by src/prompts_page.rs; don't edit. Regenerate with UPDATE_PROMPTS_PAGE=1 cargo test prompts_page -->\n";
    const END: &str = "<!-- End of the generated fragment -->\n";

    /// The golden-file test: the page carries the fragment as rendered now.
    /// The crates.io package leaves `site/` out, so it skips there.
    #[test]
    fn prompts_page_shows_what_the_binary_sends() {
        let site = Path::new(env!("CARGO_MANIFEST_DIR")).join("site");
        if !site.is_dir() {
            eprintln!(
                "skipping: no {} (the crates.io package leaves it out)",
                site.display()
            );
            return;
        }
        let page_path = site.join("prompts/index.html");
        let page = fs::read_to_string(&page_path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", page_path.display()));
        let (before, rest) = page
            .split_once(BEGIN)
            .unwrap_or_else(|| panic!("{} has no line {BEGIN}", page_path.display()));
        let (checked_in, after) = rest
            .split_once(END)
            .unwrap_or_else(|| panic!("{} has no line {END}", page_path.display()));
        let rendered = render();
        if std::env::var_os("UPDATE_PROMPTS_PAGE").is_some() {
            fs::write(&page_path, format!("{before}{BEGIN}{rendered}{END}{after}"))
                .unwrap_or_else(|error| panic!("could not write {}: {error}", page_path.display()));
            return;
        }
        assert!(
            checked_in == rendered,
            "{} is out of date with the prompts, the claude arguments or the skills:\n{}\nRegenerate it with: {UPDATE}",
            page_path.display(),
            diff(checked_in, &rendered, before.lines().count() + 2),
        );
    }

    /// The lines of `old` and `new` that differ, as `-` and `+` lines by
    /// longest common subsequence, numbered as lines of `new` from `first`.
    fn diff(old: &str, new: &str, first: usize) -> String {
        let (old, new): (Vec<_>, Vec<_>) = (old.lines().collect(), new.lines().collect());
        // common[i][j]: the longest common subsequence of old[i..] and new[j..].
        let mut common = vec![vec![0usize; new.len() + 1]; old.len() + 1];
        for i in (0..old.len()).rev() {
            for j in (0..new.len()).rev() {
                common[i][j] = if old[i] == new[j] {
                    common[i + 1][j + 1] + 1
                } else {
                    common[i + 1][j].max(common[i][j + 1])
                };
            }
        }
        let (mut i, mut j, mut out) = (0, 0, String::new());
        while i < old.len() || j < new.len() {
            if i < old.len() && j < new.len() && old[i] == new[j] {
                (i, j) = (i + 1, j + 1);
            } else if j == new.len() || (i < old.len() && common[i + 1][j] >= common[i][j + 1]) {
                let _ = writeln!(out, "{:>5} - {}", first + j, old[i]);
                i += 1;
            } else {
                let _ = writeln!(out, "{:>5} + {}", first + j, new[j]);
                j += 1;
            }
        }
        out
    }

    #[test]
    fn diff_shows_only_the_changed_lines() {
        assert_eq!(
            diff("a\nb\nc\n", "a\nB\nc\nd\n", 1),
            "    2 - b\n    2 + B\n    4 + d\n"
        );
    }

    #[test]
    fn placeholders_are_highlighted_and_fixed_text_is_escaped() {
        assert_eq!(
            highlighted("Push <branch>; see `gh run view <run-id>` & <n>"),
            r##"Push <mark class="ph">&lt;branch&gt;</mark>; see `gh run view &lt;run-id&gt;` &amp; <mark class="ph">&lt;n&gt;</mark>"##
        );
    }
}
