//! Host-side image preparation for the existing issue a Triage session reads.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use reqwest::{Url, blocking::Client, redirect::Policy};

use crate::{git::Git, github::GitHub, issue::IssueUrl, process, prompt, skills};

const DIRECTORY: &str = ".thirdshift-issue-images";
const MAX_IMAGES: usize = 10;
const MAX_BYTES: u64 = 20 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

/// All network and filesystem preparation finishes on the host, before the
/// session (including a read-machine session) starts reading its issue.
pub(super) fn hand_over(issue: &IssueUrl, worktree: &Path) -> Result<()> {
    skills::exclude(&Git::new(worktree), &format!("/{}", prompt::ISSUE_FILE))?;
    if let Err(error) = skills::exclude(&Git::new(worktree), &format!("/{DIRECTORY}/")) {
        eprintln!("warning: could not exclude cached issue images from git: {error:#}");
    }
    let mut issue = GitHub::new().session_issue(issue)?;
    let mut http = None;
    for warning in cache(
        &mut issue.markdown,
        &issue.trusted_bodies,
        worktree,
        |url| {
            let http = match &http {
                Some(http) => http,
                None => http.insert(Downloads::new()?),
            };
            http.open(url)
        },
    ) {
        eprintln!("warning: {warning}");
    }
    fs::write(worktree.join(prompt::ISSUE_FILE), issue.markdown)
        .context("could not write the Triage issue text")
}

fn cache(
    markdown: &mut String,
    bodies: &[String],
    worktree: &Path,
    mut download: impl FnMut(&Url) -> Result<Box<dyn Read>>,
) -> Vec<String> {
    let urls = image_urls(bodies);
    if urls.is_empty() {
        return Vec::new();
    }
    let mut warnings = Vec::new();
    if urls.len() > MAX_IMAGES {
        warnings
            .push("issue images exceed the limit of 10 unique images; skipping the rest".into());
    }
    let directory = worktree.join(DIRECTORY);
    let setup = (|| -> Result<()> {
        // A repository may already contain this name. Refuse symlinks, and
        // never overwrite an existing file when publishing a download.
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(&directory)?;
                ensure!(metadata.is_dir(), "image directory is not a directory");
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    })();
    if let Err(error) = setup {
        warnings.push(format!("could not prepare issue images: {error:#}"));
        return warnings;
    }
    let mut listing = String::new();
    for (index, url) in urls.iter().take(MAX_IMAGES).enumerate() {
        let result = (|| -> Result<String> {
            let reader = download(url)?;
            let mut partial = tempfile::NamedTempFile::new_in(&directory)?;
            let size = io::copy(&mut reader.take(MAX_BYTES + 1), &mut partial)?;
            ensure!(size <= MAX_BYTES, "image exceeds 20 MiB");
            let extension = url
                .path()
                .rsplit('.')
                .next()
                .filter(|extension| {
                    matches!(
                        *extension,
                        "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "svg" | "bmp"
                    )
                })
                .unwrap_or("img");
            let relative = format!("{DIRECTORY}/image-{}.{extension}", index + 1);
            partial.persist_noclobber(worktree.join(&relative))?;
            Ok(relative)
        })();
        match result {
            Ok(path) => listing.push_str(&format!("- <{url}> → `{path}`\n")),
            Err(error) => warnings.push(format!("could not download issue image {url}: {error:#}")),
        }
    }
    if !listing.is_empty() {
        markdown.push_str("\n## Downloaded issue images\n\nInspect these local images as evidence for the investigation:\n\n");
        markdown.push_str(&listing);
    }
    warnings
}

fn allowed(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url
            .host_str()
            .is_some_and(|host| host == "github.com" || host.ends_with(".githubusercontent.com"))
}

fn image_urls(bodies: &[String]) -> Vec<Url> {
    let selector = scraper::Selector::parse("img[src]").expect("static image selector");
    let mut seen = HashSet::new();
    let mut urls = Vec::new();
    for body in bodies {
        let mut html = String::new();
        pulldown_cmark::html::push_html(&mut html, pulldown_cmark::Parser::new(body));
        let document = scraper::Html::parse_fragment(&html);
        for image in document.select(&selector) {
            if let Some(url) = image.attr("src").and_then(|src| Url::parse(src).ok())
                && allowed(&url)
            {
                let mut key = url.clone();
                key.set_fragment(None);
                if seen.insert(key) {
                    urls.push(url);
                    // One extra establishes the warning without collecting
                    // an unbounded list of attachments from many comments.
                    if urls.len() > MAX_IMAGES {
                        return urls;
                    }
                }
            }
        }
    }
    urls
}

