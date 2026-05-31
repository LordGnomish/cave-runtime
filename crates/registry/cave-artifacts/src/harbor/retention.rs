// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: goharbor/harbor@c80058d52f555c9bd4552ea14c9d3e73ba0e4b12
//   src/pkg/retention/policy/rule/{index.go,lastx,latestps,latestpk,latestpl,
//   dayspl,daysps,always} + src/pkg/retention/policy/alg/or/processor.go
//! Harbor tag-retention rule engine.
//!
//! Harbor retention runs a project-scoped [`Policy`] of [`Rule`]s against the
//! tags of a repository and partitions them into *retain* and *delete* sets.
//! The data models (`RetentionPolicy`, `RetentionRule`, `RetentionSelector`)
//! live in [`super::harbor`]; this module is the *evaluator* the HTTP handler
//! `execute_retention` previously stubbed out.
//!
//! Faithful semantics (upstream `alg/or/processor.go`):
//!
//!   * Each enabled rule has **scope selectors** (repository name) and **tag
//!     selectors** (tag name / label). A candidate is *selected* by a rule iff
//!     it satisfies every scope selector AND every tag selector.
//!   * The rule's **performer** (template) chooses which of its selected
//!     candidates to *retain*; the rest of that rule's selected set are
//!     deletion candidates.
//!   * Rules combine with **OR**: an artifact retained by ANY rule is kept.
//!     `delete = (∪ each rule's selected) − (∪ each rule's retained)`.
//!   * Artifacts selected by no rule are out of scope — left untouched.
//!
//! Count-based performers (`latestPushedK`, `latestPulledN`) are evaluated
//! **per repository**, mirroring Harbor running retention one repository at a
//! time.

use chrono::{DateTime, Duration, Utc};
use std::collections::BTreeMap;

/// A single tagged artifact considered by the retention run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub repository: String,
    pub tag: String,
    pub push_time: DateTime<Utc>,
    pub pull_time: Option<DateTime<Utc>>,
    pub labels: Vec<String>,
}

impl Candidate {
    fn key(&self) -> (String, String) {
        (self.repository.clone(), self.tag.clone())
    }
}

/// Selector decoration — `matches` keeps the matched set, `excludes` inverts it
/// (Harbor's `Decoration` field: "matches" | "excludes").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decoration {
    Matches,
    Excludes,
}

/// Selector kind (Harbor `Kind`: "doublestar" | "regexp" | "label").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorKind {
    Doublestar,
    Regexp,
    Label,
}

/// One tag/repository/label selector.
#[derive(Debug, Clone)]
pub struct Selector {
    pub kind: SelectorKind,
    pub decoration: Decoration,
    pub pattern: String,
}

impl Selector {
    pub fn doublestar(decoration: Decoration, pattern: &str) -> Self {
        Self { kind: SelectorKind::Doublestar, decoration, pattern: pattern.to_string() }
    }
    pub fn regexp(decoration: Decoration, pattern: &str) -> Self {
        Self { kind: SelectorKind::Regexp, decoration, pattern: pattern.to_string() }
    }
    pub fn label(decoration: Decoration, pattern: &str) -> Self {
        Self { kind: SelectorKind::Label, decoration, pattern: pattern.to_string() }
    }

    /// Raw (pre-decoration) test of the pattern against a tag string.
    fn raw_tag_hit(&self, tag: &str) -> bool {
        match self.kind {
            SelectorKind::Doublestar => doublestar_match(&self.pattern, tag),
            SelectorKind::Regexp => regexp_full_match(&self.pattern, tag),
            // A label selector never matches by tag name.
            SelectorKind::Label => false,
        }
    }

    /// Whether this selector *selects* a tag string, honouring decoration.
    pub fn matches_tag(&self, tag: &str) -> bool {
        match self.decoration {
            Decoration::Matches => self.raw_tag_hit(tag),
            Decoration::Excludes => !self.raw_tag_hit(tag),
        }
    }

    /// Whether this selector selects a full candidate (handles label kind too).
    pub fn matches_candidate(&self, c: &Candidate) -> bool {
        let raw = match self.kind {
            SelectorKind::Label => c.labels.iter().any(|l| l == &self.pattern),
            _ => self.raw_tag_hit(&c.tag),
        };
        match self.decoration {
            Decoration::Matches => raw,
            Decoration::Excludes => !raw,
        }
    }

    /// Whether this selector selects a candidate by *repository* name.
    fn matches_repo(&self, c: &Candidate) -> bool {
        let raw = match self.kind {
            SelectorKind::Doublestar => doublestar_match(&self.pattern, &c.repository),
            SelectorKind::Regexp => regexp_full_match(&self.pattern, &c.repository),
            SelectorKind::Label => c.labels.iter().any(|l| l == &self.pattern),
        };
        match self.decoration {
            Decoration::Matches => raw,
            Decoration::Excludes => !raw,
        }
    }
}

/// Retention performer (Harbor rule template id).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    /// `latestPushedK` — keep the K most-recently-pushed (per repository).
    LatestPushedK(usize),
    /// `latestPulledN` — keep the N most-recently-pulled (per repository).
    LatestPulledN(usize),
    /// `nDaysSinceLastPush` — keep tags pushed within the last N days.
    NDaysSinceLastPush(i64),
    /// `nDaysSinceLastPull` — keep tags pulled within the last N days.
    NDaysSinceLastPull(i64),
    /// `always` — retain everything the selectors matched.
    Always,
}

/// One retention rule: selectors + a performer.
#[derive(Debug, Clone)]
pub struct Rule {
    pub disabled: bool,
    pub template: Template,
    pub scope_selectors: Vec<Selector>,
    pub tag_selectors: Vec<Selector>,
}

