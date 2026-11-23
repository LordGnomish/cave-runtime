// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `diff::walking_differ` applies a layer inside its target directory.
//!
//! Applying a layer archive may create, change and remove entries below the
//! target directory and nowhere else. That also holds when the target
//! directory contains symlinks, left by an earlier layer or created by the
//! archive itself, and for whiteouts and opaque markers. `rootfs::extract_layer`
//! gives the same guarantees; its tests are in its own module.
//!
//! The tests run the real apply on archives built in memory. The target is
//! `<tmp>/layer`. Next to it, `<tmp>/outside` holds files that stand for
//! everything that is not part of the target: they have to be the same
//! before and after every apply.

use cave_cri::diff::walking_differ::{apply_layer, apply_uncompressed_tar, ApplyError, ApplyStats};
use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// The scratch tree of one test.
struct Scratch {
    _tmp: tempfile::TempDir,
    /// `<tmp>`, with symlinks resolved.
    root: PathBuf,
    /// `<tmp>/layer`: the directory the archive is applied to.
    layer: PathBuf,
    /// `<tmp>/outside`: holds `sentinel.txt` and `keep/inner.txt`.
    outside: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let layer = root.join("layer");
        let outside = root.join("outside");
        std::fs::create_dir(&layer).unwrap();
        std::fs::create_dir_all(outside.join("keep")).unwrap();
        std::fs::write(outside.join("sentinel.txt"), b"outside data").unwrap();
        std::fs::write(outside.join("keep").join("inner.txt"), b"outside data").unwrap();
        Scratch {
            _tmp: tmp,
            root,
            layer,
            outside,
        }
    }

    /// Absolute path of an entry of the `outside` directory, as text.
    fn outside_path(&self, name: &str) -> String {
        self.outside.join(name).to_str().unwrap().to_string()
    }

    /// Path → kind, mode, link count and content for `dir` and everything
    /// below it, except `skip`.
    fn snapshot_of(dir: &Path, skip: Option<&Path>) -> BTreeMap<PathBuf, String> {
        fn walk(dir: &Path, skip: Option<&Path>, seen: &mut BTreeMap<PathBuf, String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if Some(path.as_path()) == skip {
                    continue;
                }
                let meta = std::fs::symlink_metadata(&path).unwrap();
                let state = if meta.file_type().is_symlink() {
                    format!("symlink to {:?}", std::fs::read_link(&path).unwrap())
                } else if meta.is_dir() {
                    walk(&path, skip, seen);
                    format!("dir, mode {:o}", meta.mode())
                } else {
                    format!(
                        "file, mode {:o}, {} link(s), content {:?}",
                        meta.mode(),
                        meta.nlink(),
                        String::from_utf8_lossy(&std::fs::read(&path).unwrap())
                    )
                };
                seen.insert(path, state);
            }
        }
        let mut seen = BTreeMap::new();
        match std::fs::symlink_metadata(dir) {
            Ok(meta) => {
                seen.insert(dir.to_path_buf(), format!("dir, mode {:o}", meta.mode()));
                walk(dir, skip, &mut seen);
            }
            Err(e) => {
                seen.insert(dir.to_path_buf(), format!("missing: {e}"));
            }
        }
        seen
    }

    /// Everything in the scratch tree except the target directory. Equal
    /// snapshots before and after an apply mean that nothing outside the
    /// target directory was created, changed or removed.
    fn snapshot_outside(&self) -> BTreeMap<PathBuf, String> {
        Self::snapshot_of(&self.root, Some(&self.layer))
    }

    /// Everything in the target directory.
    fn snapshot_layer(&self) -> BTreeMap<PathBuf, String> {
        Self::snapshot_of(&self.layer, None)
    }

    /// A symlink `layer/<name>` that is there before the apply, as an
    /// earlier layer would leave it.
    fn existing_symlink(&self, name: &str, target: impl AsRef<Path>) {
        std::os::unix::fs::symlink(target, self.layer.join(name)).unwrap();
    }
}

/// One archive member. Its header bytes are filled in directly, because
/// `tar::Builder::append_data` and `tar::Header::set_path` refuse some of
/// the names these tests have to feed to the apply.
struct RawMember<'a> {
    name: &'a str,
    kind: tar::EntryType,
    link: &'a str,
    mode: u32,
    data: &'a [u8],
}

