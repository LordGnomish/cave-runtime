// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Archive expansion — TruffleHog's `pkg/handlers/{tar,gz,zip}.go`.
//!
//! We support gzip-framed tar archives ("ustar") in pure Rust. The tar parser
//! walks 512-byte headers per [POSIX.1-2017 §20.6.1], pulls each entry's name
//! and size, and re-feeds the entry payload through the configured detector
//! pipeline. Tar entries that are not regular files (directories, symlinks)
//! are skipped.

use crate::decoders::{decode_gzip, scan_gzip_blob};
use crate::detector::{scan, Finding, SecretDetector};

const TAR_BLOCK: usize = 512;
const MAX_ENTRY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct TarEntry {
    pub name: String,
    pub size: usize,
    pub data: Vec<u8>,
}

/// Parse a single uncompressed ustar archive into entry records. Stops at the
/// first all-zero block (end-of-archive marker) or on a malformed header.
pub fn parse_tar(bytes: &[u8]) -> Vec<TarEntry> {
    let mut entries = Vec::new();
    let mut offset = 0;
    while offset + TAR_BLOCK <= bytes.len() {
        let header = &bytes[offset..offset + TAR_BLOCK];
        if header.iter().all(|b| *b == 0) {
            break;
        }
        let name = match parse_tar_name(header) {
            Some(n) => n,
            None => break,
        };
        let size = match parse_tar_size(header) {
            Some(s) if s <= MAX_ENTRY_BYTES => s,
            _ => break,
        };
        let typeflag = header[156];
        offset += TAR_BLOCK;
        if offset + size > bytes.len() {
            break;
        }
        if typeflag == b'0' || typeflag == 0 {
            entries.push(TarEntry {
                name,
                size,
                data: bytes[offset..offset + size].to_vec(),
            });
        }
        let blocks = size.div_ceil(TAR_BLOCK);
        offset += blocks * TAR_BLOCK;
    }
    entries
}

fn parse_tar_name(header: &[u8]) -> Option<String> {
    let raw = &header[0..100];
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let name = std::str::from_utf8(&raw[..end]).ok()?.to_string();
    if name.is_empty() {
        return None;
    }
    Some(name)
}

fn parse_tar_size(header: &[u8]) -> Option<usize> {
    let raw = &header[124..136];
    let end = raw
        .iter()
        .position(|&b| b == 0 || b == b' ')
        .unwrap_or(raw.len());
    let s = std::str::from_utf8(&raw[..end]).ok()?.trim();
    if s.is_empty() {
        return Some(0);
    }
    usize::from_str_radix(s, 8).ok()
}

/// Walk a tar archive and scan each regular-file entry.
pub fn scan_tar(bytes: &[u8], detectors: &[SecretDetector]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for entry in parse_tar(bytes) {
        let Ok(text) = std::str::from_utf8(&entry.data) else {
            continue;
        };
        let virt = format!("tar://{}", entry.name);
        findings.extend(scan(text, &virt, detectors));
    }
    findings
}

/// Walk a gzip-framed tar (`.tar.gz`) and scan each regular-file entry.
pub fn scan_tar_gz(bytes: &[u8], detectors: &[SecretDetector]) -> Vec<Finding> {
    let Some(tarball) = decode_gzip(bytes) else {
        return Vec::new();
    };
    scan_tar(&tarball, detectors)
}

/// Convenience: dispatch by magic bytes.
pub fn scan_archive(bytes: &[u8], detectors: &[SecretDetector]) -> Vec<Finding> {
    if bytes.len() >= 2 && bytes[..2] == [0x1f, 0x8b] {
        if let Some(decoded) = decode_gzip(bytes) {
            if looks_like_tar(&decoded) {
                return scan_tar(&decoded, detectors);
            }
            if let Ok(text) = std::str::from_utf8(&decoded) {
                return scan(text, "gzip://blob", detectors);
            }
            return scan_gzip_blob(bytes, "gzip://blob", detectors);
        }
    }
    if looks_like_tar(bytes) {
        return scan_tar(bytes, detectors);
    }
    Vec::new()
}

