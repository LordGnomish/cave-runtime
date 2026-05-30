// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Diff *production* path — the double-tree differ that containerd's
//! `core/diff/walking/differ.go` (`Compare`) implements on top of
//! `continuity/fs.Changes`. Given a `lower` (parent snapshot) and an
//! `upper` (current rootfs) directory, it computes the set of changes
//! and emits an OCI layer tarball (additions/modifications carry the
//! upper bytes; deletions carry `.wh.<name>` whiteout markers).
//!
//! The defining correctness property is round-trip closure: applying
//! the produced layer on top of a copy of `lower` reproduces `upper`
//! byte-for-byte. cave-cri's existing `apply_layer` is the read side;
//! this exercises the write side against it.

use cave_cri::diff::producer::{
    compute_changes, diff_layer, write_diff_tar, Change, ChangeKind, DiffLayer,
};
use cave_cri::diff::walking_differ::apply_uncompressed_tar;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Materialize a tree from `(relpath, Option<content>)` pairs. `None`
/// content means "create as an empty directory".
fn build_tree(root: &Path, entries: &[(&str, Option<&[u8]>)]) {
    for (rel, content) in entries {
        let p = root.join(rel);
        match content {
            Some(bytes) => {
                if let Some(parent) = p.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(&p, bytes).unwrap();
            }
            None => {
                fs::create_dir_all(&p).unwrap();
            }
        }
    }
}

/// Recursively snapshot a tree into a sorted list of
/// `(relpath, Option<bytes>)` — `None` for directories — for equality
/// comparison. Whiteout markers are never present in a clean tree.
fn snapshot_tree(root: &Path) -> Vec<(String, Option<Vec<u8>>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Option<Vec<u8>>)>) {
        let mut entries: Vec<_> = fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            let ty = e.file_type().unwrap();
            if ty.is_dir() {
                out.push((rel, None));
                walk(root, &p, out);
            } else {
                out.push((rel, Some(fs::read(&p).unwrap())));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for (rel, content) in snapshot_tree(src) {
        let p = dst.join(&rel);
        match content {
            None => fs::create_dir_all(&p).unwrap(),
            Some(bytes) => {
                if let Some(parent) = p.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(&p, &bytes).unwrap();
            }
        }
    }
}

fn change_for<'a>(changes: &'a [Change], rel: &str) -> Option<&'a Change> {
    changes.iter().find(|c| c.path == PathBuf::from(rel))
}

#[test]
fn identical_trees_produce_no_changes() {
    let lower = TempDir::new().unwrap();
    let upper = TempDir::new().unwrap();
    build_tree(lower.path(), &[("a.txt", Some(b"same")), ("d/b.txt", Some(b"x"))]);
    build_tree(upper.path(), &[("a.txt", Some(b"same")), ("d/b.txt", Some(b"x"))]);
    let changes = compute_changes(lower.path(), upper.path()).unwrap();
    assert!(changes.is_empty(), "expected no changes, got {changes:?}");
}

#[test]
fn added_file_is_change_add() {
    let lower = TempDir::new().unwrap();
    let upper = TempDir::new().unwrap();
    build_tree(lower.path(), &[("keep.txt", Some(b"k"))]);
    build_tree(upper.path(), &[("keep.txt", Some(b"k")), ("new.txt", Some(b"hi"))]);
    let changes = compute_changes(lower.path(), upper.path()).unwrap();
    let c = change_for(&changes, "new.txt").expect("new.txt change");
    assert_eq!(c.kind, ChangeKind::Add);
    // Unchanged files produce no change entry.
    assert!(change_for(&changes, "keep.txt").is_none());
}

#[test]
fn modified_file_is_change_modify() {
    let lower = TempDir::new().unwrap();
    let upper = TempDir::new().unwrap();
    build_tree(lower.path(), &[("f.txt", Some(b"old"))]);
    build_tree(upper.path(), &[("f.txt", Some(b"new-and-longer"))]);
    let changes = compute_changes(lower.path(), upper.path()).unwrap();
    let c = change_for(&changes, "f.txt").expect("f.txt change");
    assert_eq!(c.kind, ChangeKind::Modify);
}

#[test]
fn deleted_file_is_change_delete_and_emits_whiteout() {
    let lower = TempDir::new().unwrap();
    let upper = TempDir::new().unwrap();
    build_tree(lower.path(), &[("gone.txt", Some(b"bye")), ("stay.txt", Some(b"s"))]);
    build_tree(upper.path(), &[("stay.txt", Some(b"s"))]);
    let changes = compute_changes(lower.path(), upper.path()).unwrap();
    let c = change_for(&changes, "gone.txt").expect("gone.txt change");
    assert_eq!(c.kind, ChangeKind::Delete);

    // The produced tar carries the aufs/OCI whiteout marker so apply
    // removes the file from the lower layer.
    let tar = write_diff_tar(lower.path(), upper.path()).unwrap();
    let applied = TempDir::new().unwrap();
    copy_tree(lower.path(), applied.path());
    apply_uncompressed_tar(&tar, applied.path()).unwrap();
    assert!(!applied.path().join("gone.txt").exists());
    assert!(applied.path().join("stay.txt").exists());
}

