// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Git URL provider normalization and permalink generation.
//!
//! Faithful Rust port of TruffleHog `pkg/giturl/giturl.go` (v3.63.7):
//! `determineProvider`, `NormalizeOrgRepoURL` (+ provider wrappers),
//! `GenerateLink`, and `UpdateLinkLineNumber`.
//!
//! These build provider-specific permalinks (GitHub/GitLab/Bitbucket/Azure,
//! plus GitHub Gists and on-prem instances) to the exact commit/file/line a
//! secret was found at — a concern distinct from cave-runtime's fetcher, which
//! clones/reads but does not craft per-provider source links for findings.

use regex::Regex;
use std::sync::OnceLock;

// Provider detection substrings — must mirror upstream's url* consts exactly
// (note the trailing slash, which guards against e.g. "github.community").
const URL_GITHUB: &str = "github.com/";
const URL_GITLAB: &str = "gitlab.com/";
const URL_BITBUCKET: &str = "bitbucket.org/";
const URL_AZURE: &str = "dev.azure.com/";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    Github,
    Gitlab,
    Bitbucket,
    Azure,
    /// On-prem / unknown — treated like GitHub/GitLab for link shape.
    Unknown,
}

fn determine_provider(repo: &str) -> Provider {
    if repo.contains(URL_GITHUB) {
        Provider::Github
    } else if repo.contains(URL_GITLAB) {
        Provider::Gitlab
    } else if repo.contains(URL_BITBUCKET) {
        Provider::Bitbucket
    } else if repo.contains(URL_AZURE) {
        Provider::Azure
    } else {
        Provider::Unknown
    }
}

/// Strip the trailing `.git` (upstream `repo[:len(repo)-4]`). Returns the input
/// unchanged when it is shorter than the suffix (defensive — upstream assumes
/// the suffix is present).
fn strip_git(repo: &str) -> &str {
    if repo.len() >= 4 {
        &repo[..repo.len() - 4]
    } else {
        repo
    }
}

/// `GenerateLink` — craft a link to a specific file at a commit.
/// Supports GitHub, GitLab, Bitbucket, Azure Repos, GitHub Gists and on-prem
/// GitHub/GitLab. When the provider supports line anchors, `line` (> 0) is
/// included.
pub fn generate_link(repo: &str, commit: &str, file: &str, line: i64) -> String {
    // Some paths contain '%' which breaks URL parsing if not encoded.
    let file = file.replace('%', "%25");

    match determine_provider(repo) {
        Provider::Bitbucket => format!("{}/commits/{}", strip_git(repo), commit),

        Provider::Azure => {
            let mut base = format!("{}/commit/{}/{}", repo, commit, file);
            if line > 0 {
                base.push_str(&format!("?line={}", line));
            }
            base
        }

        // github/gitlab/on-prem all share the same shape.
        Provider::Github | Provider::Gitlab | Provider::Unknown => {
            // Gist links are formatted differently.
            if repo.starts_with("https://gist.github.com") {
                let mut base = format!("{}/", strip_git(repo));
                if !commit.is_empty() {
                    base.push_str(&format!("{}/", commit));
                }
                if !file.is_empty() {
                    let cleaned = file.replace('.', "-");
                    base.push_str(&format!("#file-{}", cleaned));
                }
                if line > 0 {
                    if base.contains('#') {
                        base.push_str(&format!("-L{}", line));
                    } else {
                        base.push_str(&format!("#L{}", line));
                    }
                }
                base
            } else if file.is_empty() {
                format!("{}/commit/{}", strip_git(repo), commit)
            } else {
                let mut base = format!("{}/blob/{}/{}", strip_git(repo), commit, file);
                if line > 0 {
                    base.push_str(&format!("#L{}", line));
                }
                base
            }
        }
    }
}

fn line_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"L\d+").unwrap())
}

/// `UpdateLinkLineNumber` — rewrite the line number in an existing link. Used
/// post-generation to refine the reported location within a scanned block.
/// Returns the link unchanged when `new_line <= 0`, for Bitbucket (no line
/// support), or when the link does not parse as a URL.
pub fn update_link_line_number(link: &str, new_line: i64) -> String {
    // Mirror the url.Parse error path: a string that isn't a URL is returned
    // unchanged (upstream logs the parse error and returns the link).
    if !link.contains("://") {
        return link.to_string();
    }
    if new_line <= 0 {
        return link.to_string();
    }

    match determine_provider(link) {
        Provider::Bitbucket => link.to_string(),

        Provider::Azure => {
            // Line numbers are the `?line=<n>` query parameter. Upstream uses
            // url.Values.Encode(), which alpha-sorts keys; we replace/insert the
            // `line` param and re-encode in sorted order.
            let (base, query) = match link.split_once('?') {
                Some((b, q)) => (b, q),
                None => (link, ""),
            };
            let mut pairs: Vec<(String, String)> = query
                .split('&')
                .filter(|s| !s.is_empty())
                .map(|kv| match kv.split_once('=') {
                    Some((k, v)) => (k.to_string(), v.to_string()),
                    None => (kv.to_string(), String::new()),
                })
                .filter(|(k, _)| k != "line")
                .collect();
            pairs.push(("line".to_string(), new_line.to_string()));
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            let encoded = pairs
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join("&");
            format!("{}?{}", base, encoded)
        }

        // github/gitlab/on-prem: rewrite the `#L<n>` fragment.
        Provider::Github | Provider::Gitlab | Provider::Unknown => {
            let (base, frag) = match link.split_once('#') {
                Some((b, f)) => (b, f),
                None => (link, ""),
            };
            let want = format!("L{}", new_line);
            let new_frag = if line_pattern().is_match(frag) {
                line_pattern().replace_all(frag, want.as_str()).into_owned()
            } else {
                format!("{}{}", frag, want)
            };
            format!("{}#{}", base, new_frag)
        }
    }
}

