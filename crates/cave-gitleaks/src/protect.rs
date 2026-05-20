// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Protect command — pre-commit / pre-push staged-blob enforcement.
//!
//! Mirrors `cmd/protect.go` upstream (`v8.29.1`). The upstream behaviour:
//! enumerate staged files (or `--staged` blobs), run the detector over
//! each blob, and exit non-zero if any leak is detected.
//!
//! cave-gitleaks's protect surface takes the staged-blob bytes as input
//! so callers (cavectl + Git hooks) can plug in their own enumeration
//! (`git diff --cached --name-only` plus `git show :path`). This keeps
//! the module pure and unit-testable without needing a live repo.

use crate::detect::Detector;
use crate::finding::Finding;

/// Outcome of [`protect_staged_blobs`].
#[derive(Debug, Clone)]
pub enum ProtectOutcome {
    /// No findings in any staged blob.
    Clean,
    /// One or more findings — caller should refuse the commit / push.
    Blocked { findings: Vec<Finding> },
}

impl ProtectOutcome {
    pub fn is_clean(&self) -> bool {
        matches!(self, ProtectOutcome::Clean)
    }
}

/// Scan a list of `(path, content)` staged blobs with the default
/// built-in detector. Convenience wrapper around [`protect_staged_with`].
pub fn protect_staged_blobs(staged: &[(String, String)]) -> ProtectOutcome {
    let detector = Detector::with_builtins();
    protect_staged_with(&detector, staged)
}

/// Scan staged blobs with an explicit detector. Returns
/// [`ProtectOutcome::Clean`] if no findings, otherwise
/// [`ProtectOutcome::Blocked`] with the union of findings across blobs.
pub fn protect_staged_with(detector: &Detector, staged: &[(String, String)]) -> ProtectOutcome {
    let mut all = Vec::new();
    for (path, blob) in staged {
        let findings = detector.scan_str(path, blob);
        all.extend(findings);
    }
    if all.is_empty() {
        ProtectOutcome::Clean
    } else {
        ProtectOutcome::Blocked { findings: all }
    }
}

/// Exit code for the protect command — mirrors the upstream contract:
/// 0 on clean, 1 on blocked (so a CI / hook can branch on `$?`).
pub fn exit_code(outcome: &ProtectOutcome) -> i32 {
    match outcome {
        ProtectOutcome::Clean => 0,
        ProtectOutcome::Blocked { .. } => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_staged_set_is_clean() {
        assert!(protect_staged_blobs(&[]).is_clean());
    }

    #[test]
    fn exit_code_one_for_blocked() {
        let blocked = ProtectOutcome::Blocked { findings: vec![] };
        assert_eq!(exit_code(&blocked), 1);
    }

    #[test]
    fn exit_code_zero_for_clean() {
        assert_eq!(exit_code(&ProtectOutcome::Clean), 0);
    }
}