#[test]
fn deleted_directory_emits_single_delete_not_per_child() {
    let lower = TempDir::new().unwrap();
    let upper = TempDir::new().unwrap();
    build_tree(
        lower.path(),
        &[("d/one", Some(b"1")), ("d/two", Some(b"2")), ("d/sub/three", Some(b"3"))],
    );
    build_tree(upper.path(), &[("other.txt", Some(b"o"))]);
    let changes = compute_changes(lower.path(), upper.path()).unwrap();
    // A single Delete for the directory subsumes the subtree.
    let dir_delete = change_for(&changes, "d").expect("d delete");
    assert_eq!(dir_delete.kind, ChangeKind::Delete);
    assert!(change_for(&changes, "d/one").is_none(), "no per-child delete under deleted dir");
    assert!(change_for(&changes, "d/sub").is_none());
}

#[test]
fn added_directory_recurses_to_include_children() {
    let lower = TempDir::new().unwrap();
    let upper = TempDir::new().unwrap();
    build_tree(lower.path(), &[("base.txt", Some(b"b"))]);
    build_tree(
        upper.path(),
        &[("base.txt", Some(b"b")), ("newdir/a", Some(b"a")), ("newdir/sub/b", Some(b"bb"))],
    );
    let changes = compute_changes(lower.path(), upper.path()).unwrap();
    assert_eq!(change_for(&changes, "newdir").unwrap().kind, ChangeKind::Add);
    assert_eq!(change_for(&changes, "newdir/a").unwrap().kind, ChangeKind::Add);
    assert_eq!(change_for(&changes, "newdir/sub/b").unwrap().kind, ChangeKind::Add);
}

#[test]
fn changes_are_deterministically_sorted() {
    let lower = TempDir::new().unwrap();
    let upper = TempDir::new().unwrap();
    build_tree(lower.path(), &[]);
    build_tree(
        upper.path(),
        &[("zeta", Some(b"z")), ("alpha", Some(b"a")), ("mid/x", Some(b"x"))],
    );
    let changes = compute_changes(lower.path(), upper.path()).unwrap();
    let paths: Vec<String> = changes.iter().map(|c| c.path.to_string_lossy().into()).collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted, "changes must be emitted in sorted path order");
}

#[test]
fn diff_layer_diff_id_is_sha256_of_uncompressed_tar() {
    let lower = TempDir::new().unwrap();
    let upper = TempDir::new().unwrap();
    build_tree(lower.path(), &[]);
    build_tree(upper.path(), &[("x", Some(b"payload"))]);
    let tar = write_diff_tar(lower.path(), upper.path()).unwrap();
    let DiffLayer { diff_id, tar_gz, uncompressed_len, changes } =
        diff_layer(lower.path(), upper.path()).unwrap();
    use cave_cri::content::digest::{Digest, DigestAlgorithm};
    assert_eq!(diff_id, Digest::compute(DigestAlgorithm::Sha256, &tar));
    assert_eq!(uncompressed_len, tar.len());
    assert!(!tar_gz.is_empty());
    assert_eq!(changes.len(), 1);
    // The gzipped layer decompresses back to the exact tar.
    let restored = cave_cri::diff::compression::decompress_gzip(&tar_gz).unwrap();
    assert_eq!(restored, tar);
}

#[test]
fn round_trip_apply_of_produced_layer_reproduces_upper() {
    let lower = TempDir::new().unwrap();
    let upper = TempDir::new().unwrap();
    // A mixed scenario: add, modify, delete-file, delete-dir, nested add.
    build_tree(
        lower.path(),
        &[
            ("unchanged.txt", Some(b"keep")),
            ("modified.txt", Some(b"v1")),
            ("deleted.txt", Some(b"remove-me")),
            ("deldir/a", Some(b"a")),
            ("deldir/b", Some(b"b")),
        ],
    );
    build_tree(
        upper.path(),
        &[
            ("unchanged.txt", Some(b"keep")),
            ("modified.txt", Some(b"v2-bigger-content")),
            ("added.txt", Some(b"brand-new")),
            ("newdir/nested/deep.txt", Some(b"deep")),
        ],
    );

    let tar = write_diff_tar(lower.path(), upper.path()).unwrap();
    let applied = TempDir::new().unwrap();
    copy_tree(lower.path(), applied.path());
    apply_uncompressed_tar(&tar, applied.path()).unwrap();

    assert_eq!(
        snapshot_tree(applied.path()),
        snapshot_tree(upper.path()),
        "applying the produced layer onto lower must reproduce upper exactly"
    );
}

#[test]
fn empty_diff_of_identical_trees_round_trips_to_no_op() {
    let lower = TempDir::new().unwrap();
    let upper = TempDir::new().unwrap();
    build_tree(lower.path(), &[("a", Some(b"1")), ("d/b", Some(b"2"))]);
    build_tree(upper.path(), &[("a", Some(b"1")), ("d/b", Some(b"2"))]);
    let tar = write_diff_tar(lower.path(), upper.path()).unwrap();
    let applied = TempDir::new().unwrap();
    copy_tree(lower.path(), applied.path());
    apply_uncompressed_tar(&tar, applied.path()).unwrap();
    assert_eq!(snapshot_tree(applied.path()), snapshot_tree(upper.path()));
}
