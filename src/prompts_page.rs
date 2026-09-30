//! The Prompts and skills page's generated fragment: the `claude` command
//! line, every prompt a Run sends, and every Factory skill, rendered from the
//! same functions and embedded files the binary uses, so the page can't drift
//! from what it does. Each prompt is also rendered as a Markdown file in
//! `prompts/`. Test-only: the golden-file tests below keep the checked-in
//! fragment in `site/prompts/index.html` and the `prompts/` files up to date.

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
const SPEC_URL: &str = "<Spec URL>";
const ISSUE_NUMBER: u64 = u64::MAX;
const NUMBER: &str = "<n>";
const BASE: &str = "<base>";
const BRANCH: &str = "<branch>";
const SPEC_BRANCH: &str = "<Spec branch>";
const PR_URL: &str = "<pull request URL>";
const OWN_HEAD: &str = "<own head>";
const CHECK: &str = "<failing check>";
const CHECK_URL: &str = "<check URL>";
const BACKGROUND_WORK: &str = "<background work>";
const PLUGIN_DIR: &str = "<plugin dir>";
const SESSION_ID: &str = "<session id>";
const PROMPT: &str = "<prompt>";
const PLACEHOLDERS: [&str; 14] = [
    ISSUE_URL,
    SPEC_URL,
    NUMBER,
    BASE,
    BRANCH,
    SPEC_BRANCH,
    PR_URL,
    OWN_HEAD,
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
    let spec = IssueUrl {
        url: SPEC_URL.to_string(),
        ..issue.clone()
    };
    let failed = [Check {
        name: CHECK.to_string(),
        state: CheckState::Failed,
        url: Some(CHECK_URL.to_string()),
    }];
    let with_sentinel = vec![
        Prompt {
            id: "prompt-fresh",
            title: "Fresh",
            when: "Starts the implement session, which goes on to review and open the pull request, when the Run starts a new Issue branch.",
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
            id: "prompt-spec-review",
            title: "Spec review",
            when: "In a Spec run, starts the Spec review once every Ticket has landed on the Spec branch, before the Spec PR, a draft until then, is marked ready.",
            units: &[REVIEW],
            text: prompt::spec_review(&spec, BASE, SPEC_BRANCH, PR_URL),
        },
        Prompt {
            id: "prompt-conflict-repair",
            title: "Conflict Repair",
            when: "Starts a Repair session when merging the Base branch into the Issue branch leaves conflicts.",
            units: &[FINISH],
            text: prompt::conflict_repair(&issue, BASE, BRANCH, PR_URL),
        },
        Prompt {
            id: "prompt-foreign-conflict-repair",
            title: "Conflict Repair, on Foreign commits",
            when: "In a Merge run, starts a Repair session when merging Foreign commits from the Issue branch on origin into the local one leaves conflicts.",
            units: &[FINISH],
            text: prompt::conflict_repair(&issue, BRANCH, BRANCH, PR_URL),
        },
        Prompt {
            id: "prompt-review-repair",
            title: "Review Repair",
            when: "In a Merge run, starts a Repair session once Foreign commits are merged into the Issue branch, to review them from the head the Run last knew as its own before they can be merged.",
            units: &[FINISH],
            text: prompt::review_repair(&issue, BRANCH, PR_URL, OWN_HEAD),
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
            when: "Continues any session, once, that ended its turn while waiting on background work, which was killed with it.",
            units: &[IMPLEMENT, REVIEW, FINISH],
            text: prompt::resume(&[BACKGROUND_WORK]),
        },
    ];
    with_sentinel
        .into_iter()
        .map(|prompt| Prompt {
            text: prompt
                .text
                .replace(&format!("#{ISSUE_NUMBER}"), &format!("#{NUMBER}")),
            ..prompt
        })
        .collect()
}

/// The command that regenerates the page's fragment and `prompts/`.
const UPDATE: &str = "UPDATE_PROMPTS=1 cargo test prompts_page";

/// A file in `prompts/`: its name, after the prompt's id, and its contents.
struct PromptFile {
    name: String,
    contents: String,
}

/// Each prompt as a Markdown file for `prompts/`.
fn prompt_files() -> Vec<PromptFile> {
    prompts()
        .iter()
        .map(|prompt| {
            let name = prompt
                .id
                .strip_prefix("prompt-")
                .unwrap_or_else(|| panic!("the prompt id {} has no prompt- prefix", prompt.id));
            PromptFile {
                name: format!("{name}.md"),
                contents: markdown(prompt),
            }
        })
        .collect()
}

