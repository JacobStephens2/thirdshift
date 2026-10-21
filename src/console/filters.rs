//! Template filters shared by the Console's pages.
use askama::filters::{Escaper, Html, Safe};
use askama::{Result, Values};

/// Escape `text` and link each `https://` URL in it, as the findings list does.
/// Trailing sentence punctuation stays outside the link.
#[askama::filter_fn]
pub fn linkify(text: impl std::fmt::Display, _: &dyn Values) -> Result<Safe<String>> {
    let text = text.to_string();
    let mut html = String::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find("https://") {
        let (before, tail) = rest.split_at(start);
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`'))
            .unwrap_or(tail.len());
        let url = tail[..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')']);
        Html.write_escaped_str(&mut html, before)?;
        if url.len() > "https://".len() {
            html.push_str("<a href=\"");
            Html.write_escaped_str(&mut html, url)?;
            html.push_str("\">");
            Html.write_escaped_str(&mut html, url)?;
            html.push_str("</a>");
        } else {
            Html.write_escaped_str(&mut html, url)?;
        }
        rest = &tail[url.len()..];
    }
    Html.write_escaped_str(&mut html, rest)?;
    Ok(Safe(html))
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
}
