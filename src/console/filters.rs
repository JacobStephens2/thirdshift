//! Escaping and URL linking shared by the Console's templates and its
//! conversation renderer. Askama finds template filters by this module's name.
use askama::filters::{Escaper, Html, Safe};
use askama::{Result, Values};

/// Escape `text` and link each `https://` URL in it; other schemes stay text.
/// Instants in it read in this machine's zone, to the minute.
#[askama::filter_fn]
pub fn linkify(text: impl std::fmt::Display, _: &dyn Values) -> Result<Safe<String>> {
    Ok(Safe(linked(&crate::local_time::readable(
        &text.to_string(),
    ))))
}

/// `text` with its instants in this machine's zone, to the minute.
#[askama::filter_fn]
pub fn readable(text: impl std::fmt::Display, _: &dyn Values) -> Result<String> {
    Ok(crate::local_time::readable(&text.to_string()).into_owned())
}

/// `text` on one line. A carriage return in the terminal-style crontab
/// listing would start another line and look like a separate entry.
#[askama::filter_fn]
pub fn one_line(text: impl std::fmt::Display, _: &dyn Values) -> Result<String> {
    Ok(text.to_string().replace('\r', "\u{240D}"))
}

/// Escaped HTML for `text`, with each `https://` URL a link. Trailing
/// sentence punctuation, and a `)` the URL did not open, stay outside it.
pub(super) fn linked(text: &str) -> String {
    let mut html = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("https://") {
        let (before, tail) = rest.split_at(start);
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`'))
            .unwrap_or(tail.len());
        let mut url = &tail[..end];
        loop {
            let trimmed = url.trim_end_matches(['.', ',', ';', ':', '!', '?']);
            match trimmed.strip_suffix(')') {
                Some(open) if trimmed.matches('(').count() < trimmed.matches(')').count() => {
                    url = open
                }
                _ => {
                    url = trimmed;
                    break;
                }
            }
        }
        escape(&mut html, before);
        if url.len() > "https://".len() {
            html.push_str("<a href=\"");
            escape(&mut html, url);
            html.push_str("\">");
            escape(&mut html, url);
            html.push_str("</a>");
        } else {
            escape(&mut html, url);
        }
        rest = &tail[url.len()..];
    }
    escape(&mut html, rest);
    html
}

/// Escaped HTML for `text`, the same as the templates' own escaping.
pub(super) fn escaped(text: &str) -> String {
    let mut html = String::new();
    escape(&mut html, text);
    html
}

fn escape(html: &mut String, text: &str) {
    Html.write_escaped_str(html, text)
        .expect("writing to a String cannot fail");
}

#[cfg(test)]
mod tests {
    use crate::console::filters;

    #[derive(askama::Template)]
    #[template(source = "{{ text|linkify }}", ext = "html")]
    struct Linked<'a> {
        text: &'a str,
    }

    fn linked(text: &str) -> String {
        askama::Template::render(&Linked { text }).unwrap()
    }

    #[test]
    fn links_https_urls_and_escapes_everything_else() {
        assert_eq!(
            linked("PR https://github.com/a/b/pull/1 is a draft."),
            "PR <a href=\"https://github.com/a/b/pull/1\">https://github.com/a/b/pull/1</a> is a draft."
        );
        assert_eq!(
            linked("(see https://x.test/a?b=1&c=2), <b>"),
            "(see <a href=\"https://x.test/a?b=1&#38;c=2\">https://x.test/a?b=1&#38;c=2</a>), &#60;b&#62;"
        );
        assert_eq!(
            linked("https://x.test/\"onmouseover=1"),
            "<a href=\"https://x.test/\">https://x.test/</a>&#34;onmouseover=1"
        );
        assert_eq!(
            linked("http://x.test javascript:alert(1) https://"),
            "http://x.test javascript:alert(1) https://"
        );
    }

    #[derive(askama::Template)]
    #[template(source = "{{ text|one_line }}", ext = "html")]
    struct OneLine<'a> {
        text: &'a str,
    }

    #[test]
    fn shows_a_carriage_return_without_starting_a_line() {
        let html = askama::Template::render(&OneLine {
            text: "thirdshift AB\r@daily echo SECOND <script>",
        })
        .unwrap();
        assert_eq!(
            html,
            "thirdshift AB\u{240D}@daily echo SECOND &#60;script&#62;"
        );
        assert!(!html.contains('\r'));
    }

    #[test]
    fn keeps_a_closing_paren_that_belongs_to_the_url() {
        assert_eq!(
            linked("see https://en.wikipedia.org/wiki/Rust_(programming_language)."),
            "see <a href=\"https://en.wikipedia.org/wiki/Rust_(programming_language)\">https://en.wikipedia.org/wiki/Rust_(programming_language)</a>."
        );
    }
}