/// A prompt as Markdown: a note on where it comes from, then its title, when
/// it is sent, and its text, fenced.
fn markdown(prompt: &Prompt) -> String {
    let fence = fence(&prompt.text);
    format!(
        "<!-- Generated by src/prompts_page.rs from the prompts in src/prompt.rs and their titles and when sentences in src/prompts_page.rs; don't edit. Regenerate with {UPDATE} -->\n\n# {title}\n\n{when}\n\n{fence}\n{text}\n{fence}\n",
        title = prompt.title,
        when = prompt.when,
        text = prompt.text.trim_end_matches('\n'),
    )
}

/// A code fence longer than any run of backticks in `text`, so none closes it.
fn fence(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    "`".repeat(longest.max(2) + 1)
}

/// The unit whose session uses each Factory skill, and when, if not always.
fn skill_unit(skill: &str) -> (Unit, Option<&'static str>) {
    match skill {
        "implement" | "tdd" => (IMPLEMENT, None),
        "code-review" | "pr" => (REVIEW, None),
        "resolving-merge-conflicts" => (FINISH, Some("in a Repair")),
        _ => panic!("the Factory skill {skill} has no unit: add it to skill_unit"),
    }
}

/// The fragment: the command line, the prompts, the skills, and the skills'
/// licence.
fn render() -> String {
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
            text = highlighted(&prompt.text),
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

/// Links to `units` on the home page, as a list ending "or".
fn unit_links(units: &[Unit]) -> String {
    let links: Vec<_> = units
        .iter()
        .map(|unit| format!(r##"<a href="/#{}">{}</a>"##, unit.anchor, unit.name))
        .collect();
    match links.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => String::new(),
    }
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
    use std::io::ErrorKind;
    use std::path::{Path, PathBuf};

    use super::*;

    const END: &str = "<!-- End of the generated fragment -->\n";

    /// The line the page's fragment starts after.
    fn begin() -> String {
        format!(
            "<!-- Generated from the source by src/prompts_page.rs; don't edit. Regenerate with {UPDATE} -->\n"
        )
    }

    /// Whether to regenerate the page's fragment and `prompts/` rather than
    /// check them.
    fn updating() -> bool {
        std::env::var_os("UPDATE_PROMPTS").is_some()
    }

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
        let begin = begin();
        let (before, rest) = page
            .split_once(&begin)
            .unwrap_or_else(|| panic!("{} has no line {begin}", page_path.display()));
        let (checked_in, after) = rest
            .split_once(END)
            .unwrap_or_else(|| panic!("{} has no line {END}", page_path.display()));
        let rendered = render();
        if updating() {
            fs::write(&page_path, format!("{before}{begin}{rendered}{END}{after}"))
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

    /// The golden-file test for `prompts/`: a file for each prompt, as
    /// rendered now, and no other. The crates.io package leaves `prompts/`
    /// out, so it skips there.
    #[test]
    fn prompt_files_show_what_the_binary_sends() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("prompts");
        if !dir.is_dir() {
            eprintln!(
                "skipping: no {} (the crates.io package leaves it out)",
                dir.display()
            );
            return;
        }
        let files = prompt_files();
        if updating() {
            write_prompt_files(&dir, &files);
            return;
        }
        let problems = check_prompt_files(&dir, &files);
        assert!(
            problems.is_empty(),
            "{}\nRegenerate them with: {UPDATE}",
            problems.join("\n"),
        );
    }

    /// What is wrong with the files in `dir`, compared with `files`: each
    /// file that differs or is missing, with a diff, and each stale one.
    fn check_prompt_files(dir: &Path, files: &[PromptFile]) -> Vec<String> {
        let mut problems = Vec::new();
        for file in files {
            let path = dir.join(&file.name);
            match fs::read_to_string(&path) {
                Ok(checked_in) if checked_in == file.contents => {}
                Ok(checked_in) => problems.push(format!(
                    "{} is out of date with the prompts:\n{}",
                    path.display(),
                    diff(&checked_in, &file.contents, 1),
                )),
                Err(error) if error.kind() == ErrorKind::NotFound => problems.push(format!(
                    "{} is missing:\n{}",
                    path.display(),
                    diff("", &file.contents, 1),
                )),
                Err(error) => panic!("could not read {}: {error}", path.display()),
            }
        }
        for path in stale_files(dir, files) {
            problems.push(format!(
                "{} is stale: no prompt produces it",
                path.display()
            ));
        }
        problems
    }

    /// Writes `files` into `dir` and deletes the stale ones.
    fn write_prompt_files(dir: &Path, files: &[PromptFile]) {
        for file in files {
            let path = dir.join(&file.name);
            fs::write(&path, &file.contents)
                .unwrap_or_else(|error| panic!("could not write {}: {error}", path.display()));
        }
        for path in stale_files(dir, files) {
            let removed = if path.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            };
            removed.unwrap_or_else(|error| panic!("could not delete {}: {error}", path.display()));
        }
    }

    /// The entries in `dir` that none of `files` is named after.
    fn stale_files(dir: &Path, files: &[PromptFile]) -> Vec<PathBuf> {
        let entries = fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", dir.display()));
        let mut stale: Vec<_> = entries
            .map(|entry| {
                entry
                    .unwrap_or_else(|error| panic!("could not read {}: {error}", dir.display()))
                    .path()
            })
            .filter(|path| {
                !files
                    .iter()
                    .any(|file| path.file_name() == Some(OsStr::new(&file.name)))
            })
            .collect();
        stale.sort();
        stale
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
    fn a_fence_is_longer_than_any_run_of_backticks_in_the_text() {
        assert_eq!(fence("no backticks"), "```");
        assert_eq!(fence("see `gh run view`"), "```");
        assert_eq!(fence("a block:\n```sh\nls\n```\n"), "````");
        assert_eq!(fence("``` then `````"), "``````");
    }

    #[test]
    fn a_prompt_file_carries_the_title_the_when_and_the_fenced_text() {
        let prompt = Prompt {
            id: "prompt-example",
            title: "Example",
            when: "When an example runs.",
            units: &[IMPLEMENT],
            text: "Say ```hi```.\n".to_string(),
        };
        let markdown = markdown(&prompt);
        let (note, rest) = markdown.split_once("\n\n").unwrap();
        assert!(
            note.starts_with("<!-- Generated ") && note.contains(UPDATE),
            "{note}"
        );
        assert_eq!(
            rest,
            "# Example\n\nWhen an example runs.\n\n````\nSay ```hi```.\n````\n"
        );
    }

    #[test]
    fn checking_prompts_names_each_missing_differing_and_stale_file() {
        let temp = tempfile::TempDir::new().unwrap();
        let dir = temp.path();
        fs::write(dir.join("same.md"), "same\n").unwrap();
        fs::write(dir.join("changed.md"), "old\n").unwrap();
        fs::write(dir.join("stray.md"), "stray\n").unwrap();
        fs::create_dir(dir.join("stray-dir")).unwrap();
        let file = |name: &str, contents: &str| PromptFile {
            name: name.to_string(),
            contents: contents.to_string(),
        };
        let files = [
            file("same.md", "same\n"),
            file("changed.md", "new\n"),
            file("missing.md", "missing\n"),
        ];
        let problems = check_prompt_files(dir, &files);
        assert_eq!(problems.len(), 4, "{problems:?}");
        assert!(
            problems[0].contains("changed.md")
                && problems[0].contains("    1 - old\n    1 + new\n")
        );
        assert!(problems[1].contains("missing.md") && problems[1].contains("    1 + missing\n"));
        assert!(problems[2].contains("stray-dir") && problems[2].contains("no prompt"));
        assert!(problems[3].contains("stray.md") && problems[3].contains("no prompt"));

        write_prompt_files(dir, &files);
        assert!(check_prompt_files(dir, &files).is_empty());
        assert!(!dir.join("stray.md").exists() && !dir.join("stray-dir").exists());
    }

    #[test]
    fn placeholders_are_highlighted_and_fixed_text_is_escaped() {
        assert_eq!(
            highlighted("Push <branch>; see `gh run view <run-id>` & <n>"),
            r##"Push <mark class="ph">&lt;branch&gt;</mark>; see `gh run view &lt;run-id&gt;` &amp; <mark class="ph">&lt;n&gt;</mark>"##
        );
    }
}
