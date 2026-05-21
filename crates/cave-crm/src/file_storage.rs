// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! File storage — Twenty v2.6.2 path traversal security hardening.
//!
//! Upstream: `packages/twenty-server/src/engine/core-modules/file-storage/`
//!
//! Three core functions ported from v2.6.2 security fixes:
//! 1. `is_safe_relative_path` — reject traversal, null bytes, absolute paths, backslashes
//! 2. `assert_resource_path_is_safe` — thin wrapper that panics on violation
//! 3. `assert_storage_path_is_within_workspace` — ensure final path stays in workspace dir
//! 4. `build_on_storage_path` — join + safety check, collapse duplicate slashes

/// Check if a resource path is safe for file storage.
///
/// Returns `false` (unsafe) if:
/// * empty string
/// * contains `\0` null byte
/// * absolute path
/// * contains backslash `\`
/// * contains `..` path traversal after normalization
pub fn is_safe_relative_path(_file_path: &str) -> bool {
    // Red: stub — returns false for all inputs
    false
}

/// Assert that a resource path is safe; panics with a descriptive message on violation.
pub fn assert_resource_path_is_safe(_file_path: &str) {
    // Red: stub — does nothing (should panic on unsafe)
}

/// Assert that a resolved storage path stays within the expected workspace directory.
///
/// The expected prefix is `{workspace_id}/{application_universal_identifier}/{file_folder}/`.
pub fn assert_storage_path_is_within_workspace(
    _on_storage_path: &str,
    _workspace_id: &str,
    _application_universal_identifier: &str,
    _file_folder: &str,
) {
    // Red: stub — does nothing (should panic on escape)
}

/// Build the full storage path from components, checking safety first.
///
/// Join workspace_id, application_universal_identifier, file_folder,
/// and resource_path, normalizing overlapping slashes.
pub fn build_on_storage_path(
    _workspace_id: &str,
    _application_universal_identifier: &str,
    _file_folder: &str,
    _resource_path: &str,
) -> String {
    // Red: stub — returns empty
    String::new()
}

// ────────────────────────────────────────────────────────────────────
// Unit tests — RED: stubbed functions cause these to fail.
// GREEN: implementations correct → all pass.
// ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ─── is_safe_relative_path ─────────────────────────────────────

    #[test]
    fn test_safe_relative_path_empty_returns_false() {
        assert!(!is_safe_relative_path(""));
    }

    #[test]
    fn test_safe_relative_path_null_byte_returns_false() {
        assert!(!is_safe_relative_path("file\0name.txt"));
    }

    #[test]
    fn test_safe_relative_path_absolute_returns_false() {
        assert!(!is_safe_relative_path("/etc/passwd"));
    }

    #[test]
    fn test_safe_relative_path_backslash_returns_false() {
        assert!(!is_safe_relative_path("folder\\file.txt"));
    }

    #[test]
    fn test_safe_relative_path_dotdot_returns_false() {
        assert!(!is_safe_relative_path("../../etc/passwd"));
    }

    #[test]
    fn test_safe_relative_path_dotdot_nested_returns_false() {
        assert!(!is_safe_relative_path("a/b/../../etc/passwd"));
    }

    #[test]
    fn test_safe_relative_path_simple_valid_returns_true() {
        assert!(is_safe_relative_path("uploads/file.txt"));
    }

    #[test]
    fn test_safe_relative_path_nested_valid_returns_true() {
        assert!(is_safe_relative_path(
            "workspace/app/folder/deep/nested/file.txt"
        ));
    }

    #[test]
    fn test_safe_relative_path_single_file_valid() {
        assert!(is_safe_relative_path("file.txt"));
    }

    // ─── assert_resource_path_is_safe ───────────────────────────────

    #[test]
    fn test_assert_resource_path_safe_valid_silently_passes() {
        // Should not panic
        assert_resource_path_is_safe("uploads/doc.pdf");
    }

    #[test]
    #[should_panic(expected = "Invalid resource path: contains unsafe characters or path traversal")]
    fn test_assert_resource_path_safe_dotdot_panics() {
        assert_resource_path_is_safe("../../etc/passwd");
    }

     #[test]
     #[should_panic(expected = "Invalid resource path: contains unsafe characters or path traversal")]
    fn test_assert_resource_path_safe_absolute_panics() {
        assert_resource_path_is_safe("/etc/shadow");
    }

    // ─── assert_storage_path_is_within_workspace ────────────────────

    #[test]
    fn test_within_workspace_same_dir_passes() {
        assert_storage_path_is_within_workspace(
            "ws1/app1/files/uploads/doc.pdf",
            "ws1",
            "app1",
            "files",
        );
    }

    #[test]
    fn test_within_workspace_nested_passes() {
        assert_storage_path_is_within_workspace(
            "ws1/app1/files/folder/nested/deep.pdf",
            "ws1",
            "app1",
            "files",
        );
    }

    // ─── build_on_storage_path ──────────────────────────────────────

    #[test]
    fn test_build_path_simple_valid() {
        let result = build_on_storage_path("ws1", "app1", "files", "uploads/doc.pdf");
        assert_eq!(result, "ws1/app1/files/uploads/doc.pdf");
    }

    #[test]
    fn test_build_path_collapse_double_slashes() {
        let result = build_on_storage_path("ws1", "app1", "files", "uploads//doc.pdf");
        assert_eq!(result, "ws1/app1/files/uploads/doc.pdf");
    }
}