fn file<'a>(name: &'a str, data: &'a [u8]) -> RawMember<'a> {
    RawMember {
        name,
        kind: tar::EntryType::Regular,
        link: "",
        mode: 0o644,
        data,
    }
}

fn executable<'a>(name: &'a str, data: &'a [u8]) -> RawMember<'a> {
    RawMember {
        mode: 0o755,
        ..file(name, data)
    }
}

fn dir(name: &str) -> RawMember<'_> {
    RawMember {
        name,
        kind: tar::EntryType::Directory,
        link: "",
        mode: 0o755,
        data: b"",
    }
}

fn symlink<'a>(name: &'a str, target: &'a str) -> RawMember<'a> {
    RawMember {
        name,
        kind: tar::EntryType::Symlink,
        link: target,
        mode: 0o777,
        data: b"",
    }
}

fn hard_link<'a>(name: &'a str, target: &'a str) -> RawMember<'a> {
    RawMember {
        name,
        kind: tar::EntryType::Link,
        link: target,
        mode: 0o644,
        data: b"",
    }
}

/// `members` as an uncompressed tar archive, in the order given.
fn raw_tar(members: &[RawMember<'_>]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for member in members {
        let mut header = tar::Header::new_ustar();
        {
            let raw = header.as_old_mut();
            assert!(
                member.name.len() <= raw.name.len() && member.link.len() <= raw.linkname.len(),
                "{:?} -> {:?} does not fit a plain tar header",
                member.name,
                member.link
            );
            raw.name[..member.name.len()].copy_from_slice(member.name.as_bytes());
            raw.linkname[..member.link.len()].copy_from_slice(member.link.as_bytes());
        }
        header.set_entry_type(member.kind);
        header.set_mode(member.mode);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(1_700_000_000);
        header.set_size(member.data.len() as u64);
        header.set_cksum();
        tar.append(&header, member.data).unwrap();
    }
    tar.into_inner().unwrap()
}

/// Assert that `result` is the refusal of an archive because of its member
/// `offender`, and that the apply left everything outside the target
/// directory as it was in `outside_before`. Returns the error text.
fn assert_refusal(
    scratch: &Scratch,
    outside_before: &BTreeMap<PathBuf, String>,
    result: Result<ApplyStats, ApplyError>,
    offender: &str,
) -> String {
    assert_eq!(
        outside_before,
        &scratch.snapshot_outside(),
        "applying {offender:?} touched paths outside the target directory \
         (the apply returned {result:?})"
    );
    match result {
        Err(ApplyError::PathEscape(msg)) => {
            assert!(
                msg.contains(offender),
                "the error does not name the member {offender:?}: {msg}"
            );
            msg
        }
        other => panic!("an archive with {offender:?} must be refused, got {other:?}"),
    }
}

