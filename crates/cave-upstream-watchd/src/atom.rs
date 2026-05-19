// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Atom-feed release poller — primary release-detection path as of
//! 2026-05-19 (ADR-026).
//!
//! GitHub serves a per-repo Atom feed at
//! `https://github.com/<owner>/<repo>/releases.atom` that is publicly
//! readable with NO authentication, has its own generous unauthenticated
//! rate limit (separate from the JSON REST API's 60 req/h ceiling), and
//! supports `If-None-Match` + `If-Modified-Since` conditional caching.
//!
//! Trade-off vs. the REST API (`/repos/<repo>/releases/latest`):
//!
//! | Field                     | REST JSON | Atom feed                      |
//! | ------------------------- | --------- | ------------------------------ |
//! | tag_name                  | yes       | yes (`<title>`)                |
//! | published_at              | yes       | yes (`<updated>`)              |
//! | body (release notes)      | markdown  | escaped HTML in `<content>`    |
//! | html_url                  | yes       | yes (`<link href>`)            |
//! | prerelease flag           | yes       | NO (Atom does not surface it)  |
//! | asset URLs                | yes       | NO                             |
//! | rate limit                | 60 req/h  | unauthenticated, much higher   |
//! | requires PAT for prod use | YES       | NO                             |
//!
//! For the dispatcher's GAP_OPENED pipeline we need tag + published_at +
//! body — all three are present in Atom. The HTML body is converted to
//! plain text by [`html_to_text`] before being handed to the changelog
//! parser; that gives the same Added/Changed/Deprecated bucketing as the
//! REST path produces.
//!
//! Optional GitHub App enrichment (see [`crate::github_app`]) can flip
//! the daemon back to the REST JSON path on a per-tick basis when the
//! operator wants asset URLs or the prerelease flag for a downstream
//! consumer. The Atom path remains the default.

use crate::poller::{LatestRelease, PollError, PollOutcome};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::time::Duration;

/// Default Atom-feed base URL. Tests override this to point at a
/// httpmock server.
pub const DEFAULT_ATOM_BASE_URL: &str = "https://github.com";

/// Atom-feed-flavoured GitHub release client. Shares the
/// `reqwest::Client` shape with [`crate::poller::GitHubClient`] but
/// targets the `<owner>/<repo>/releases.atom` endpoint instead of the
/// JSON REST API. No bearer auth is sent — the feed is public.
#[derive(Debug, Clone)]
pub struct AtomClient {
    inner: reqwest::Client,
    base_url: String,
}

impl AtomClient {
    pub fn new() -> Self {
        Self::with_base_url(DEFAULT_ATOM_BASE_URL.to_string())
    }

