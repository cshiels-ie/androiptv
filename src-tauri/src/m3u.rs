//! Hand-rolled, streaming M3U/M3U8 IPTV playlist parser.
//!
//! This parses *channel* playlists (`#EXTINF:` entries pointing at stream
//! URLs) — not HLS media playlists (`#EXT-X-*` segment lists), which is why
//! the `m3u8-rs` crate does not fit here. The file is consumed one line at
//! a time via `BufRead`, so arbitrarily large playlists can be imported
//! without ever holding the whole file in memory.

use std::collections::HashMap;
use std::io::BufRead;
use std::sync::OnceLock;

use regex::Regex;

/// A single channel parsed from the playlist, ready for DB import.
#[derive(Debug, Clone, Default)]
pub struct ParsedChannel {
    pub name: String,
    pub url: String,
    pub logo_url: Option<String>,
    pub tvg_id: Option<String>,
    pub tvg_chno: Option<i64>,
    pub group: Option<String>,
}

/// Cached regex for `key="value"` attribute pairs.
fn attr_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"([A-Za-z0-9_-]+)="([^"]*)""#).expect("static attribute regex"))
}

/// Split the `#EXTINF` body at the first comma that is *not* inside a quoted
/// attribute value, returning (attribute list, display name).
fn split_first_unquoted_comma(s: &str) -> (&str, &str) {
    let mut in_quotes = false;
    for (i, ch) in s.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => return (&s[..i], &s[i + 1..]),
            _ => {}
        }
    }
    // No unquoted comma: everything is an attribute, no display name.
    (s, "")
}

/// Attributes + display name extracted from a single `#EXTINF:` line.
struct Extinf {
    display_name: String,
    attrs: HashMap<String, String>,
}

fn parse_extinf(body: &str) -> Extinf {
    let (attr_part, name_part) = split_first_unquoted_comma(body);
    let mut attrs = HashMap::new();
    for caps in attr_regex().captures_iter(attr_part) {
        // Keys are lowercased so `TVG-ID=` and `tvg-id=` behave identically.
        attrs.insert(caps[1].to_lowercase(), caps[2].to_string());
    }
    Extinf {
        display_name: name_part.trim().to_string(),
        attrs,
    }
}