/// Apply `members` and assert that the archive is refused because of
/// `offender`, with nothing outside the target directory created, changed
/// or removed. Returns the error text.
fn assert_refused(scratch: &Scratch, members: &[RawMember<'_>], offender: &str) -> String {
    let archive = raw_tar(members);
    let before = scratch.snapshot_outside();
    let result = apply_uncompressed_tar(&archive, &scratch.layer);
    assert_refusal(scratch, &before, result, offender)
}

/// Names of the entries of `dir`, sorted.
fn entry_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

/// A symlink member may point anywhere: it is stored, not followed. A later
/// member of the same archive is not created by following it out of the
/// target directory.
#[test]
fn apply_refuses_a_member_below_a_symlink_from_the_same_archive_that_leaves_the_target() {
    for relative_target in [false, true] {
        let probe = Scratch::new();
        let elsewhere = probe.outside_path("keep/inner.txt");
        for through in [
            file("out/new.txt", b"from the layer"),
            file("out/sentinel.txt", b"from the layer"),
            file("out/sub/deeper/new.txt", b"from the layer"),
            dir("out/newdir"),
            dir("out/keep/newdir"),
            dir("out"),
            symlink("out/link", "sentinel.txt"),
            symlink("out/sentinel.txt", &elsewhere),
        ] {
            let scratch = Scratch::new();
            let absolute = scratch.outside_path("");
            let target = if relative_target {
                "../outside"
            } else {
                absolute.as_str()
            };
            let offender = through.name;
            assert_refused(&scratch, &[symlink("out", target), through], offender);
        }
    }
}

/// The same for a symlink that is in the target directory already.
#[test]
fn apply_refuses_a_member_below_an_existing_symlink_that_leaves_the_target() {
    for through in [
        file("out/new.txt", b"from the layer"),
        file("out/keep/inner.txt", b"from the layer"),
        file("out/sub/deeper/new.txt", b"from the layer"),
        dir("out/newdir"),
        dir("out"),
        symlink("out/link", "sentinel.txt"),
    ] {
        let scratch = Scratch::new();
        scratch.existing_symlink("out", &scratch.outside);
        let offender = through.name;
        assert_refused(&scratch, &[through], offender);
    }
}

/// A symlink that cannot be resolved has no directory behind it that could
/// be shown to be inside the target directory: nothing is created through
/// it, neither where it points nor anywhere else outside.
#[test]
fn apply_creates_nothing_below_a_symlink_that_cannot_be_resolved() {
    for through in [
        file("out/new.txt", b"from the layer"),
        dir("out/newdir"),
        dir("out"),
        symlink("out/link", "sentinel.txt"),
    ] {
        let scratch = Scratch::new();
        scratch.existing_symlink("out", scratch.outside.join("absent"));
        let before = scratch.snapshot_outside();
        let name = through.name;

        let result = apply_uncompressed_tar(&raw_tar(&[through]), &scratch.layer);

        assert!(result.is_err(), "{name:?} was applied: {result:?}");
        assert_eq!(
            before,
            scratch.snapshot_outside(),
            "applying {name:?} touched paths outside the target directory"
        );
    }
}

#[test]
fn apply_refuses_a_whiteout_below_a_symlink_that_leaves_the_target() {
    for through in [
        "out/.wh.sentinel.txt",
        "out/.wh.keep",
        "out/keep/.wh.inner.txt",
        "out/.wh.absent",
    ] {
        // The symlink comes with the archive ...
        let scratch = Scratch::new();
        assert_refused(
            &scratch,
            &[symlink("out", "../outside"), file(through, b"")],
            through,
        );

        // ... or is there already.
        let scratch = Scratch::new();
        scratch.existing_symlink("out", &scratch.outside);
        assert_refused(&scratch, &[file(through, b"")], through);
    }
}

#[test]
fn apply_refuses_an_opaque_marker_below_a_symlink_that_leaves_the_target() {
    for through in ["out/.wh..wh..opq", "out/keep/.wh..wh..opq"] {
        // The symlink comes with the archive ...
        let scratch = Scratch::new();
        assert_refused(
            &scratch,
            &[symlink("out", "../outside"), file(through, b"")],
            through,
        );

        // ... or is there already.
        let scratch = Scratch::new();
        scratch.existing_symlink("out", &scratch.outside);
        assert_refused(&scratch, &[file(through, b"")], through);
    }
}

/// The name after `.wh.` has to be one entry of the whiteout's own
/// directory. An empty name or `.` names that directory itself and `..` the
/// directory above it, which for a whiteout at the top is not part of the
/// target. Such a whiteout is an error; it is not counted as applied.
#[test]
fn apply_refuses_a_whiteout_that_names_no_entry_of_its_directory() {
    for name in [
        ".wh.",
        ".wh..",
        ".wh...",
        "conf/.wh.",
        "conf/.wh..",
        "conf/.wh...",
        "./.wh...",
    ] {
        let scratch = Scratch::new();
        std::fs::create_dir(scratch.layer.join("conf")).unwrap();
        std::fs::write(scratch.layer.join("conf/hostname"), b"cave\n").unwrap();
        std::fs::write(scratch.layer.join("motd"), b"hello\n").unwrap();
        let layer_before = scratch.snapshot_layer();

        let msg = assert_refused(&scratch, &[file(name, b"")], name);

        assert!(msg.contains("whiteout"), "unexpected error: {msg}");
        assert_eq!(
            layer_before,
            scratch.snapshot_layer(),
            "{name:?} removed entries of the target directory"
        );
    }
}

/// `apply_layer` is the same apply behind a gzip decoder.
#[test]
fn apply_layer_refuses_what_apply_uncompressed_tar_refuses() {
    let scratch = Scratch::new();
    scratch.existing_symlink("out", &scratch.outside);
    let gz = cave_cri::diff::compress_gzip(&raw_tar(&[file("out/new.txt", b"x")])).unwrap();
    let before = scratch.snapshot_outside();
    let result = apply_layer(&gz, &scratch.layer);
    assert_refusal(&scratch, &before, result, "out/new.txt");

    let scratch = Scratch::new();
    let gz = cave_cri::diff::compress_gzip(&raw_tar(&[
        symlink("out", "../outside"),
        file("out/keep/.wh..wh..opq", b""),
    ]))
    .unwrap();
    let before = scratch.snapshot_outside();
    let result = apply_layer(&gz, &scratch.layer);
    assert_refusal(&scratch, &before, result, "out/keep/.wh..wh..opq");

    let scratch = Scratch::new();
    std::fs::write(scratch.layer.join("motd"), b"hello\n").unwrap();
    let gz = cave_cri::diff::compress_gzip(&raw_tar(&[file(".wh..", b"")])).unwrap();
    let before = scratch.snapshot_outside();
    let result = apply_layer(&gz, &scratch.layer);
    assert_refusal(&scratch, &before, result, ".wh..");
    assert_eq!(
        std::fs::read(scratch.layer.join("motd")).unwrap(),
        b"hello\n"
    );
}

/// Names that are absolute or contain `..` were refused before and still
/// are, for every kind of member.
#[test]
fn apply_refuses_member_names_that_leave_the_target() {
    for name in ["../new.txt", "sub/../../new.txt", "conf/../hostname"] {
        let scratch = Scratch::new();
        assert_refused(&scratch, &[file(name, b"from the layer")], name);
    }
    for name in ["../outside/.wh.sentinel.txt", "../.wh..wh..opq"] {
        let scratch = Scratch::new();
        assert_refused(&scratch, &[file(name, b"")], name);
    }
    let scratch = Scratch::new();
    let name = scratch.outside_path("created.txt");
    assert_refused(&scratch, &[file(&name, b"from the layer")], &name);
    let scratch = Scratch::new();
    assert_refused(&scratch, &[dir("..")], "..");
    let scratch = Scratch::new();
    assert_refused(&scratch, &[symlink("../link", "layer")], "../link");
}

/// Hard link members are not applied. One whose target is outside the
/// target directory therefore creates nothing in it, and the file it names
/// does not gain a link.
#[test]
fn apply_creates_no_hard_link_to_a_target_outside() {
    let scratch = Scratch::new();
    let outside_file = scratch.outside_path("sentinel.txt");
    let archive = raw_tar(&[
        file("data.txt", b"from the layer"),
        hard_link("linked-absolute", &outside_file),
        hard_link("linked-relative", "../outside/sentinel.txt"),
        symlink("out", "../outside"),
        hard_link("linked-through", "out/sentinel.txt"),
        hard_link("out/linked-below", "data.txt"),
    ]);
    let before = scratch.snapshot_outside();

    let stats = apply_uncompressed_tar(&archive, &scratch.layer).unwrap();

    assert_eq!(before, scratch.snapshot_outside());
    assert_eq!(
        stats,
        ApplyStats {
            files_written: 1,
            dirs_created: 0,
            symlinks_created: 1,
            whiteouts_applied: 0,
        }
    );
    assert_eq!(entry_names(&scratch.layer), ["data.txt", "out"]);
    assert_eq!(
        std::fs::metadata(scratch.layer.join("data.txt"))
            .unwrap()
            .nlink(),
        1
    );
}

/// A whiteout removes the entry it names. If that entry is a symlink, the
/// link goes away and what it points to stays. The same holds for the
/// entries an opaque marker clears.
#[test]
fn apply_whiteout_of_a_symlink_removes_the_link_not_its_target() {
    let scratch = Scratch::new();
    let outside_dir = scratch.outside_path("keep");
    let outside_file = scratch.outside_path("sentinel.txt");
    let archive = raw_tar(&[
        symlink("to-dir", &outside_dir),
        symlink("to-file", &outside_file),
        dir("d"),
        symlink("d/to-dir", &outside_dir),
        symlink("d/to-file", &outside_file),
        file(".wh.to-dir", b""),
        file(".wh.to-file", b""),
        file("d/.wh..wh..opq", b""),
    ]);
    let before = scratch.snapshot_outside();

    let stats = apply_uncompressed_tar(&archive, &scratch.layer).unwrap();

    assert_eq!(before, scratch.snapshot_outside());
    assert_eq!(stats.whiteouts_applied, 3);
    assert_eq!(entry_names(&scratch.layer), ["d"]);
    assert!(entry_names(&scratch.layer.join("d")).is_empty());
}

/// A member whose name is taken by a symlink replaces the link instead of
/// writing through it.
#[test]
fn apply_replaces_a_symlink_that_has_the_name_of_a_member() {
    let scratch = Scratch::new();
    let outside_file = scratch.outside_path("sentinel.txt");
    let archive = raw_tar(&[
        symlink("motd", &outside_file),
        file("motd", b"from the layer"),
        symlink("alias", &outside_file),
        symlink("alias", "motd"),
    ]);
    let before = scratch.snapshot_outside();

    apply_uncompressed_tar(&archive, &scratch.layer).unwrap();

    assert_eq!(before, scratch.snapshot_outside());
    let motd = scratch.layer.join("motd");
    assert!(std::fs::symlink_metadata(&motd).unwrap().is_file());
    assert_eq!(std::fs::read(&motd).unwrap(), b"from the layer");
    assert_eq!(
        std::fs::read_link(scratch.layer.join("alias")).unwrap(),
        Path::new("motd")
    );
}

/// Regular files and directories of a well-formed layer.
#[test]
fn apply_writes_files_and_nested_directories() {
    let scratch = Scratch::new();
    let layer = &scratch.layer;
    // What a lower layer left behind.
    std::fs::create_dir_all(layer.join("conf")).unwrap();
    std::fs::write(layer.join("conf/app.conf"), b"old").unwrap();
    std::fs::write(layer.join("conf/kept.conf"), b"kept").unwrap();

    let archive = raw_tar(&[
        dir("./"),
        dir("conf/"),
        dir("opt/"),
        dir("opt/tool/bin/"),
        // A directory whose parents have no member of their own.
        dir("srv/a/b/c"),
        file("conf/hostname", b"cave\n"),
        file("conf/app.conf", b"new"),
        // A file whose parents have no member of their own.
        file("usr/share/doc/readme", b"read me"),
        executable("./opt/tool/bin/run", b"#!/bin/sh\n"),
        file("empty", b""),
    ]);
    let before = scratch.snapshot_outside();

    let stats = apply_uncompressed_tar(&archive, layer).unwrap();

    assert_eq!(before, scratch.snapshot_outside());
    assert_eq!(
        stats,
        ApplyStats {
            files_written: 5,
            dirs_created: 4,
            symlinks_created: 0,
            whiteouts_applied: 0,
        }
    );
    assert_eq!(
        entry_names(layer),
        ["conf", "empty", "opt", "srv", "usr"],
        "the target directory has other entries than the archive names"
    );
    assert_eq!(
        std::fs::read(layer.join("conf/hostname")).unwrap(),
        b"cave\n"
    );
    assert_eq!(std::fs::read(layer.join("conf/app.conf")).unwrap(), b"new");
    assert_eq!(
        std::fs::read(layer.join("conf/kept.conf")).unwrap(),
        b"kept"
    );
    assert_eq!(
        std::fs::read(layer.join("usr/share/doc/readme")).unwrap(),
        b"read me"
    );
    assert_eq!(
        std::fs::read(layer.join("opt/tool/bin/run")).unwrap(),
        b"#!/bin/sh\n"
    );
    assert_eq!(std::fs::read(layer.join("empty")).unwrap(), b"");
    assert!(layer.join("srv/a/b/c").is_dir());
    let mode = |name: &str| std::fs::metadata(layer.join(name)).unwrap().mode() & 0o7777;
    assert_eq!(mode("opt/tool/bin/run"), 0o755);
    assert_eq!(mode("conf/hostname"), 0o644);
    assert_eq!(
        std::fs::metadata(layer.join("conf/hostname"))
            .unwrap()
            .mtime(),
        1_700_000_000
    );
}

/// Whiteouts and opaque markers of a well-formed layer: a whiteout removes
/// the one entry it names, an opaque marker everything in its directory,
/// and neither leaves a file of its own behind.
#[test]
fn apply_removes_what_whiteouts_and_opaque_markers_name() {
    let scratch = Scratch::new();
    let layer = &scratch.layer;
    // What a lower layer left behind.
    std::fs::create_dir_all(layer.join("conf")).unwrap();
    std::fs::write(layer.join("conf/old.conf"), b"old").unwrap();
    std::fs::write(layer.join("conf/kept.conf"), b"kept").unwrap();
    std::fs::create_dir_all(layer.join("var/cache/deep")).unwrap();
    std::fs::write(layer.join("var/cache/deep/blob"), b"old").unwrap();
    std::fs::create_dir_all(layer.join("srv/data/sub")).unwrap();
    std::fs::write(layer.join("srv/data/a"), b"old").unwrap();
    std::fs::write(layer.join("srv/data/sub/b"), b"old").unwrap();
    std::fs::write(layer.join("srv/kept"), b"kept").unwrap();
    std::fs::write(layer.join("top"), b"old").unwrap();

    let archive = raw_tar(&[
        // A file, a directory with content, a file at the top.
        file("conf/.wh.old.conf", b""),
        file("./var/.wh.cache", b""),
        file(".wh.top", b""),
        // Nothing of that name, and no such directory.
        file("conf/.wh.absent", b""),
        file("tmp/.wh.absent", b""),
        // A directory with content, and no such directory.
        file("srv/data/.wh..wh..opq", b""),
        file("nowhere/.wh..wh..opq", b""),
    ]);
    let before = scratch.snapshot_outside();

    let stats = apply_uncompressed_tar(&archive, layer).unwrap();

    assert_eq!(before, scratch.snapshot_outside());
    assert_eq!(
        stats,
        ApplyStats {
            files_written: 0,
            dirs_created: 0,
            symlinks_created: 0,
            whiteouts_applied: 7,
        }
    );
    assert_eq!(entry_names(layer), ["conf", "srv", "var"]);
    assert_eq!(entry_names(&layer.join("conf")), ["kept.conf"]);
    assert!(entry_names(&layer.join("var")).is_empty());
    assert_eq!(entry_names(&layer.join("srv")), ["data", "kept"]);
    assert!(entry_names(&layer.join("srv/data")).is_empty());
}

/// Symlinks of a well-formed layer: a symlink member is stored as it is,
/// wherever it points, and members named through a symlink that stays
/// inside the target directory land where it points.
#[test]
fn apply_stores_symlinks_and_follows_one_that_stays_inside() {
    let scratch = Scratch::new();
    let layer = &scratch.layer;
    // What a lower layer left behind.
    std::fs::create_dir_all(layer.join("usr/lib/stale")).unwrap();
    std::fs::write(layer.join("usr/lib/old.so"), b"old").unwrap();
    std::fs::write(layer.join("usr/lib/stale/x"), b"old").unwrap();
    scratch.existing_symlink("lib64", "usr/lib");
    let outside_file = scratch.outside_path("sentinel.txt");

    let archive = raw_tar(&[
        file("conf/hostname", b"cave\n"),
        // Stored as they are.
        symlink("conf/alias", "hostname"),
        symlink("conf/absolute", &outside_file),
        symlink("up", "../outside"),
        // Reached through a symlink of this archive ...
        symlink("lib", "usr/lib"),
        file("lib/liby.so", b"y"),
        dir("lib/modules"),
        dir("lib"),
        file("lib/.wh.old.so", b""),
        file("lib/stale/.wh..wh..opq", b""),
        // ... and through one that was there before.
        file("lib64/libz.so", b"z"),
        dir("lib64/"),
    ]);
    let before = scratch.snapshot_outside();

    let stats = apply_uncompressed_tar(&archive, layer).unwrap();

    assert_eq!(before, scratch.snapshot_outside());
    assert_eq!(
        stats,
        ApplyStats {
            files_written: 3,
            dirs_created: 3,
            symlinks_created: 4,
            whiteouts_applied: 2,
        }
    );
    assert_eq!(
        std::fs::read_link(layer.join("conf/alias")).unwrap(),
        Path::new("hostname")
    );
    assert_eq!(std::fs::read(layer.join("conf/alias")).unwrap(), b"cave\n");
    assert_eq!(
        std::fs::read_link(layer.join("conf/absolute")).unwrap(),
        Path::new(&outside_file)
    );
    assert_eq!(
        std::fs::read_link(layer.join("up")).unwrap(),
        Path::new("../outside")
    );
    for link in ["lib", "lib64"] {
        assert_eq!(
            std::fs::read_link(layer.join(link)).unwrap(),
            Path::new("usr/lib"),
            "{link} is no longer the symlink it was"
        );
    }
    assert_eq!(
        entry_names(&layer.join("usr/lib")),
        ["liby.so", "libz.so", "modules", "stale"]
    );
    assert_eq!(std::fs::read(layer.join("usr/lib/liby.so")).unwrap(), b"y");
    assert_eq!(std::fs::read(layer.join("usr/lib/libz.so")).unwrap(), b"z");
    assert!(layer.join("usr/lib/modules").is_dir());
    assert!(entry_names(&layer.join("usr/lib/stale")).is_empty());
}