struct Downloads {
    client: Client,
    token: Option<String>,
}

impl Downloads {
    fn new() -> Result<Self> {
        let client = Client::builder()
            .https_only(true)
            .timeout(TIMEOUT)
            .redirect(redirect_policy())
            .build()?;
        // gh knows environment tokens as well as its saved login. Keep token
        // lookup bounded and let public images work without a login.
        let token = process::output_until(
            Command::new("gh").args(["auth", "token", "--hostname", "github.com"]),
            None,
            process::Control {
                name: "gh auth token",
                interruption: process::Interruption::Ordinary,
                stop: &|child| process::stop(child, &[libc::SIGTERM]),
            },
            Instant::now() + Duration::from_secs(5),
            &|| Ok(()),
        )
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty());
        Ok(Self { client, token })
    }

    fn open(&self, url: &Url) -> Result<Box<dyn Read>> {
        process::Interruption::Ordinary.check()?;
        // The request timeout also bounds the async response body's total
        // lifetime; a blocking-client timeout alone resets on every read.
        let mut request = self.client.get(url.clone()).timeout(TIMEOUT);
        // Only GitHub receives its credential. reqwest strips Authorization
        // on cross-host redirects to signed githubusercontent URLs.
        if url.host_str() == Some("github.com")
            && let Some(token) = &self.token
        {
            request = request.bearer_auth(token);
        }
        Ok(Box::new(request.send()?.error_for_status()?))
    }
}