fn looks_like_tar(bytes: &[u8]) -> bool {
    bytes.len() >= 263 && &bytes[257..262] == b"ustar"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detector::builtin_detectors;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;

    fn build_tar(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, data) in entries {
            let mut hdr = [0u8; TAR_BLOCK];
            let name_bytes = name.as_bytes();
            hdr[..name_bytes.len().min(100)]
                .copy_from_slice(&name_bytes[..name_bytes.len().min(100)]);
            let mode = b"0000644\0";
            hdr[100..100 + mode.len()].copy_from_slice(mode);
            let uid = b"0000000\0";
            hdr[108..108 + uid.len()].copy_from_slice(uid);
            let gid = b"0000000\0";
            hdr[116..116 + gid.len()].copy_from_slice(gid);
            let size_str = format!("{:011o}", data.len());
            hdr[124..124 + size_str.len()].copy_from_slice(size_str.as_bytes());
            hdr[135] = 0;
            let mtime = b"00000000000\0";
            hdr[136..136 + mtime.len()].copy_from_slice(mtime);
            hdr[148..156].copy_from_slice(b"        ");
            hdr[156] = b'0';
            hdr[257..262].copy_from_slice(b"ustar");
            hdr[262] = 0;
            let mut chk: u32 = 0;
            for b in hdr.iter() {
                chk = chk.wrapping_add(*b as u32);
            }
            let chk_str = format!("{:06o}\0 ", chk);
            hdr[148..148 + chk_str.len()].copy_from_slice(chk_str.as_bytes());

            out.extend_from_slice(&hdr);
            out.extend_from_slice(data);
            let pad = (TAR_BLOCK - (data.len() % TAR_BLOCK)) % TAR_BLOCK;
            out.extend(std::iter::repeat(0u8).take(pad));
        }
        out.extend([0u8; TAR_BLOCK * 2]);
        out
    }

    #[test]
    fn parse_simple_tar_entry() {
        let tar = build_tar(&[("hello.txt", b"hello world\n")]);
        let entries = parse_tar(&tar);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "hello.txt");
        assert_eq!(&entries[0].data, b"hello world\n");
    }

    #[test]
    fn parse_two_entries() {
        let tar = build_tar(&[("a", b"AA"), ("b", b"BB")]);
        let entries = parse_tar(&tar);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "a");
        assert_eq!(entries[1].name, "b");
    }

    #[test]
    fn scan_tar_finds_secret_in_entry() {
        let tar = build_tar(&[("config.env", b"AWS_KEY=AKIAIOSFODNN7EXAMPLE\n")]);
        let det = builtin_detectors();
        let findings = scan_tar(&tar, &det);
        assert!(findings.iter().any(|f| f.detector == "aws-access-key"));
        assert!(findings[0].file.starts_with("tar://"));
    }

    #[test]
    fn scan_tar_gz_round_trip() {
        let tar = build_tar(&[("config.env", b"AWS_KEY=AKIAIOSFODNN7EXAMPLE\n")]);
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&tar).unwrap();
        let bytes = enc.finish().unwrap();
        let findings = scan_tar_gz(&bytes, &builtin_detectors());
        assert!(findings.iter().any(|f| f.detector == "aws-access-key"));
    }

    #[test]
    fn empty_tar_yields_no_entries() {
        let entries = parse_tar(&[0u8; TAR_BLOCK * 2]);
        assert!(entries.is_empty());
    }

    #[test]
    fn looks_like_tar_detects_ustar() {
        let tar = build_tar(&[("a", b"x")]);
        assert!(looks_like_tar(&tar));
        assert!(!looks_like_tar(b"hello"));
    }

    #[test]
    fn scan_archive_dispatches_gzip_tar() {
        let tar = build_tar(&[("c.env", b"AWS_KEY=AKIAIOSFODNN7EXAMPLE\n")]);
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&tar).unwrap();
        let bytes = enc.finish().unwrap();
        let findings = scan_archive(&bytes, &builtin_detectors());
        assert!(findings.iter().any(|f| f.detector == "aws-access-key"));
    }

    #[test]
    fn scan_archive_dispatches_plain_tar() {
        let tar = build_tar(&[("c.env", b"AWS_KEY=AKIAIOSFODNN7EXAMPLE\n")]);
        let findings = scan_archive(&tar, &builtin_detectors());
        assert!(findings.iter().any(|f| f.detector == "aws-access-key"));
    }

    #[test]
    fn scan_archive_returns_empty_for_unknown_format() {
        let findings = scan_archive(b"not an archive", &builtin_detectors());
        assert!(findings.is_empty());
    }

    #[test]
    fn parse_tar_size_octal() {
        let mut hdr = [0u8; TAR_BLOCK];
        hdr[124..124 + 6].copy_from_slice(b"000777");
        let s = parse_tar_size(&hdr).unwrap();
        assert_eq!(s, 0o777);
    }

    #[test]
    fn tar_entry_size_too_large_terminates() {
        let mut hdr = [0u8; TAR_BLOCK];
        hdr[0..4].copy_from_slice(b"big\0");
        let huge = format!("{:011o}", MAX_ENTRY_BYTES + 1);
        hdr[124..124 + huge.len()].copy_from_slice(huge.as_bytes());
        hdr[156] = b'0';
        hdr[257..262].copy_from_slice(b"ustar");
        let entries = parse_tar(&hdr);
        assert!(entries.is_empty());
    }
}
