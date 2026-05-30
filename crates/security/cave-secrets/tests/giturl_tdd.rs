// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD parity port of TruffleHog `pkg/giturl/giturl.go` (v3.63.7).
//!
//! Vectors copied verbatim from upstream `pkg/giturl/giturl_test.go`
//! (TestGenerateLink / TestUpdateLinkLineNumber / Test_NormalizeOrgRepoURL).
//! RED first: `cave_secrets::giturl` does not exist yet.

use cave_secrets::giturl;

// ── GenerateLink ─────────────────────────────────────────────────────────────

#[test]
fn github_link_gen() {
    assert_eq!(
        giturl::generate_link(
            "https://github.com/trufflesec-julian/confluence-go-api.git",
            "047b4a2ba42fc5b6c0bd535c5307434a666db5ec",
            ".gitignore",
            0,
        ),
        "https://github.com/trufflesec-julian/confluence-go-api/blob/047b4a2ba42fc5b6c0bd535c5307434a666db5ec/.gitignore"
    );
}

#[test]
fn github_link_gen_with_line() {
    assert_eq!(
        giturl::generate_link(
            "https://github.com/trufflesec-julian/confluence-go-api.git",
            "047b4a2ba42fc5b6c0bd535c5307434a666db5ec",
            ".gitignore",
            4,
        ),
        "https://github.com/trufflesec-julian/confluence-go-api/blob/047b4a2ba42fc5b6c0bd535c5307434a666db5ec/.gitignore#L4"
    );
}

#[test]
fn github_link_gen_no_file() {
    assert_eq!(
        giturl::generate_link(
            "https://github.com/trufflesec-julian/confluence-go-api.git",
            "047b4a2ba42fc5b6c0bd535c5307434a666db5ec",
            "",
            0,
        ),
        "https://github.com/trufflesec-julian/confluence-go-api/commit/047b4a2ba42fc5b6c0bd535c5307434a666db5ec"
    );
}

#[test]
fn azure_link_gen() {
    assert_eq!(
        giturl::generate_link(
            "https://dev.azure.com/org/project/_git/repo",
            "abcdef",
            "main.go",
            0,
        ),
        "https://dev.azure.com/org/project/_git/repo/commit/abcdef/main.go"
    );
}

#[test]
fn azure_link_gen_with_line() {
    assert_eq!(
        giturl::generate_link(
            "https://dev.azure.com/org/project/_git/repo",
            "abcdef",
            "main.go",
            20,
        ),
        "https://dev.azure.com/org/project/_git/repo/commit/abcdef/main.go?line=20"
    );
}

#[test]
fn bitbucket_link_gen_no_line_support() {
    // Bitbucket: repo[:len-4] + "/commits/" + commit (no file/line)
    assert_eq!(
        giturl::generate_link(
            "https://bitbucket.org/org/repo.git",
            "xyz123",
            "main.go",
            30,
        ),
        "https://bitbucket.org/org/repo/commits/xyz123"
    );
}

#[test]
fn unknown_provider_onprem() {
    assert_eq!(
        giturl::generate_link(
            "https://onprem.customdomain.com/org/repo.git",
            "xyz123",
            "main.go",
            30,
        ),
        "https://onprem.customdomain.com/org/repo/blob/xyz123/main.go#L30"
    );
}

#[test]
fn unknown_provider_onprem_no_file() {
    assert_eq!(
        giturl::generate_link(
            "https://onprem.customdomain.com/org/repo.git",
            "xyz123",
            "",
            0,
        ),
        "https://onprem.customdomain.com/org/repo/commit/xyz123"
    );
}

#[test]
fn gist_link_gen() {
    assert_eq!(
        giturl::generate_link(
            "https://gist.github.com/joeleonjr/be68e34b002e236160dbb394bbda86fb.git",
            "e94c5a1d5607e68f1cae4962bc4dce5de522371b",
            "test",
            4,
        ),
        "https://gist.github.com/joeleonjr/be68e34b002e236160dbb394bbda86fb/e94c5a1d5607e68f1cae4962bc4dce5de522371b/#file-test-L4"
    );
}

#[test]
fn gist_link_gen_multiple_extensions() {
    assert_eq!(
        giturl::generate_link(
            "https://gist.github.com/joeleonjr/be68e34b002e236160dbb394bbda86fb.git",
            "c64bf2345256cca7d2621f9cb78401e8860f82c8",
            "test.txt.ps1",
            4,
        ),
        "https://gist.github.com/joeleonjr/be68e34b002e236160dbb394bbda86fb/c64bf2345256cca7d2621f9cb78401e8860f82c8/#file-test-txt-ps1-L4"
    );
}

#[test]
fn link_gen_file_percent_in_path_is_encoded() {
    assert_eq!(
        giturl::generate_link(
            "https://github.com/GeekMasher/tree-sitter-hcl.git",
            "a7f23cc5795769262f5515e52902f86c1b768994",
            "example/real_world_stuff/coreos/coreos%tectonic-installer%installer%frontend%ui-tests%output%metal.tfvars",
            1,
        ),
        "https://github.com/GeekMasher/tree-sitter-hcl/blob/a7f23cc5795769262f5515e52902f86c1b768994/example/real_world_stuff/coreos/coreos%25tectonic-installer%25installer%25frontend%25ui-tests%25output%25metal.tfvars#L1"
    );
}

