// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Finding/output sanitizer.
//!
//! Faithful line-port of TruffleHog `pkg/sanitizer/utf8.go`:
//!
//! ```go
//! package sanitizer
//!
//! func UTF8(in string) string {
//!     return strings.Replace(strings.ToValidUTF8(in, "❗"), "\x00", "", -1)
//! }
//! ```
//!
//! TruffleHog runs every detector result value through `sanitizer.UTF8`
//! before it is serialized or persisted: invalid UTF-8 byte runs are replaced
//! with the sentinel rune `❗` (matching Go's `strings.ToValidUTF8` semantics —
//! one sentinel per maximal invalid run) and embedded NUL bytes are stripped
//! entirely (Postgres `text` columns reject `\x00`). This keeps secret values
//! and surrounding context renderable in JSON output and the portal without
//! emitting raw control bytes.

/// The replacement rune emitted for each maximal run of invalid UTF-8 bytes —
/// matches the literal `"❗"` argument passed to `strings.ToValidUTF8`.
pub const SENTINEL: &str = "❗";

/// Sanitize a Rust `&str` (already valid UTF-8) by stripping NUL bytes.
///
/// A Rust `&str` is guaranteed valid UTF-8, so the `ToValidUTF8` step is a
/// no-op for this entry point and only the NUL-stripping `strings.Replace`
/// remains. For raw, possibly-invalid byte input use [`sanitize_utf8_bytes`].
pub fn sanitize_utf8(input: &str) -> String {
    // strings.Replace(in, "\x00", "", -1) — drop every NUL.
    input.chars().filter(|&c| c != '\u{0}').collect()
}

/// Sanitize arbitrary bytes the way upstream `sanitizer.UTF8` sanitizes a Go
/// string (which is just a byte slice): replace each maximal run of invalid
/// UTF-8 bytes with [`SENTINEL`], then strip NUL bytes from the result.
///
/// This mirrors `strings.ToValidUTF8(in, "❗")` followed by
/// `strings.Replace(_, "\x00", "", -1)`.
pub fn sanitize_utf8_bytes(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        match std::str::from_utf8(&input[i..]) {
            Ok(valid) => {
                // Remainder is all valid UTF-8.
                push_stripping_nul(&mut out, valid);
                break;
            }
            Err(e) => {
                let good_upto = e.valid_up_to();
                if good_upto > 0 {
                    // SAFETY: validated as UTF-8 by from_utf8 up to good_upto.
                    let valid = std::str::from_utf8(&input[i..i + good_upto]).unwrap();
                    push_stripping_nul(&mut out, valid);
                }
                i += good_upto;
                // Go's `strings.ToValidUTF8` replaces each *maximal run* of
                // invalid bytes with a single sentinel. Rust's `from_utf8`
                // surfaces invalid bytes one (sub)sequence at a time, so we
                // must coalesce the contiguous invalid run ourselves: emit one
                // sentinel, then skip every following byte until valid UTF-8
                // resumes.
                out.push_str(SENTINEL);
                match e.error_len() {
                    Some(bad) => {
                        i += bad;
                        // Swallow any further immediately-adjacent invalid bytes
                        // so the whole run collapses to the single sentinel.
                        while i < input.len() {
                            match std::str::from_utf8(&input[i..]) {
                                Ok(_) => break,
                                Err(e2) if e2.valid_up_to() > 0 => break,
                                Err(e2) => match e2.error_len() {
                                    Some(bad2) => i += bad2,
                                    None => {
                                        i = input.len();
                                        break;
                                    }
                                },
                            }
                        }
                    }
                    // Truncated mid-sequence at the very end: the rest is one
                    // invalid run, already covered by the sentinel above.
                    None => break,
                }
            }
        }
    }
    out
}

/// Append `s` to `out`, dropping any NUL chars — the second half of upstream
/// `strings.Replace(_, "\x00", "", -1)`.
fn push_stripping_nul(out: &mut String, s: &str) {
    for c in s.chars() {
        if c != '\u{0}' {
            out.push(c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_passthrough() {
        assert_eq!(sanitize_utf8("hello123"), "hello123");
    }

    #[test]
    fn strips_nul_from_str() {
        assert_eq!(sanitize_utf8("a\u{0}b"), "ab");
    }

    #[test]
    fn single_invalid_run_one_sentinel() {
        // Two adjacent invalid bytes form one maximal run -> one sentinel.
        assert_eq!(sanitize_utf8_bytes(&[0xFF, 0xFE]), SENTINEL);
    }

    #[test]
    fn valid_multibyte_preserved() {
        let s = "café→ok";
        assert_eq!(sanitize_utf8_bytes(s.as_bytes()), s);
    }
}
