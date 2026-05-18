// SPDX-License-Identifier: AGPL-3.0-or-later
//! RED tests for pulp_deb: ar(1) archive reader, RFC 822 control field
//! parser, Debian version comparison.

use cave_artifacts::pulp::plugins::deb::{
    cmp_debian_version, parse_ar_archive, parse_deb_control, ArFileHeader, DebControl,
};

#[test]
fn ar_archive_reads_member_headers() {
    // Build a minimal ar(1) archive: magic + one 6-byte file ("hello\n").
    let mut buf = Vec::new();
    buf.extend_from_slice(b"!<arch>\n");
    // 60-byte header for "debian-binary" with size 4
    let name = format!("{:<16}", "debian-binary");
    let header = format!(
        "{name}{mtime:<12}{owner:<6}{group:<6}{mode:<8}{size:<10}\x60\n",
        name = name,
        mtime = "1700000000",
        owner = "0",
        group = "0",
        mode = "100644",
        size = "4",
    );
    assert_eq!(header.len(), 60);
    buf.extend_from_slice(header.as_bytes());
    buf.extend_from_slice(b"2.0\n");
    // even-byte padding (4 is already even)

    let entries: Vec<ArFileHeader> = parse_ar_archive(&buf).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "debian-binary");
    assert_eq!(entries[0].size, 4);
    assert_eq!(&buf[entries[0].data_offset..entries[0].data_offset + entries[0].size as usize], b"2.0\n");
}

#[test]
fn ar_archive_rejects_bad_magic() {
    let buf = b"not-an-archive";
    assert!(parse_ar_archive(buf).is_err());
}

#[test]
fn ar_archive_odd_size_member_padded_to_even() {
    let mut buf = Vec::new();
    buf.extend_from_slice(b"!<arch>\n");
    // first member, odd size = 3
    let h1 = format!(
        "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}\x60\n",
        "a", "0", "0", "0", "100644", "3"
    );
    buf.extend_from_slice(h1.as_bytes());
    buf.extend_from_slice(b"abc");
    buf.push(b'\n'); // padding to even
    // second member, size 2
    let h2 = format!(
        "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}\x60\n",
        "b", "0", "0", "0", "100644", "2"
    );
    buf.extend_from_slice(h2.as_bytes());
    buf.extend_from_slice(b"xy");

    let entries = parse_ar_archive(&buf).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, "a");
    assert_eq!(entries[1].name, "b");
}

#[test]
fn parse_control_field_simple() {
    let raw = "\
Package: libc6
Source: glibc
Version: 2.35-0ubuntu3.4
Architecture: amd64
Maintainer: Ubuntu GLibC Maintainers <ubuntu-devel@lists.ubuntu.com>
Section: libs
Priority: optional
Depends: libgcc-s1 (>= 3.3), libcrypt1 (>= 1:4.4.10-10ubuntu4)
Description: GNU C Library: Shared libraries
 Contains the standard libraries that are used by
 nearly all programs on the system.
";
    let c: DebControl = parse_deb_control(raw).unwrap();
    assert_eq!(c.package, "libc6");
    assert_eq!(c.source.as_deref(), Some("glibc"));
    assert_eq!(c.version, "2.35-0ubuntu3.4");
    assert_eq!(c.architecture, "amd64");
    assert_eq!(c.section.as_deref(), Some("libs"));
    assert_eq!(c.priority.as_deref(), Some("optional"));
    assert!(c
        .depends
        .as_deref()
        .unwrap()
        .contains("libgcc-s1 (>= 3.3)"));
    assert!(c.description.as_deref().unwrap().starts_with("GNU C Library"));
    assert!(c.description.as_deref().unwrap().contains("\nContains the standard libraries"));
}

#[test]
fn parse_control_rejects_missing_package_or_version() {
    let raw = "Architecture: amd64\n";
    assert!(parse_deb_control(raw).is_err());
}

#[test]
fn deb_version_comparison_basic() {
    // Debian policy §5.6.12: epoch ▶ upstream ▶ revision.
    // Numeric runs compared as integers, non-digit runs lex with `~` < empty < other.
    use std::cmp::Ordering::*;
    assert_eq!(cmp_debian_version("1.0", "1.0"), Equal);
    assert_eq!(cmp_debian_version("1.0", "1.1"), Less);
    assert_eq!(cmp_debian_version("2.0", "1.99"), Greater);
    // Epoch dominates
    assert_eq!(cmp_debian_version("1:1.0", "2.0"), Greater);
    assert_eq!(cmp_debian_version("0:1.0", "1.0"), Equal);
    // Tilde sorts before everything
    assert_eq!(cmp_debian_version("1.0~rc1", "1.0"), Less);
    assert_eq!(cmp_debian_version("1.0~rc1", "1.0~rc2"), Less);
    // Revision
    assert_eq!(cmp_debian_version("1.0-1", "1.0-2"), Less);
}