// ── Repo URL normalization ───────────────────────────────────────────────────

/// `NormalizeOrgRepoURL` — normalize an `example.com/org/repo` style URL to its
/// `.git` form. Returns the input unchanged when it already ends in `.git`.
/// `provider` is the display name used in error messages (e.g. "Github").
pub fn normalize_org_repo_url(provider: &str, repo_url: &str) -> Result<String, String> {
    if repo_url.ends_with(".git") {
        return Ok(repo_url.to_string());
    }

    let (scheme_host, path) = split_scheme_host_path(repo_url);

    // The provider repo url should have a path of 3 segments when split on '/':
    //   "" / org / repo   (from "/org/repo")
    let parts: Vec<&str> = path.split('/').collect();
    match parts.len() {
        n if n <= 1 => {
            return Err(format!(
                "{} repo appears to be missing the path. Repo url: {:?}",
                provider, repo_url
            ));
        }
        2 => {
            let org = parts[1];
            if org.is_empty() {
                return Err(format!(
                    "{} repo appears to be missing the org name. Repo url: {:?}",
                    provider, repo_url
                ));
            } else {
                return Err(format!(
                    "{} repo appears to be missing the repo name. Org: {:?} Repo url: {:?}",
                    provider, org, repo_url
                ));
            }
        }
        3 => {
            let (org, repo) = (parts[1], parts[2]);
            if org.is_empty() {
                return Err(format!(
                    "{} repo appears to be missing the org name. Repo url: {:?}",
                    provider, repo_url
                ));
            }
            if repo.is_empty() {
                return Err(format!(
                    "{} repo appears to be missing the repo name. Org: {:?} Repo url: {:?}",
                    provider, org, repo_url
                ));
            }
        }
        _ => {
            if path.ends_with('/') {
                return Err(format!(
                    "{} repo contains a trailing slash. Repo url: {:?}",
                    provider, repo_url
                ));
            }
        }
    }

    // Probably a provider repo missing ".git"; append it.
    Ok(format!("{}{}.git", scheme_host, path))
}

/// Split a URL into `(scheme://host, path)`. `path` keeps its leading slash and
/// is empty when there is no path (mirroring Go's `url.Parse(...).Path`).
fn split_scheme_host_path(url: &str) -> (&str, &str) {
    let Some(after_scheme_idx) = url.find("://") else {
        return (url, "");
    };
    let host_start = after_scheme_idx + 3;
    match url[host_start..].find('/') {
        Some(rel) => {
            let split = host_start + rel;
            (&url[..split], &url[split..])
        }
        None => (url, ""),
    }
}

pub fn normalize_bitbucket_repo(repo_url: &str) -> Result<String, String> {
    if !repo_url.starts_with("https") {
        return Err(
            "Bitbucket requires https repo urls: e.g. https://bitbucket.org/org/repo.git"
                .to_string(),
        );
    }
    normalize_org_repo_url("Bitbucket", repo_url)
}

pub fn normalize_github_repo(repo_url: &str) -> Result<String, String> {
    normalize_org_repo_url("Github", repo_url)
}

pub fn normalize_gitlab_repo(repo_url: &str) -> Result<String, String> {
    if !repo_url.starts_with("http:") && !repo_url.starts_with("https:") {
        return Err(
            "Gitlab requires http/https repo urls: e.g. https://gitlab.com/org/repo.git"
                .to_string(),
        );
    }
    normalize_org_repo_url("Gitlab", repo_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn determine_provider_uses_trailing_slash_guard() {
        assert_eq!(determine_provider("https://github.com/a/b"), Provider::Github);
        // "github.community" must not match the "github.com/" substring.
        assert_eq!(
            determine_provider("https://github.community/a/b"),
            Provider::Unknown
        );
    }

    #[test]
    fn strip_git_is_safe_for_short_inputs() {
        assert_eq!(strip_git("abc"), "abc");
        assert_eq!(strip_git("repo.git"), "repo");
    }
}