impl Rule {
    /// Candidates this rule's selectors pick out.
    fn select<'a>(&self, candidates: &'a [Candidate]) -> Vec<&'a Candidate> {
        candidates
            .iter()
            .filter(|c| {
                self.scope_selectors.iter().all(|s| s.matches_repo(c))
                    && self.tag_selectors.iter().all(|s| s.matches_candidate(c))
            })
            .collect()
    }

    /// Of the selected set, which to retain (per the performer).
    fn retain<'a>(&self, selected: &[&'a Candidate], now: DateTime<Utc>) -> Vec<&'a Candidate> {
        match self.template {
            Template::Always => selected.to_vec(),
            Template::NDaysSinceLastPush(days) => {
                let cutoff = now - Duration::days(days);
                selected.iter().copied().filter(|c| c.push_time >= cutoff).collect()
            }
            Template::NDaysSinceLastPull(days) => {
                let cutoff = now - Duration::days(days);
                selected
                    .iter()
                    .copied()
                    .filter(|c| c.pull_time.map(|t| t >= cutoff).unwrap_or(false))
                    .collect()
            }
            Template::LatestPushedK(k) => {
                Self::top_k_per_repo(selected, k, |c| Some(c.push_time))
            }
            Template::LatestPulledN(n) => {
                // Tags never pulled sort last and are dropped once the quota fills.
                Self::top_k_per_repo(selected, n, |c| c.pull_time)
            }
        }
    }

    /// Keep the `k` candidates with the greatest key, grouped by repository.
    /// Candidates whose key is `None` are eligible only after all keyed ones.
    fn top_k_per_repo<'a, F>(selected: &[&'a Candidate], k: usize, key: F) -> Vec<&'a Candidate>
    where
        F: Fn(&Candidate) -> Option<DateTime<Utc>>,
    {
        let mut by_repo: BTreeMap<&str, Vec<&'a Candidate>> = BTreeMap::new();
        for c in selected {
            by_repo.entry(c.repository.as_str()).or_default().push(c);
        }
        let mut keep = Vec::new();
        for (_repo, mut group) in by_repo {
            // Descending: Some(newest) first, None last; tie-break by tag for
            // deterministic output.
            group.sort_by(|a, b| {
                key(b)
                    .cmp(&key(a))
                    .then_with(|| a.tag.cmp(&b.tag))
            });
            for c in group.into_iter().take(k) {
                keep.push(c);
            }
        }
        keep
    }
}

/// A retention policy — an OR-combined set of rules.
#[derive(Debug, Clone)]
pub struct Policy {
    pub rules: Vec<Rule>,
}

/// Result of a retention run.
#[derive(Debug, Clone, Default)]
pub struct RetentionOutcome {
    /// Tags kept (retained-by-a-rule plus tags out of every rule's scope).
    pub retained: Vec<Candidate>,
    /// Tags selected by a rule but not retained — slated for deletion.
    pub deleted: Vec<Candidate>,
}

impl Policy {
    /// Partition `candidates` into retain/delete per the OR processor.
    pub fn evaluate(&self, candidates: &[Candidate], now: DateTime<Utc>) -> RetentionOutcome {
        use std::collections::BTreeSet;

        let mut in_scope: BTreeSet<(String, String)> = BTreeSet::new();
        let mut retained_keys: BTreeSet<(String, String)> = BTreeSet::new();

        for rule in self.rules.iter().filter(|r| !r.disabled) {
            let selected = rule.select(candidates);
            for c in &selected {
                in_scope.insert(c.key());
            }
            for c in rule.retain(&selected, now) {
                retained_keys.insert(c.key());
            }
        }

        let mut out = RetentionOutcome::default();
        for c in candidates {
            let key = c.key();
            // Deleted iff a rule put it in scope AND no rule retained it.
            if in_scope.contains(&key) && !retained_keys.contains(&key) {
                out.deleted.push(c.clone());
            } else {
                out.retained.push(c.clone());
            }
        }
        out
    }
}

// ── Pattern helpers ───────────────────────────────────────────────────────────

/// Anchored full-string regexp match (Harbor's regexp selector is implicitly
/// anchored via `^(?:pattern)$`). A pattern that fails to compile matches
/// nothing, mirroring upstream treating a bad rule as a no-op.
fn regexp_full_match(pattern: &str, text: &str) -> bool {
    let anchored = format!("^(?:{})$", pattern);
    match regex::Regex::new(&anchored) {
        Ok(re) => re.is_match(text),
        Err(_) => false,
    }
}

/// doublestar (bmatcuk/doublestar) glob match. Tag names are flat (no `/`), so
/// `*`, `?` and `**` all collapse to "any run of characters" here; for
/// repository names `/` is treated as an ordinary character which is sufficient
/// for the `**` / prefix globs Harbor templates emit.
fn doublestar_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    glob(&p, &t)
}

fn glob(pat: &[char], txt: &[char]) -> bool {
    // Classic two-pointer wildcard match with backtracking.
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star_p, mut star_t): (Option<usize>, usize) = (None, 0);
    while ti < txt.len() {
        if pi < pat.len() && (pat[pi] == '?' || pat[pi] == txt[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pat.len() && pat[pi] == '*' {
            // Collapse consecutive '*' (handles '**').
            while pi + 1 < pat.len() && pat[pi + 1] == '*' {
                pi += 1;
            }
            star_p = Some(pi);
            star_t = ti;
            pi += 1;
        } else if let Some(sp) = star_p {
            pi = sp + 1;
            star_t += 1;
            ti = star_t;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == '*' {
        pi += 1;
    }
    pi == pat.len()
}