fn redirect_policy() -> Policy {
    Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 || !allowed(attempt.url()) {
            attempt.error("issue image redirect outside the allowed hosts or too many redirects")
        } else {
            attempt.follow()
        }
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    #[test]
    fn caches_markdown_and_html_images_once_and_lists_local_paths() {
        let directory = tempfile::tempdir().unwrap();
        let mut markdown = "trusted issue text\n".to_string();
        let bodies = vec![String::from(
            "![screen](https://github.com/user-attachments/assets/one)\n\
             <img alt='screen' src='https://private-user-images.githubusercontent.com/two.png'>\n\
             ![duplicate](https://github.com/user-attachments/assets/one)\n\
             ![http](http://github.com/unsafe) ![foreign](https://example.com/no)\n\
             <img src='https://github.com.evil.example/no'>",
        )];
        let mut fetched = Vec::new();
        let warnings = super::cache(&mut markdown, &bodies, directory.path(), |url| {
            fetched.push(url.to_string());
            Ok(Box::new(Cursor::new(b"image bytes")) as Box<dyn Read>)
        });
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            fetched,
            [
                "https://github.com/user-attachments/assets/one",
                "https://private-user-images.githubusercontent.com/two.png",
            ]
        );
        for (i, url) in fetched.iter().enumerate() {
            let extension = if i == 0 { "img" } else { "png" };
            let path = format!(".thirdshift-issue-images/image-{}.{extension}", i + 1);
            assert_eq!(
                std::fs::read(directory.path().join(&path)).unwrap(),
                b"image bytes"
            );
            assert!(markdown.contains(url), "{markdown}");
            assert!(markdown.contains(&path), "{markdown}");
        }
    }

    #[test]
    fn limits_attempts_to_ten_unique_images_and_warns_about_excess() {
        let directory = tempfile::tempdir().unwrap();
        let mut markdown = "issue".to_string();
        let body = (1..=12)
            .map(|i| format!("![{i}](https://github.com/{i}.png)\n"))
            .collect::<String>();
        let mut attempts = 0;
        let warnings = super::cache(&mut markdown, &[body], directory.path(), |_| {
            attempts += 1;
            Ok(Box::new(Cursor::new(b"png")) as Box<dyn Read>)
        });
        assert_eq!(attempts, 10);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("limit of 10"), "{warnings:?}");
        assert_eq!(
            std::fs::read_dir(directory.path().join(super::DIRECTORY))
                .unwrap()
                .count(),
            10
        );
        assert!(markdown.contains("image-10.png"));
        assert!(!markdown.contains("image-11.png"));
    }

    #[test]
    fn failed_and_oversized_images_leave_no_partial_files_and_twenty_mib_is_allowed() {
        let directory = tempfile::tempdir().unwrap();
        let mut markdown = "issue evidence".to_string();
        let bodies = [String::from(
            "![failed](https://github.com/failed.png)\n\
             ![large](https://github.com/large.png)\n\
             ![exact](https://github.com/exact.png)",
        )];
        let warnings = super::cache(&mut markdown, &bodies, directory.path(), |url| {
            match url.path() {
                "/failed.png" => anyhow::bail!("HTTP 404"),
                "/large.png" => Ok(Box::new(std::io::repeat(0)) as Box<dyn Read>),
                "/exact.png" => {
                    Ok(Box::new(std::io::repeat(1).take(20 * 1024 * 1024)) as Box<dyn Read>)
                }
                other => panic!("unexpected download {other}"),
            }
        });
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("HTTP 404"), "{warnings:?}");
        assert!(warnings[1].contains("exceeds 20 MiB"), "{warnings:?}");
        let files = std::fs::read_dir(directory.path().join(super::DIRECTORY))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(files, ["image-3.png"]);
        assert_eq!(
            std::fs::metadata(directory.path().join(super::DIRECTORY).join("image-3.png"))
                .unwrap()
                .len(),
            20 * 1024 * 1024
        );
        assert!(markdown.starts_with("issue evidence"));
        assert!(!markdown.contains("image-1.png"));
        assert!(!markdown.contains("image-2.png"));
        assert!(markdown.contains("https://github.com/exact.png"));
    }

    #[test]
    fn parses_image_syntax_without_fetching_code_links_or_disallowed_origins() {
        let directory = tempfile::tempdir().unwrap();
        let body = r###"
![escaped alt \]](<https://github.com/image_(one).png> "title")
<IMG width="50" SRC="https://user-images.githubusercontent.com/two.png?a=1&amp;b=2">
![fragment duplicate](https://github.com/image_(one).png#caption)
[ordinary link](https://github.com/link.png)
`![code](https://github.com/code.png)`
```
<img src="https://github.com/fenced.png">
```
![credentials](https://user:password@github.com/credentials)
![port](https://github.com:9999/port)
![host](https://githubusercontent.com/apex)
![suffix](https://evilgithubusercontent.com/suffix)
<img src="//github.com/relative">
<img src="https://github.com.evil.example/foreign">
<img src="http://github.com/insecure">
"###;
        let mut fetched = Vec::new();
        let warnings = super::cache(
            &mut String::new(),
            &[body.to_string()],
            directory.path(),
            |url| {
                fetched.push(url.to_string());
                Ok(Box::new(Cursor::new(b"image")) as Box<dyn Read>)
            },
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            fetched,
            [
                "https://github.com/image_(one).png",
                "https://user-images.githubusercontent.com/two.png?a=1&b=2",
            ]
        );
    }

    #[test]
    fn refuses_a_symlink_cache_directory_and_keeps_the_issue_text() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), directory.path().join(super::DIRECTORY))
            .unwrap();
        let mut markdown = "original issue".to_string();
        let warnings = super::cache(
            &mut markdown,
            &["![screen](https://github.com/screen.png)".into()],
            directory.path(),
            |_| panic!("must refuse the directory before downloading"),
        );
        assert_eq!(warnings.len(), 1);
        assert_eq!(markdown, "original issue");
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[test]
    fn http_download_uses_authentication_and_rejects_errors_and_foreign_redirects() {
        use std::io::Write;
        use std::net::TcpListener;
        use std::time::Duration;

        for (response, succeeds) in [
            ("HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\npng", true),
            ("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n", false),
            (
                "HTTP/1.1 302 Found\r\nLocation: https://evil.example/image.png\r\nContent-Length: 0\r\n\r\n",
                false,
            ),
            (
                "HTTP/1.1 302 Found\r\nLocation: http://github.com/image.png\r\nContent-Length: 0\r\n\r\n",
                false,
            ),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(30)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                stream.write_all(response.as_bytes()).unwrap();
                String::from_utf8(request).unwrap()
            });
            let downloads = super::Downloads {
                client: reqwest::blocking::Client::builder()
                    .no_proxy()
                    .resolve("github.com", address)
                    .redirect(super::redirect_policy())
                    .build()
                    .unwrap(),
                token: Some("fixture-token".into()),
            };
            let url =
                reqwest::Url::parse(&format!("http://github.com:{}/private.png", address.port()))
                    .unwrap();
            let result = downloads.open(&url).and_then(|mut reader| {
                let mut bytes = Vec::new();
                reader.read_to_end(&mut bytes)?;
                Ok(bytes)
            });
            let request = server.join().unwrap();
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer fixture-token"),
                "{request}"
            );
            assert_eq!(result.is_ok(), succeeds, "{result:?}");
            if let Ok(bytes) = result {
                assert_eq!(bytes, b"png");
            }
        }
    }
}
