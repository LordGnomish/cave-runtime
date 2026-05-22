// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! File storage — Twenty v2.6.2 path traversal security hardening.
//!
//! Upstream: `packages/twenty-server/src/engine/core-modules/file-storage/`
//!
//! Three core functions ported from v2.6.2 security fixes:
//! 1. `is_safe_relative_path` — reject traversal, null bytes, abs paths, backslashes
//! 2. `assert_resource_path_is_safe` — thin wrapper that panics on violation
//! 3. `assert_storage_path_is_within_workspace` — ensure final path stays in workspace dir
//! 4. `build_on_storage_path` — join + safety check, collapse duplicate slashes

/// Normalize a relative path: collapse middle `..`, preserve leading `..`,
/// filter empty components from double-slash sequences.
/// Mirrors Node.js `path.normalize()` semantics for the path component logic.
fn normalize_path(s: &str) -> String {
    let parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();
    let mut result: Vec<String> = Vec::new();
    for part in &parts {
        if *part == ".." {
            if !result.is_empty() && !result.last().unwrap().starts_with("..") {
                result.pop();
            } else {
                result.push(part.to_string());
            }
        } else {
            result.push(part.to_string());
        }
    }
    result.join("/")
}

/// Check if a resource path is safe for file storage.
///
/// Returns `false` (unsafe) if:
/// * empty string
/// * contains `\0` null byte
/// * absolute path
/// * contains backslash `\`
/// * contains `..` path traversal after normalization
pub fn is_safe_relative_path(file_path: &str) -> bool {
    if file_path.is_empty() {
        return false;
      }
    if file_path.contains('\0') {
        return false;
      }
    if file_path.starts_with('/') {
        return false;
      }
    if file_path.contains('\\') {
        return false;
      }
      // Reject any path containing ".." as a segment — Node.js normalize
      // can resolve "a/b/../../etc" to "etc" but a path that traverses
      // above its own root is still an exploitation vector regardless.
    if file_path.split('/').any(|c| c == "..") {
        return false;
      }
    // Double-check: normalized path must not contain ".." either.
    let normalized = normalize_path(file_path);
    if normalized.split('/').any(|c| c == "..") {
        return false;
      }
    true
}

/// Assert that a resource path is safe; panics with a descriptive message on violation.
pub fn assert_resource_path_is_safe(file_path: &str) {
    assert!(
        is_safe_relative_path(file_path),
         "Invalid resource path: contains unsafe characters or path traversal"
     );
}

/// Assert that a resolved storage path stays within the expected workspace directory.
///
/// The expected prefix is `{workspace_id}/{application_universal_identifier}/{file_folder}/`.
pub fn assert_storage_path_is_within_workspace(
    on_storage_path: &str,
    workspace_id: &str,
    application_universal_identifier: &str,
    file_folder: &str,
) {
    let expected_prefix = format!(
        "{}/{}/{}",
        workspace_id, application_universal_identifier, file_folder
    );
    let expected_prefix = format!("{}/", expected_prefix);
    let normalized_path = normalize_path(on_storage_path);
    let normalized_prefix = normalize_path(&expected_prefix);
    assert!(
        normalized_path.starts_with(&normalized_prefix),
         "Invalid storage path: resolved path escapes the expected directory"
     );
}

/// Build the full storage path from components, checking safety first.
///
/// Join workspace_id, application_universal_identifier, file_folder,
/// and resource_path, normalizing overlapping slashes.
pub fn build_on_storage_path(
    workspace_id: &str,
    application_universal_identifier: &str,
    file_folder: &str,
    resource_path: &str,
) -> String {
    assert_resource_path_is_safe(resource_path);
    let joined = format!(
        "{}/{}/{}/{}",
        workspace_id, application_universal_identifier, file_folder, resource_path
    );
     // Collapse multiple slashes via normalization
    let result = normalize_path(&joined);
    result
}

// ────────────────────────────────────────────────────────────────────
// Unit tests — GREEN: all implementations correct → all pass.
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
     #[should_panic(
        expected = "Invalid resource path: contains unsafe characters or path traversal"
     )]
    fn test_assert_resource_path_safe_dotdot_panics() {
        assert_resource_path_is_safe("../../etc/passwd");
     }

     #[test]
     #[should_panic(
        expected = "Invalid resource path: contains unsafe characters or path traversal"
     )]
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
