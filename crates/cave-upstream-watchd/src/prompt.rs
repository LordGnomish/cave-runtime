//! Port-prompt builder.
//!
//! Given a `GapEvent` + a per-crate context (parity ratio, unmapped
//! modules), produce the prompt string the auto-port dispatcher
//! submits to a [`crate::auto_port::TaskQueue`]. The prompt is
//! plain text — pluggable backends (Qwen pump, Anthropic API)
//! consume it directly without any wrapper format.

use crate::changelog::{ChangeKind, Changelog};
use crate::diff::Severity;
use crate::event::GapEvent;
use serde::{Deserialize, Serialize};

/// Per-crate context the dispatcher resolves before building the
/// prompt. The dispatcher reads this from `parity-index.json` (live)
/// + the crate's `parity.manifest.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortContext {
    pub crate_name: String,
    pub current_fill_ratio: Option<f64>,
    pub upstream_org_repo: String,
    pub unmapped_modules: Vec<String>,
    /// Optional branch name the dispatcher wants the task to land
    /// on. Defaults to `auto-port/<event_id>`.
    pub target_branch: Option<String>,
}

/// Build the port prompt for one (event, ctx) pair. Sections:
///
/// 1. Project identity (`UPSTREAM PROJECT`)
/// 2. Version diff (`CURRENT_PIN → LATEST_TAG`, severity)
/// 3. Parsed changelog (Added / Changed / Deprecated / Breaking)
/// 4. Crate context (current parity, unmapped list)
/// 5. Task description (charter rules — line-by-line, no stubs)
/// 6. Charter v2 gate (cargo check + test + ratio bump + zero stubs)
pub fn build_prompt(event: &GapEvent, ctx: &PortContext) -> String {
    let added = changelog_section(&event.changelog, ChangeKind::Added);
    let changed = changelog_section(&event.changelog, ChangeKind::Changed);
    let deprecated = changelog_section(&event.changelog, ChangeKind::Deprecated);
    let breaking = changelog_section(&event.changelog, ChangeKind::Breaking);
    let fixed = changelog_section(&event.changelog, ChangeKind::Fixed);
    let removed = changelog_section(&event.changelog, ChangeKind::Removed);
    let security = changelog_section(&event.changelog, ChangeKind::Security);

    let pin = event.previous_pin.as_deref().unwrap_or("(unpinned)");
    let parity = ctx
        .current_fill_ratio
        .map(|r| format!("{:.4}", r))
        .unwrap_or_else(|| "(unmeasured)".to_string());
    let unmapped = if ctx.unmapped_modules.is_empty() {
        "(none recorded — see parity.manifest.toml [[unmapped]] block)".to_string()
    } else {
        ctx.unmapped_modules
            .iter()
            .map(|m| format!("  - {m}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let target_branch = ctx
        .target_branch
        .clone()
        .unwrap_or_else(|| format!("auto-port/{}", event.event_id));

    format!(
        r#"You are auto-porting an upstream gap detected by cave-upstream-watchd.

== UPSTREAM ==
project        : {repo}
previous pin   : {pin}
latest release : {latest}
severity       : {sev:?}
released at    : {released}

== CHANGELOG (parsed) ==
Added:
{added}
Changed:
{changed}
Deprecated:
{deprecated}
Breaking:
{breaking}
Fixed:
{fixed}
Removed:
{removed}
Security:
{security}

== CRATE ==
name              : {crate_name}
current fill_ratio: {parity}
unmapped modules :
{unmapped}

== TASK ==
1. Pick the highest-priority unmapped module impacted by the new release.
2. Charter altın kuralı — line-by-line parity, NO stubs:
   * forbidden: `unimplemented!()`, `todo!()`, `#[ignore = "impl pending"]`
   * every public function must have at least one deterministic test
3. Tests must be hermetic — no live network / cluster / fs-outside-tempdir.
4. Update `crates/{crate_name}/parity.manifest.toml`:
   * flip the chosen `[[unmapped]]` block to `[[mapped]]` with `local_files`.
   * bump `mapped_count`, `unmapped_count`, `fill_ratio`, `last_audit`.
5. Push the branch — TWO commits required (see TDD STRICT MODE below).
   Do NOT merge to main — the charter-v2 gate is responsible for
   verifying + merging.

== TDD STRICT MODE (REQUIRED — gate enforced) ==
The dispatcher runs cave-upstream-watchd's tdd_analyzer over your
branch's commit chain. Output MUST be at least TWO commits in this
exact order; a single combined commit FAILS the gate.

1. RED commit (push this FIRST):
   * touches ONLY `tests/` or `src/<module>/tests.rs` (cfg(test) mod)
   * adds failing tests for the module you are about to port
   * `cargo test -p {crate_name}` MUST fail at this commit
     (the analyzer verifies via `git stash apply <red>` + run)
   * commit message: `test({crate_name}): red — <feature> tests fail before impl`

2. GREEN commit (push this SECOND):
   * touches `src/<module>.rs` (and optionally `parity.manifest.toml`)
   * implements the module so the red tests now pass
   * NO new stubs anywhere in the workspace
     (forbidden: `unimplemented!()`, `todo!()`, `panic!("not yet")`,
      `#[ignore = "impl pending"]`)
   * `cargo test -p {crate_name} --include-ignored` MUST exit 0
   * commit message: `feat({crate_name}): green — auto-port {latest} {short_desc}`

3. REFACTOR commit (OPTIONAL, push LAST if present):
   * idiomatic cleanup, NO behaviour change
   * tests still green
   * commit message: `refactor({crate_name}): cleanup`

The gate's `verify_with_tdd` rejects branches where:
  - the first commit's diff doesn't classify as `CommitKind::TestsOnly`
  - the second commit's diff doesn't classify as `CommitKind::ImplOnly`
  - any commit reintroduces a stub (stub_scan delta > 0)
  - the test count between RED and GREEN doesn't strictly increase

== CHARTER v2 GATE (the dispatcher runs this — pass to merge) ==
  cargo check --workspace --tests                            (must be clean)
  cargo test  -p {crate_name} --include-ignored              (must pass)
  parity_ratio({crate_name}) AFTER must be > BEFORE          (kanıt: fill_ratio delta > 0)
  zero new stub introductions across the workspace
  every new public fn is covered by at least one test

OUTPUT (the dispatcher reads these from `git log -1`):
  commit_sha     : <40-char SHA of the auto-port commit>
  branch         : {branch}
  files_changed  : <int>
  lines_added    : <int>
  test_count     : <delta>
"#,
        repo = event.github_repo,
        pin = pin,
        latest = event.latest_tag,
        sev = event.severity,
        released = event.at.format("%Y-%m-%d"),
        added = added,
        changed = changed,
        deprecated = deprecated,
        breaking = breaking,
        fixed = fixed,
        removed = removed,
        security = security,
        crate_name = ctx.crate_name,
        parity = parity,
        unmapped = unmapped,
        branch = target_branch,
        short_desc = short_description(event),
    )
}

fn short_description(event: &GapEvent) -> String {
    // Use the first changelog entry as a hint when present.
    if let Some(first) = event.changelog.entries.first() {
        let bullet = first.description.trim();
        if !bullet.is_empty() {
            return bullet.chars().take(60).collect();
        }
    }
    format!("{:?}-bump to {}", event.severity, event.latest_tag)
}

fn changelog_section(c: &Changelog, kind: ChangeKind) -> String {
    let entries = c.of_kind(kind);
    if entries.is_empty() {
        return "  (none)".to_string();
    }
    entries
        .iter()
        .map(|e| format!("  - {}", e.description.trim()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changelog::{ChangelogEntry, Changelog};
    use chrono::{DateTime, Utc};

    fn ts() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-13T14:00:00Z").unwrap().with_timezone(&Utc)
    }

    fn sample_event() -> GapEvent {
        let cl = Changelog {
            entries: vec![
                ChangelogEntry { kind: ChangeKind::Added, description: "scheduler kubelet image-locality v2".into(), breaking: false },
                ChangelogEntry { kind: ChangeKind::Breaking, description: "remove deprecated --foo flag".into(), breaking: true },
            ],
        };
        GapEvent::new(
            "cave-scheduler",
            "kubernetes/kubernetes",
            Some("v1.36.0".into()),
            "v1.37.0",
            Severity::Minor,
            Some(3600),
            Some(0.8966),
            cl,
            ts(),
        )
    }

    fn sample_ctx() -> PortContext {
        PortContext {
            crate_name: "cave-scheduler".into(),
            current_fill_ratio: Some(0.8966),
            upstream_org_repo: "kubernetes/kubernetes".into(),
            unmapped_modules: vec![
                "pkg/scheduler/framework/plugins/interpodaffinity/".into(),
                "pkg/scheduler/framework/plugins/volumezone/".into(),
            ],
            target_branch: None,
        }
    }

    #[test]
    fn prompt_includes_upstream_identity_and_version_diff() {
        let p = build_prompt(&sample_event(), &sample_ctx());
        assert!(p.contains("kubernetes/kubernetes"));
        assert!(p.contains("v1.36.0"));
        assert!(p.contains("v1.37.0"));
        assert!(p.contains("Minor"));
    }

    #[test]
    fn prompt_renders_changelog_by_kind_with_dashes() {
        let p = build_prompt(&sample_event(), &sample_ctx());
        assert!(p.contains("Added:\n  - scheduler kubelet image-locality v2"));
        assert!(p.contains("Breaking:\n  - remove deprecated --foo flag"));
        // Empty sections render as `(none)`.
        assert!(p.contains("Deprecated:\n  (none)"));
    }

    #[test]
    fn prompt_includes_crate_context_with_current_parity_and_unmapped() {
        let p = build_prompt(&sample_event(), &sample_ctx());
        assert!(p.contains("cave-scheduler"));
        assert!(p.contains("0.8966"));
        assert!(p.contains("interpodaffinity"));
        assert!(p.contains("volumezone"));
    }

    #[test]
    fn prompt_specifies_charter_gate_with_zero_stubs_clause() {
        let p = build_prompt(&sample_event(), &sample_ctx());
        assert!(p.contains("Charter altın kuralı"));
        assert!(p.contains("unimplemented!()"));
        assert!(p.contains("todo!()"));
        assert!(p.contains("impl pending"));
        assert!(p.contains("CHARTER v2 GATE"));
        assert!(p.contains("cargo check --workspace"));
        assert!(p.contains("zero new stub"));
    }

    /// TDD STRICT MODE (charter v2 enforcement) must be baked into the
    /// prompt so the model knows the gate requires red → green → refactor
    /// commit chain. Added 2026-05-13 (feat/auto-port-prod-activation-tdd-strict).
    #[test]
    fn prompt_declares_tdd_strict_mode_red_green_refactor() {
        let p = build_prompt(&sample_event(), &sample_ctx());
        // Top-level section heading.
        assert!(
            p.contains("TDD STRICT MODE"),
            "prompt must declare TDD STRICT MODE section"
        );
        // Three phase markers — order matters for the analyzer.
        assert!(p.contains("RED commit"), "prompt must name the RED phase");
        assert!(p.contains("GREEN commit"), "prompt must name the GREEN phase");
        assert!(
            p.contains("REFACTOR commit"),
            "prompt must mention the optional REFACTOR phase"
        );
        // Stub list reaffirmed inside TDD section (defence in depth).
        assert!(
            p.contains("panic!(\"not yet\")"),
            "TDD section must explicitly forbid panic!(\"not yet\") as a stub form"
        );
        // Analyzer hook referenced so the model knows the gate is real.
        assert!(
            p.contains("verify_with_tdd"),
            "prompt must reference CharterV2Gate::verify_with_tdd"
        );
        // Commit-message convention the gate's classifier matches on.
        // The format!() in build_prompt substitutes `{crate_name}` so the
        // rendered prompt carries the concrete crate name (sample_ctx
        // uses "cave-scheduler").
        assert!(
            p.contains("test(cave-scheduler): red"),
            "RED commit-message template missing for sample crate"
        );
        assert!(
            p.contains("feat(cave-scheduler): green"),
            "GREEN commit-message template missing for sample crate"
        );
        assert!(
            p.contains("refactor(cave-scheduler): cleanup"),
            "REFACTOR commit-message template missing for sample crate"
        );
    }

    #[test]
    fn prompt_carries_target_branch_default_event_id() {
        let p = build_prompt(&sample_event(), &sample_ctx());
        assert!(p.contains("auto-port/GAP-2026"));
    }

    #[test]
    fn prompt_carries_explicit_target_branch_override() {
        let mut ctx = sample_ctx();
        ctx.target_branch = Some("custom-branch".into());
        let p = build_prompt(&sample_event(), &ctx);
        assert!(p.contains("custom-branch"));
        assert!(!p.contains("auto-port/GAP-"));
    }

    #[test]
    fn empty_unmapped_list_shows_helpful_placeholder() {
        let mut ctx = sample_ctx();
        ctx.unmapped_modules = vec![];
        let p = build_prompt(&sample_event(), &ctx);
        assert!(p.contains("(none recorded"));
    }

    #[test]
    fn short_description_uses_first_changelog_entry_when_present() {
        let event = sample_event();
        let s = short_description(&event);
        assert!(s.contains("image-locality"));
    }

    #[test]
    fn short_description_falls_back_to_severity_when_changelog_empty() {
        let mut event = sample_event();
        event.changelog = Changelog::default();
        let s = short_description(&event);
        assert!(s.contains("v1.37.0"));
    }

    #[test]
    fn missing_pin_renders_unpinned_placeholder() {
        let mut event = sample_event();
        event.previous_pin = None;
        let p = build_prompt(&event, &sample_ctx());
        assert!(p.contains("(unpinned)"));
    }

    #[test]
    fn output_section_documents_dispatcher_contract_fields() {
        let p = build_prompt(&sample_event(), &sample_ctx());
        for field in &["commit_sha", "branch", "files_changed", "lines_added", "test_count"] {
            assert!(p.contains(field), "missing OUTPUT field: {field}");
        }
    }
}