/// Parse an M3U/M3U8 IPTV playlist, feeding every channel to `sink` as it
/// is discovered. Returns the number of channels emitted.
///
/// Semantics: the `#EXTM3U` header and other comment lines (`#EXTGRP`,
/// `#PLAYLIST`, `#KODIPROP`, ...) are ignored. An `#EXTINF:` line starts a
/// pending channel; the next non-comment, non-empty line is its URL. A
/// dangling `#EXTINF:` at the end of the file is dropped. `BufRead::lines()`
/// strips both `\n` and `\r\n` line endings.
pub fn parse_m3u<R: BufRead>(
    reader: R,
    mut sink: impl FnMut(ParsedChannel),
) -> Result<usize, String> {
    let mut pending: Option<Extinf> = None;
    let mut count = 0usize;

    for line in reader.lines() {
        let line = line.map_err(|e| format!("error reading playlist: {e}"))?;
        if line.is_empty() {
            continue;
        }
        if let Some(body) = line.strip_prefix("#EXTINF:") {
            // Starts a pending channel; the URL comes on a later line.
            pending = Some(parse_extinf(body));
            continue;
        }
        if line.starts_with('#') {
            // #EXTM3U header, #EXTGRP, #KODIPROP, ... — all ignored.
            continue;
        }
        // A non-comment, non-empty line is the stream URL of the pending
        // channel; comments and blank lines in between are skipped.
        let Some(extinf) = pending.take() else {
            continue; // stray URL with no preceding EXTINF — ignore
        };
        let url = line.trim().to_string();
        if url.is_empty() {
            continue;
        }

        // Display name: tvg-name attribute, else the text after the comma,
        // else a fallback.
        let tvg_name = extinf
            .attrs
            .get("tvg-name")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let name = tvg_name.unwrap_or_else(|| {
            if extinf.display_name.is_empty() {
                "Unknown".to_string()
            } else {
                extinf.display_name.clone()
            }
        });
        let tvg_id = extinf.attrs.get("tvg-id").filter(|v| !v.is_empty()).cloned();
        let tvg_chno = extinf
            .attrs
            .get("tvg-chno")
            .and_then(|v| v.parse::<i64>().ok());
        let group = extinf
            .attrs
            .get("group-title")
            .filter(|v| !v.is_empty())
            .cloned();
        let logo_url = extinf
            .attrs
            .get("tvg-logo")
            .filter(|v| !v.is_empty())
            .cloned();

        sink(ParsedChannel {
            name,
            url,
            logo_url,
            tvg_id,
            tvg_chno,
            group,
        });
        count += 1;
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_channel(
        got: &ParsedChannel,
        name: &str,
        url: &str,
        logo_url: Option<&str>,
        tvg_id: Option<&str>,
        tvg_chno: Option<i64>,
        group: Option<&str>,
    ) {
        assert_eq!(got.name, name);
        assert_eq!(got.url, url);
        assert_eq!(got.logo_url.as_deref(), logo_url);
        assert_eq!(got.tvg_id.as_deref(), tvg_id);
        assert_eq!(got.tvg_chno, tvg_chno);
        assert_eq!(got.group.as_deref(), group);
    }

    #[test]
    fn parses_crlf_playlist_with_quoted_commas() {
        // CRLF line endings; commas inside quoted attribute values; empty
        // tvg-id/group-title; unparsable tvg-chno; comment lines between
        // EXTINF and its URL; dangling EXTINF at EOF.
        let fixture = [
            "#EXTM3U",
            "#EXTINF:-1 tvg-id=\"cnn-intl\" tvg-name=\"CNN US\" tvg-logo=\"http://cdn.example/logo.png\" group-title=\"News, US & World\" tvg-chno=\"7\",CNN HD",
            "http://stream.example/cnn.ts",
            "#EXTINF:-1 tvg-id=\"\" tvg-name=\"\" group-title=\"Movies, Action\",Action Movies",
            "http://stream.example/movies.ts",
            "#EXTGRP:Movies",
            "#PLAYLIST",
            "#EXTINF:-1 tvg-id=\"bbc1\" group-title=\"\" tvg-chno=\"not-a-number\",Free BBC",
            "#EXTGRP:UK",
            "http://stream.example/bbc.ts",
            "#EXTINF:-1 tvg-id=\"dangling\" group-title=\"Lost\",Dangling at EOF",
        ]
        .join("\r\n");

        // Verify the CRLF-stripping claim about BufRead::lines() directly.
        let mut lines = std::io::BufReader::new(fixture.as_bytes()).lines();
        assert_eq!(lines.next().unwrap().unwrap(), "#EXTM3U");

        let mut got: Vec<ParsedChannel> = Vec::new();
        let count = parse_m3u(fixture.as_bytes(), |ch| got.push(ch)).unwrap();

        assert_eq!(count, 3);
        assert_eq!(got.len(), 3);
        assert_channel(
            &got[0],
            "CNN US",
            "http://stream.example/cnn.ts",
            Some("http://cdn.example/logo.png"),
            Some("cnn-intl"),
            Some(7),
            Some("News, US & World"),
        );
        assert_channel(
            &got[1],
            "Action Movies",
            "http://stream.example/movies.ts",
            None,
            None,
            None,
            Some("Movies, Action"),
        );
        assert_channel(
            &got[2],
            "Free BBC",
            "http://stream.example/bbc.ts",
            None,
            Some("bbc1"),
            None,
            None,
        );
    }

    #[test]
    fn parses_100k_channels_under_10s() {
        // Build a 100k-channel playlist in memory, reusing the same base
        // entry template with the channel index interpolated.
        let mut playlist = String::with_capacity(100_000 * 110);
        for i in 0..100_000 {
            playlist.push_str(&format!(
                "#EXTINF:-1 tvg-id=\"id{0}\" tvg-name=\"Channel {0}\" group-title=\"Group {1}\",Channel {0}\r\nhttp://example.com/{0}.ts\r\n",
                i,
                i % 10
            ));
        }

        let start = std::time::Instant::now();
        let count = parse_m3u(playlist.as_bytes(), |_| {}).unwrap();
        let elapsed = start.elapsed();
        eprintln!(
            "parsed 100k channels from {} lines in {elapsed:?}",
            100_000 * 2
        );
        assert_eq!(count, 100_000);
        assert!(
            elapsed.as_secs_f64() < 10.0,
            "100k-channel parse took {elapsed:?}"
        );
    }
}
