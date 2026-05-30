// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD integration test for the UTF-8 finding sanitizer.
//!
//! Line-ports the behaviour of TruffleHog `pkg/sanitizer/utf8.go`:
//!   func UTF8(in string) string {
//!       return strings.Replace(strings.ToValidUTF8(in, "❗"), "\x00", "", -1)
//!   }
//! Invalid UTF-8 byte sequences are replaced with the sentinel "❗" and any NUL
//! bytes are stripped (Postgres text columns reject embedded NULs upstream).

use cave_secrets::sanitizer::{sanitize_utf8, sanitize_utf8_bytes};

#[test]
fn valid_ascii_passthrough() {
    // Upstream TestUTF8 "valid" case.
    assert_eq!(sanitize_utf8("hello123"), "hello123");
}

#[test]
fn invalid_byte_replaced_with_sentinel() {
    // Upstream TestUTF8 "santized" case: a Latin-1 0xE9 byte in the middle of
    // an otherwise-ASCII string is not valid UTF-8 and must become "❗".
    // "Gr\xE9gory Smith" -> "Gr❗gory Smith"
    let mut bytes = b"Gr".to_vec();
    bytes.push(0xE9);
    bytes.extend_from_slice(b"gory Smith");
    assert_eq!(sanitize_utf8_bytes(&bytes), "Gr❗gory Smith");
}

#[test]
fn nul_bytes_stripped() {
    // Upstream TestUTF8 third case: NUL bytes removed entirely.
    let input = "no \u{0}\u{0} nulls because postgres does not support it in text fields";
    assert_eq!(
        sanitize_utf8(input),
        "no  nulls because postgres does not support it in text fields"
    );
}

#[test]
fn sentinel_and_nul_combined() {
    // An invalid byte followed by a NUL: invalid -> sentinel, NUL -> dropped.
    let bytes = vec![b'a', 0xFF, 0x00, b'b'];
    assert_eq!(sanitize_utf8_bytes(&bytes), "a❗b");
}

#[test]
fn empty_input_yields_empty() {
    assert_eq!(sanitize_utf8(""), "");
    assert_eq!(sanitize_utf8_bytes(&[]), "");
}
