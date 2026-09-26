//! How a link to thirdshift.app looks when it is shared: the home page's Open
//! Graph and Twitter card tags, and the card image they point at, a raster of
//! the press line at the size the platforms recommend.

use std::fs;
use std::path::{Path, PathBuf};

const ORIGIN: &str = "https://thirdshift.app";

fn site() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("site")
}

fn home_page() -> String {
    fs::read_to_string(site().join("index.html")).unwrap()
}

/// The `content` of the `<meta>` whose `attribute` (`property` or `name`) is
/// `key`. The site's meta tags are hand-written one to a line, with the key
/// attribute before `content`.
fn meta(html: &str, attribute: &str, key: &str) -> Option<String> {
    let opening = format!(r#"<meta {attribute}="{key}" content=""#);
    let start = html.find(&opening)? + opening.len();
    let end = start + html[start..].find('"')?;
    Some(html[start..end].to_owned())
}

fn og(html: &str, key: &str) -> String {
    meta(html, "property", &format!("og:{key}"))
        .unwrap_or_else(|| panic!("the home page has no og:{key}"))
}

fn twitter(html: &str, key: &str) -> String {
    meta(html, "name", &format!("twitter:{key}"))
        .unwrap_or_else(|| panic!("the home page has no twitter:{key}"))
}

/// The file under `site/` an absolute thirdshift.app URL is served from.
fn served_file(url: &str) -> PathBuf {
    let path = url
        .strip_prefix(ORIGIN)
        .unwrap_or_else(|| panic!("{url} is not an absolute {ORIGIN} URL"));
    site().join(path.trim_start_matches('/'))
}

/// A PNG's width and height, from its IHDR chunk.
fn png_size(bytes: &[u8]) -> (u32, u32) {
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");
    assert_eq!(&bytes[12..16], b"IHDR");
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    (width, height)
}

#[test]
fn the_home_page_has_complete_open_graph_tags_with_absolute_urls() {
    let html = home_page();

    assert_eq!(og(&html, "type"), "website");
    assert_eq!(og(&html, "url"), format!("{ORIGIN}/"));
    for key in ["site_name", "title", "description", "image:alt"] {
        assert!(!og(&html, key).is_empty(), "og:{key} is empty");
    }
    assert!(og(&html, "image").starts_with(&format!("{ORIGIN}/")));
    assert_eq!(og(&html, "image:type"), "image/png");
}

#[test]
fn the_open_graph_title_and_description_are_the_pages_own() {
    let html = home_page();
    let title =
        &html[html.find("<title>").unwrap() + "<title>".len()..html.find("</title>").unwrap()];

    assert_eq!(og(&html, "title"), title);
    assert_eq!(
        Some(og(&html, "description")),
        meta(&html, "name", "description")
    );
}

#[test]
fn the_home_page_has_a_large_image_twitter_card_matching_open_graph() {
    let html = home_page();

    assert_eq!(twitter(&html, "card"), "summary_large_image");
    assert_eq!(twitter(&html, "title"), og(&html, "title"));
    assert_eq!(twitter(&html, "description"), og(&html, "description"));
    assert_eq!(twitter(&html, "image"), og(&html, "image"));
    assert_eq!(twitter(&html, "image:alt"), og(&html, "image:alt"));
}

#[test]
fn the_card_image_is_served_at_the_size_the_tags_declare() {
    let html = home_page();
    let image = fs::read(served_file(&og(&html, "image"))).unwrap();

    let (width, height) = png_size(&image);

    assert_eq!((width, height), (1200, 630));
    assert_eq!(og(&html, "image:width"), width.to_string());
    assert_eq!(og(&html, "image:height"), height.to_string());
}
