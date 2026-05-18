// SPDX-License-Identifier: AGPL-3.0-or-later
//! RED tests for pulp_python PEP 440 version parser, PEP 503 normalization,
//! and METADATA / PKG-INFO field extraction.

use cave_artifacts::pulp::plugins::python::{
    normalize_pep503, parse_metadata_fields, parse_pep440, Pep440Version, PreRelease,
};

#[test]
fn pep440_simple() {
    let v = parse_pep440("1.2.3").unwrap();
    assert_eq!(v.epoch, 0);
    assert_eq!(v.release, vec![1, 2, 3]);
    assert!(v.pre.is_none());
    assert_eq!(v.post, None);
    assert_eq!(v.dev, None);
    assert!(v.local.is_none());
}

#[test]
fn pep440_with_epoch_pre_post_dev_local() {
    let v = parse_pep440("2!1.0.0a1.post2.dev3+ubuntu.20.04").unwrap();
    assert_eq!(v.epoch, 2);
    assert_eq!(v.release, vec![1, 0, 0]);
    assert_eq!(v.pre, Some(PreRelease::Alpha(1)));
    assert_eq!(v.post, Some(2));
    assert_eq!(v.dev, Some(3));
    assert_eq!(v.local.as_deref(), Some("ubuntu.20.04"));
}

#[test]
fn pep440_pre_release_normalization() {
    // PEP 440 §normalization: rc / pre / preview all → "rc",
    // alpha → "a", beta → "b", c → "rc".
    assert_eq!(parse_pep440("1.0a1").unwrap().pre, Some(PreRelease::Alpha(1)));
    assert_eq!(parse_pep440("1.0.alpha.1").unwrap().pre, Some(PreRelease::Alpha(1)));
    assert_eq!(parse_pep440("1.0b2").unwrap().pre, Some(PreRelease::Beta(2)));
    assert_eq!(parse_pep440("1.0rc3").unwrap().pre, Some(PreRelease::Rc(3)));
    assert_eq!(parse_pep440("1.0c4").unwrap().pre, Some(PreRelease::Rc(4)));
}

#[test]
fn pep440_rejects_junk() {
    assert!(parse_pep440("not-a-version").is_err());
    assert!(parse_pep440("").is_err());
    assert!(parse_pep440("1.0.0bogus").is_err());
}

#[test]
fn pep440_ordering_release() {
    // PEP 440: 1.0 < 1.0.1 < 1.1 < 2.0
    let a = parse_pep440("1.0").unwrap();
    let b = parse_pep440("1.0.1").unwrap();
    let c = parse_pep440("1.1").unwrap();
    let d = parse_pep440("2.0").unwrap();
    assert!(a < b);
    assert!(b < c);
    assert!(c < d);
}

#[test]
fn pep440_ordering_pre_lt_release() {
    // PEP 440: pre-release < final, dev < pre < final < post
    let dev = parse_pep440("1.0.dev1").unwrap();
    let pre = parse_pep440("1.0a1").unwrap();
    let rel = parse_pep440("1.0").unwrap();
    let post = parse_pep440("1.0.post1").unwrap();
    assert!(dev < pre);
    assert!(pre < rel);
    assert!(rel < post);
}

#[test]
fn pep503_normalization() {
    // PEP 503: lowercase + replace runs of [-_.]+ with single '-'.
    assert_eq!(normalize_pep503("Django"), "django");
    assert_eq!(normalize_pep503("ZopE_Interface"), "zope-interface");
    assert_eq!(normalize_pep503("foo..bar___baz"), "foo-bar-baz");
    assert_eq!(normalize_pep503("Already-Normalized"), "already-normalized");
}

#[test]
fn parse_metadata_basic_fields() {
    let raw = "\
Metadata-Version: 2.1
Name: my-package
Version: 1.2.3
Summary: A test package
Home-page: https://example.com
Author: Alice
License: AGPL-3.0-or-later
Requires-Python: >=3.9
Requires-Dist: requests>=2.0
Requires-Dist: pyyaml<7

This is the description body, separated by a blank line.
It can span multiple paragraphs.
";
    let m = parse_metadata_fields(raw).unwrap();
    assert_eq!(m.name.as_deref(), Some("my-package"));
    assert_eq!(m.version.as_deref(), Some("1.2.3"));
    assert_eq!(m.summary.as_deref(), Some("A test package"));
    assert_eq!(m.requires_python.as_deref(), Some(">=3.9"));
    assert_eq!(m.requires_dist, vec!["requests>=2.0", "pyyaml<7"]);
    assert!(m.description.as_deref().unwrap().contains("description body"));
}

#[test]
fn parse_metadata_handles_continuation_lines() {
    // RFC822 continuation: leading whitespace continues previous field.
    let raw = "\
Metadata-Version: 2.1
Name: x
Version: 1.0
Summary: First line
 continuation
\tsecond
";
    let m = parse_metadata_fields(raw).unwrap();
    assert_eq!(
        m.summary.as_deref(),
        Some("First line\ncontinuation\nsecond")
    );
}