    pub fn with_base_url(base_url: String) -> Self {
        let inner = reqwest::Client::builder()
            .user_agent(concat!("cave-upstream-watchd/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(20))
            .build()
            .expect("reqwest client");
        Self { inner, base_url }
    }

    /// Fetch `<repo>/releases.atom`. Same conditional-cache contract as
    /// [`crate::poller::GitHubClient::fetch_latest`] — pass the cached
    /// `etag` / `last_modified` and the server may reply 304.
    pub async fn fetch_latest_atom(
        &self,
        repo: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<PollOutcome, PollError> {
        let url = format!("{}/{repo}/releases.atom", self.base_url);
        let mut req = self.inner.get(&url);
        req = req.header("Accept", "application/atom+xml");
        if let Some(et) = etag {
            req = req.header("If-None-Match", et);
        }
        if let Some(lm) = last_modified {
            req = req.header("If-Modified-Since", lm);
        }
        let resp = req.send().await?;
        let status = resp.status();
        let headers = resp.headers().clone();

        // Atom feed does not surface X-RateLimit-Remaining. We report
        // None and leave throttling to the caller's tick budget.
        if status.as_u16() == 304 {
            return Ok(PollOutcome::NotModified {
                rate_limit_remaining: None,
            });
        }
        if status.as_u16() == 404 {
            return Ok(PollOutcome::NoRelease);
        }
        let etag_out = headers
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let last_modified_out = headers
            .get("last-modified")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = resp.error_for_status()?.text().await?;
        let Some(release) = parse_first_entry(&body) else {
            // Feed exists but has zero entries — repo with no releases.
            return Ok(PollOutcome::NoRelease);
        };
        Ok(PollOutcome::NewRelease {
            release,
            etag: etag_out,
            last_modified: last_modified_out,
            rate_limit_remaining: None,
        })
    }
}

impl Default for AtomClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Walk the Atom XML and return the first `<entry>` flattened into a
/// [`LatestRelease`]. Returns `None` if the feed contains no entries
/// (a fresh repo without any releases yet).
///
/// Atom entries look like:
/// ```xml
/// <entry>
///   <id>tag:github.com,2008:Repository/123/v1.2.3</id>
///   <updated>2026-05-19T00:48:51Z</updated>
///   <link rel="alternate" type="text/html"
///         href="https://github.com/org/repo/releases/tag/v1.2.3"/>
///   <title>v1.2.3</title>
///   <content type="html">&lt;h2&gt;What's changed&lt;/h2&gt; ...</content>
/// </entry>
/// ```
pub fn parse_first_entry(xml: &str) -> Option<LatestRelease> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut buf = Vec::new();
    let mut in_entry = false;
    let mut in_title = false;
    let mut in_updated = false;
    let mut in_content = false;

    let mut tag_name: Option<String> = None;
    let mut updated: Option<String> = None;
    let mut html_url: Option<String> = None;
    let mut content_html: Option<String> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = e.name();
                match name.as_ref() {
                    b"entry" => in_entry = true,
                    b"title" if in_entry => in_title = true,
                    b"updated" if in_entry => in_updated = true,
                    b"content" if in_entry => in_content = true,
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) if in_entry && e.name().as_ref() == b"link" => {
                // <link rel="alternate" type="text/html" href="..."/>
                for attr in e.attributes().with_checks(false).flatten() {
                    if attr.key.as_ref() == b"href" {
                        if let Ok(val) = attr.unescape_value() {
                            html_url = Some(val.into_owned());
                        }
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if in_entry {
                    let s = t.unescape().unwrap_or_default().into_owned();
                    if in_title {
                        tag_name = Some(s);
                    } else if in_updated {
                        updated = Some(s);
                    } else if in_content {
                        content_html = Some(s);
                    }
                }
            }
            Ok(Event::CData(c)) => {
                if in_entry && in_content {
                    content_html = Some(String::from_utf8_lossy(&c.into_inner()).into_owned());
                }
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                b"title" => in_title = false,
                b"updated" => in_updated = false,
                b"content" => in_content = false,
                b"entry" => {
                    // First entry done — stop walking.
                    break;
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    // Prefer the link's `/releases/tag/<tag>` segment as the
    // authoritative tag — GitHub guarantees that's the literal git
    // ref. `<title>` is often the release *name* (e.g. "Step CA
    // v0.30.2 (26-03-23)") which would break downstream semver
    // parsing. Fall back to `<title>` only when no link was present.
    let tag = html_url
        .as_deref()
        .and_then(extract_tag_from_release_url)
        .map(str::to_string)
        .or(tag_name.clone())?;
    let display_name = tag_name;
    let published_at = updated
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
        .map(|d| d.with_timezone(&chrono::Utc));
    let body = content_html.map(|h| html_to_text(&h));

    Some(LatestRelease {
        tag_name: tag,
        name: display_name,
        body,
        published_at,
        html_url,
        // Atom does NOT carry the prerelease flag. Default to false;
        // operators who care about prereleases should opt into the
        // REST path via the GitHub App enrichment.
        prerelease: false,
    })
}

/// Extract the git tag from a GitHub release URL of the form
/// `https://github.com/<owner>/<repo>/releases/tag/<tag>`. The tag
/// segment may contain dots, dashes, plus signs etc. — anything
/// path-safe.
pub fn extract_tag_from_release_url(url: &str) -> Option<&str> {
    let pos = url.find("/releases/tag/")?;
    let after = &url[pos + "/releases/tag/".len()..];
    // Strip any trailing query/fragment.
    let end = after.find(['?', '#']).unwrap_or(after.len());
    let tag = &after[..end];
    if tag.is_empty() {
        None
    } else {
        Some(tag)
    }
}

/// Minimal HTML → plain text. Strips tags, normalises common entities,
/// and turns `<li>` items into `- ` lines so the existing
/// [`crate::changelog::parse_release_body`] still sees an Added /
/// Changed / Deprecated structure.
///
/// Not a full HTML parser — enough for GitHub's release-note flavour.
/// If GitHub's renderer changes shape we have a single function to
/// tune rather than a sprawling sanitiser.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let bytes = html.as_bytes();
    let mut i = 0;
    let mut in_tag = false;
    let mut tag_start = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if !in_tag && b == b'<' {
            in_tag = true;
            tag_start = i + 1;
        } else if in_tag && b == b'>' {
            let tag = &html[tag_start..i];
            let tag_lower = tag.trim_start_matches('/').trim().to_ascii_lowercase();
            let tag_norm: String = tag_lower
                .chars()
                .take_while(|c| !c.is_whitespace())
                .collect();
            match tag_norm.as_str() {
                "li" if !tag.starts_with('/') => out.push_str("\n- "),
                "p" | "br" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                    if tag.starts_with('/') =>
                {
                    out.push('\n');
                }
                _ => {}
            }
            in_tag = false;
        } else if !in_tag {
            out.push(b as char);
        }
        i += 1;
    }
    // Decode the small set of HTML entities GitHub emits in release
    // bodies. A full entity table is a separate dep we don't need.
    let s = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    // Squeeze runs of blank lines.
    let mut squeezed = String::with_capacity(s.len());
    let mut blank = false;
    for line in s.lines() {
        let trimmed = line.trim_end();
        let is_blank = trimmed.is_empty();
        if is_blank && blank {
            continue;
        }
        squeezed.push_str(trimmed);
        squeezed.push('\n');
        blank = is_blank;
    }
    squeezed
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::{Method::GET, MockServer};

    fn atom_with_entry(tag: &str, updated: &str, body_html: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Release notes from x</title>
  <updated>{updated}</updated>
  <entry>
    <id>tag:github.com,2008:Repository/1/{tag}</id>
    <updated>{updated}</updated>
    <link rel="alternate" type="text/html" href="https://github.com/x/y/releases/tag/{tag}"/>
    <title>{tag}</title>
    <content type="html">{body_html}</content>
  </entry>
</feed>"#
        )
    }

    #[test]
    fn parse_first_entry_extracts_tag_and_published() {
        let xml = atom_with_entry("v1.2.3", "2026-05-19T00:48:51Z", "&lt;p&gt;hi&lt;/p&gt;");
        let r = parse_first_entry(&xml).unwrap();
        assert_eq!(r.tag_name, "v1.2.3");
        assert_eq!(
            r.html_url.as_deref(),
            Some("https://github.com/x/y/releases/tag/v1.2.3")
        );
        assert_eq!(
            r.published_at,
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-05-19T00:48:51Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc)
            )
        );
        assert!(r.body.unwrap().contains("hi"));
    }

    #[test]
    fn extract_tag_from_release_url_handles_release_path() {
        assert_eq!(
            extract_tag_from_release_url(
                "https://github.com/smallstep/certificates/releases/tag/v0.30.2"
            ),
            Some("v0.30.2")
        );
        assert_eq!(
            extract_tag_from_release_url(
                "https://github.com/kubernetes/kubernetes/releases/tag/v1.36.1"
            ),
            Some("v1.36.1")
        );
        assert_eq!(extract_tag_from_release_url("https://github.com/x/y"), None);
    }

    #[test]
    fn parse_first_entry_prefers_tag_from_link_over_release_title() {
        // smallstep's title is "Step CA v0.30.2 (26-03-23)" — human
        // text. The link's `/releases/tag/<tag>` segment carries the
        // canonical tag and must win.
        let xml = r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <id>tag:github.com,2008:Repository/1/v0.30.2</id>
    <updated>2026-03-23T12:00:00Z</updated>
    <link rel="alternate" type="text/html"
          href="https://github.com/smallstep/certificates/releases/tag/v0.30.2"/>
    <title>Step CA v0.30.2 (26-03-23)</title>
    <content type="html">hi</content>
  </entry>
</feed>"#;
        let r = parse_first_entry(xml).unwrap();
        assert_eq!(r.tag_name, "v0.30.2");
        assert_eq!(r.name.as_deref(), Some("Step CA v0.30.2 (26-03-23)"));
    }

    #[test]
    fn parse_first_entry_returns_none_for_empty_feed() {
        let xml = r#"<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom">
            <title>Empty</title></feed>"#;
        assert!(parse_first_entry(xml).is_none());
    }

    #[test]
    fn html_to_text_keeps_list_structure_for_changelog_parser() {
        // html_to_text receives REAL HTML (post quick-xml entity
        // decoding) — the `&lt;` in the Atom payload becomes `<`
        // before we get here. Test with concrete HTML, not the
        // double-escaped wire form.
        let html = "<h2>What&#39;s changed</h2>\
                    <ul><li>Added <code>foo</code></li>\
                    <li>Fixed bar</li></ul>";
        let txt = html_to_text(html);
        assert!(txt.contains("- Added foo"), "got: {txt}");
        assert!(txt.contains("- Fixed bar"), "got: {txt}");
    }

    #[test]
    fn html_to_text_decodes_common_entities() {
        let html = "AT&amp;T, &lt;tag&gt;, &quot;quoted&quot;, &#39;apos&#39;, &nbsp;";
        let txt = html_to_text(html);
        assert!(txt.contains("AT&T"));
        assert!(txt.contains("<tag>"));
        assert!(txt.contains("\"quoted\""));
        assert!(txt.contains("'apos'"));
    }

    #[tokio::test]
    async fn fetch_latest_atom_returns_new_release_on_200() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET).path("/etcd-io/etcd/releases.atom");
            then.status(200)
                .header("etag", "W/\"abc\"")
                .header("content-type", "application/atom+xml")
                .body(atom_with_entry(
                    "v3.5.13",
                    "2026-05-13T12:00:00Z",
                    "&lt;p&gt;changes&lt;/p&gt;",
                ));
        });
        let cli = AtomClient::with_base_url(server.base_url());
        let outcome = cli
            .fetch_latest_atom("etcd-io/etcd", None, None)
            .await
            .unwrap();
        m.assert();
        match outcome {
            PollOutcome::NewRelease { release, etag, .. } => {
                assert_eq!(release.tag_name, "v3.5.13");
                assert_eq!(etag.as_deref(), Some("W/\"abc\""));
            }
            other => panic!("expected NewRelease, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn fetch_latest_atom_returns_not_modified_on_304() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/etcd-io/etcd/releases.atom")
                .header("If-None-Match", "W/\"abc\"");
            then.status(304);
        });
        let cli = AtomClient::with_base_url(server.base_url());
        let outcome = cli
            .fetch_latest_atom("etcd-io/etcd", Some("W/\"abc\""), None)
            .await
            .unwrap();
        m.assert();
        assert!(matches!(outcome, PollOutcome::NotModified { .. }));
    }

    #[tokio::test]
    async fn fetch_latest_atom_returns_no_release_on_404() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET).path("/none/zero/releases.atom");
            then.status(404);
        });
        let cli = AtomClient::with_base_url(server.base_url());
        let outcome = cli
            .fetch_latest_atom("none/zero", None, None)
            .await
            .unwrap();
        m.assert();
        assert!(matches!(outcome, PollOutcome::NoRelease));
    }

    #[tokio::test]
    async fn fetch_latest_atom_returns_no_release_for_empty_feed() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET).path("/empty/feed/releases.atom");
            then.status(200)
                .header("content-type", "application/atom+xml")
                .body(
                    r#"<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom"><title>x</title></feed>"#,
                );
        });
        let cli = AtomClient::with_base_url(server.base_url());
        let outcome = cli
            .fetch_latest_atom("empty/feed", None, None)
            .await
            .unwrap();
        m.assert();
        assert!(matches!(outcome, PollOutcome::NoRelease));
    }
}