// ── UpdateLinkLineNumber ─────────────────────────────────────────────────────

#[test]
fn update_bitbucket_no_line() {
    assert_eq!(
        giturl::update_link_line_number("https://bitbucket.org/org/repo/blob/xyz123/main.go", 10),
        "https://bitbucket.org/org/repo/blob/xyz123/main.go"
    );
}

#[test]
fn update_github_link_with_line() {
    assert_eq!(
        giturl::update_link_line_number(
            "https://github.com/trufflesec-julian/confluence-go-api/blob/047b4a2ba42fc5b6c0bd535c5307434a666db5ec/.gitignore#L4",
            10,
        ),
        "https://github.com/trufflesec-julian/confluence-go-api/blob/047b4a2ba42fc5b6c0bd535c5307434a666db5ec/.gitignore#L10"
    );
}

#[test]
fn update_azure_link_with_line() {
    assert_eq!(
        giturl::update_link_line_number(
            "https://dev.azure.com/org/project/_git/repo/commit/abcdef/main.go?line=20",
            40,
        ),
        "https://dev.azure.com/org/project/_git/repo/commit/abcdef/main.go?line=40"
    );
}

#[test]
fn add_line_to_github_link_without_line() {
    assert_eq!(
        giturl::update_link_line_number(
            "https://github.com/trufflesec-julian/confluence-go-api/blob/047b4a2ba42fc5b6c0bd535c5307434a666db5ec/.gitignore",
            7,
        ),
        "https://github.com/trufflesec-julian/confluence-go-api/blob/047b4a2ba42fc5b6c0bd535c5307434a666db5ec/.gitignore#L7"
    );
}

#[test]
fn update_onprem_with_line() {
    assert_eq!(
        giturl::update_link_line_number(
            "https://onprem.customdomain.com/org/repo/blob/xyz123/main.go#L30",
            50,
        ),
        "https://onprem.customdomain.com/org/repo/blob/xyz123/main.go#L50"
    );
}

#[test]
fn update_onprem_without_line() {
    assert_eq!(
        giturl::update_link_line_number(
            "https://onprem.customdomain.com/org/repo/commit/xyz123",
            50,
        ),
        "https://onprem.customdomain.com/org/repo/commit/xyz123#L50"
    );
}

#[test]
fn dont_change_when_line_is_zero() {
    assert_eq!(
        giturl::update_link_line_number("https://github.com/coinbase/cbpay-js/issues/181", 0),
        "https://github.com/coinbase/cbpay-js/issues/181"
    );
}

#[test]
fn unparseable_link_returned_unchanged() {
    // Upstream marks this wantErr (url.Parse fails) and skips the comparison;
    // we return the input unchanged when it isn't a URL.
    assert_eq!(
        giturl::update_link_line_number("definitely not a link", 50),
        "definitely not a link"
    );
}

// ── NormalizeOrgRepoURL & provider wrappers ──────────────────────────────────

#[test]
fn normalize_github_repo_good() {
    assert_eq!(
        giturl::normalize_github_repo("https://github.com/org/repo").unwrap(),
        "https://github.com/org/repo.git"
    );
}

#[test]
fn normalize_org_repo_already_git_is_noop() {
    assert_eq!(
        giturl::normalize_org_repo_url("Github", "https://github.com/org/repo.git").unwrap(),
        "https://github.com/org/repo.git"
    );
}

#[test]
fn normalize_org_repo_missing_repo_name() {
    let err = giturl::normalize_org_repo_url("example", "https://example.com/org").unwrap_err();
    assert_eq!(
        err,
        "example repo appears to be missing the repo name. Org: \"org\" Repo url: \"https://example.com/org\""
    );
}

#[test]
fn normalize_org_repo_missing_path() {
    let err =
        giturl::normalize_org_repo_url("Github", "https://github.com").unwrap_err();
    assert_eq!(
        err,
        "Github repo appears to be missing the path. Repo url: \"https://github.com\""
    );
}

#[test]
fn normalize_org_repo_two_slashes_missing_org() {
    let err = giturl::normalize_org_repo_url("Github", "https://github.com//").unwrap_err();
    assert_eq!(
        err,
        "Github repo appears to be missing the org name. Repo url: \"https://github.com//\""
    );
}

#[test]
fn normalize_org_repo_trailing_slash() {
    let err =
        giturl::normalize_org_repo_url("Github", "https://github.com/org/repo/").unwrap_err();
    assert_eq!(
        err,
        "Github repo contains a trailing slash. Repo url: \"https://github.com/org/repo/\""
    );
}

#[test]
fn normalize_bitbucket_requires_https() {
    let err = giturl::normalize_bitbucket_repo("git@bitbucket.org:org/repo.git").unwrap_err();
    assert_eq!(
        err,
        "Bitbucket requires https repo urls: e.g. https://bitbucket.org/org/repo.git"
    );
}

#[test]
fn normalize_gitlab_requires_http_or_https() {
    let err = giturl::normalize_gitlab_repo("git@gitlab.com:org/repo.git:").unwrap_err();
    assert_eq!(
        err,
        "Gitlab requires http/https repo urls: e.g. https://gitlab.com/org/repo.git"
    );
}
