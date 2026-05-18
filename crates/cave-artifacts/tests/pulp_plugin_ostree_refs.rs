// SPDX-License-Identifier: AGPL-3.0-or-later
//! RED tests for the NEW pulp_ostree content plugin.

use cave_artifacts::pulp::models::PluginType;
use cave_artifacts::pulp::plugin::ArtifactsPlugin;
use cave_artifacts::pulp::plugins::ostree::{
    parse_ostree_config, parse_ostree_ref, OstreeConfig, OstreePlugin, OstreeRef,
    OstreeRepoMode,
};

#[test]
fn parse_ref_text_format() {
    // OSTree refs are 64-hex commit checksums in a plain-text file.
    let body = "be0b1c5a8e6c2d4f6b8c0e2f4d6c8e0a2f4b6d8e0c2a4f6b8d0e2c4a6f8b0d2e\n";
    let r: OstreeRef = parse_ostree_ref(body).unwrap();
    assert_eq!(r.commit_checksum.len(), 64);
    assert!(r.commit_checksum.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn parse_ref_rejects_garbage() {
    assert!(parse_ostree_ref("not a checksum").is_err());
    assert!(parse_ostree_ref("").is_err());
    assert!(parse_ostree_ref("abcdef\n").is_err()); // too short
}

#[test]
fn parse_config_archive_z2() {
    let body = "\
[core]
repo_version=1
mode=archive-z2

[remote \"upstream\"]
url=https://ostree.fedoraproject.org/
gpg-verify=true
";
    let c: OstreeConfig = parse_ostree_config(body).unwrap();
    assert_eq!(c.repo_version, Some(1));
    assert_eq!(c.mode, OstreeRepoMode::ArchiveZ2);
    assert_eq!(c.remotes.len(), 1);
    assert_eq!(c.remotes["upstream"].url.as_deref(), Some("https://ostree.fedoraproject.org/"));
    assert_eq!(c.remotes["upstream"].gpg_verify, Some(true));
}

#[test]
fn parse_config_bare_modes() {
    let bare = "[core]\nmode=bare\n";
    assert_eq!(parse_ostree_config(bare).unwrap().mode, OstreeRepoMode::Bare);
    let bare_user = "[core]\nmode=bare-user\n";
    assert_eq!(parse_ostree_config(bare_user).unwrap().mode, OstreeRepoMode::BareUser);
}

#[test]
fn parse_config_rejects_unknown_mode() {
    assert!(parse_ostree_config("[core]\nmode=quantum\n").is_err());
}

#[test]
fn ostree_plugin_basics() {
    let plugin = OstreePlugin;
    assert_eq!(plugin.name(), "pulp_ostree");
    assert_eq!(plugin.plugin_type(), PluginType::Ostree);
}
