//! The fake `gh` and `claude` the end-to-end tests put first on PATH: one
//! executable, which acts as whichever its name says. See `fakes/gh.rs` and
//! `fakes/claude.rs` for what each does, and `support/fakes.rs` for how the
//! harness builds it.
//!
//! It uses only the standard library, so the harness can build it with plain
//! `rustc`. It started out as two Python scripts, whose startup cost was about
//! half the suite's CPU time (#207); where it mimics Python, such as in how
//! it lays out JSON, that is to keep their contract.
//!
//! Cargo also builds this file as a test target, which checks the JSON
//! layout below and puts the fakes under clippy and rustfmt.

#![cfg_attr(test, allow(dead_code))]

#[path = "fakes/claude.rs"]
mod claude;
#[path = "fakes/gh.rs"]
mod gh;
#[path = "fakes/json.rs"]
mod json;

use std::io::Write;
use std::path::Path;

use json::Json;

/// Print `message` to stderr and exit with `code`, as the fakes fail.
fn die(message: &str, code: i32) -> ! {
    eprintln!("{message}");
    exit(code)
}

/// Exit with `code`, once stdout is flushed.
fn exit(code: i32) -> ! {
    std::io::stdout().flush().unwrap();
    std::process::exit(code)
}

fn read_json(path: &Path) -> Json {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
    json::parse(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// `items` as Python shows a list of strings, for messages the Python fakes
/// wrote that way.
fn python_list(items: &[String]) -> String {
    let quoted: Vec<String> = items
        .iter()
        .map(|item| format!("'{}'", item.replace('\\', "\\\\").replace('\'', "\\'")))
        .collect();
    format!("[{}]", quoted.join(", "))
}

fn main() {
    // A fake that trips over something its contract doesn't cover fails with
    // exit code 1, as an uncaught Python exception did, not Rust's 101.
    std::panic::set_hook(Box::new(|info| {
        eprintln!("fake: {info}");
        exit(1);
    }));
    let mut args = std::env::args();
    let name = args.next().expect("no argv[0]");
    let name = Path::new(&name).file_name().unwrap().to_str().unwrap();
    match name {
        "gh" => gh::main(args.collect()),
        "claude" => claude::main(args.collect()),
        _ => die(&format!("fake: no fake is called {name}"), 2),
    }
    exit(0);
}

#[cfg(test)]
mod tests {
    use super::json::{self, Json};

    const STATE: &str = r#"{"repo": "acme/widgets", "prs": [], "checks": {"abc": [{"name": "build", "pending_polls": 2, "url": null, "ok": true}]}, "note": "caf\u00e9 \ud83d\ude00 \"q\"\n"}"#;

    #[test]
    fn prints_json_on_one_line_as_python_json_dumps_does() {
        let state = json::parse(STATE).unwrap();
        assert_eq!(state.to_string(), STATE);
    }

    #[test]
    fn dumps_json_with_an_indent_of_two_as_python_json_dump_does() {
        let state = json::parse(STATE).unwrap();
        assert_eq!(
            state.dump(),
            r#"{
  "repo": "acme/widgets",
  "prs": [],
  "checks": {
    "abc": [
      {
        "name": "build",
        "pending_polls": 2,
        "url": null,
        "ok": true
      }
    ]
  },
  "note": "caf\u00e9 \ud83d\ude00 \"q\"\n"
}"#
        );
    }

    #[test]
    fn reads_escapes_back_as_the_characters_they_stand_for() {
        let state = json::parse(STATE).unwrap();
        assert_eq!(state.at("note").str(), "café 😀 \"q\"\n");
        assert_eq!(json::parse(r#""a\/b\tc""#).unwrap(), json::string("a/b\tc"));
    }

    #[test]
    fn setting_a_key_keeps_its_place_and_a_new_key_goes_last() {
        let mut state = json::parse(STATE).unwrap();
        state.set("repo", json::string("acme/gadgets"));
        state.set("user_email", Json::Null);
        let renamed = STATE.replace("widgets", "gadgets");
        let expected = format!(
            "{}, \"user_email\": null}}",
            renamed.strip_suffix('}').unwrap()
        );
        assert_eq!(state.to_string(), expected);
    }

    #[test]
    fn rejects_text_that_is_not_one_json_value() {
        assert!(json::parse("{\"a\": 1} x").is_err());
        assert!(json::parse("[1, ]").is_err());
        assert!(json::parse("\"open").is_err());
    }
}
