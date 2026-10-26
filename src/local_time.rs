//! Instants as people read them: this machine's date, time to the minute
//! and zone name, such as `2026-10-25 17:43 EDT`. Records keep RFC 3339.
use chrono::{DateTime, TimeZone};
use std::borrow::Cow;

/// `at` in this machine's zone, to the minute, with the zone's short name.
pub fn stamp<Z: TimeZone>(at: &DateTime<Z>) -> String {
    let local = at.with_timezone(&chrono::Local);
    let zone =
        zone_abbreviation(local.timestamp()).unwrap_or_else(|| local.format("UTC%:z").to_string());
    format!("{} {zone}", local.format("%Y-%m-%d %H:%M"))
}

/// `text` with every RFC 3339 instant in it, such as
/// `2026-10-25T17:43:26.713335520-04:00`, rewritten as a [`stamp`]. Text
/// without a full instant, zone included, stays as it is.
pub fn readable(text: &str) -> Cow<'_, str> {
    let mut out = String::new();
    let mut copied = 0;
    let mut index = 0;
    let bytes = text.as_bytes();
    while index + 16 <= bytes.len() {
        match instant_at(text, index) {
            Some((end, at)) => {
                out.push_str(&text[copied..index]);
                out.push_str(&stamp(&at));
                copied = end;
                index = end;
            }
            None => index += 1,
        }
    }
    if copied == 0 {
        return Cow::Borrowed(text);
    }
    out.push_str(&text[copied..]);
    Cow::Owned(out)
}

/// The instant starting at byte `start`, and the byte after it.
fn instant_at(text: &str, start: usize) -> Option<(usize, DateTime<chrono::FixedOffset>)> {
    let bytes = text.as_bytes();
    // A digit before would make this the middle of a longer number.
    if start > 0 && bytes[start - 1].is_ascii_digit() {
        return None;
    }
    let shape = b"dddd-dd-ddTdd:dd:dd";
    if bytes.len() < start + shape.len()
        || !shape
            .iter()
            .zip(&bytes[start..])
            .all(|(want, got)| match want {
                b'd' => got.is_ascii_digit(),
                other => other == got,
            })
    {
        return None;
    }
    let mut end = start + shape.len();
    if bytes.get(end) == Some(&b'.') {
        end += 1;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
    }
    match bytes.get(end) {
        Some(b'Z') => end += 1,
        Some(b'+' | b'-') => end += 6,
        _ => return None,
    }
    let at = DateTime::parse_from_rfc3339(text.get(start..end)?).ok()?;
    Some((end, at))
}

/// The zone's short name at that instant, such as `EDT` or `UTC`; chrono
/// knows only the offset.
#[cfg(unix)]
fn zone_abbreviation(seconds: i64) -> Option<String> {
    let time = libc::time_t::try_from(seconds).ok()?;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&time, &mut tm) }.is_null() || tm.tm_zone.is_null() {
        return None;
    }
    let name = unsafe { std::ffi::CStr::from_ptr(tm.tm_zone) }
        .to_str()
        .ok()?;
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(not(unix))]
fn zone_abbreviation(_seconds: i64) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(text: &str) -> String {
        stamp(&DateTime::parse_from_rfc3339(text).unwrap())
    }

    #[test]
    fn stamp_reads_date_minutes_and_zone_name() {
        let at = DateTime::parse_from_rfc3339("2026-10-26T10:23:28Z").unwrap();
        let expected = at
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M ")
            .to_string();
        let stamp = stamp(&at);
        assert!(stamp.starts_with(&expected), "{stamp}");
        let zone = &stamp[expected.len()..];
        assert!(
            !zone.is_empty() && !zone.contains(' '),
            "zone should be one short name, got {stamp}"
        );
    }

    #[test]
    fn readable_rewrites_each_instant_in_text_and_leaves_the_rest() {
        let nanos = "2026-10-25T17:43:26.713335520-04:00";
        let utc = "2026-10-24T03:28:16Z";
        assert_eq!(
            readable(&format!("Started {nanos} · window resets at {utc}.")),
            format!(
                "Started {} · window resets at {}.",
                local(nanos),
                local(utc)
            )
        );
        assert_eq!(
            readable("closed at 2026-10-21T21:50:32+00:00"),
            format!("closed at {}", local("2026-10-21T21:50:32+00:00"))
        );
        for unchanged in [
            "pickup-20261026T061601-0400",
            "2026-10-26 07:09",
            "2026-10-26T07:09:00 without a zone",
            "12026-10-26T07:09:00Z",
            "",
        ] {
            assert!(
                matches!(readable(unchanged), Cow::Borrowed(_)),
                "{unchanged}"
            );
        }
    }
}
