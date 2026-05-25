// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zaproxy@v2.14.0
//   zap/src/main/java/org/zaproxy/zap/extension/fuzz/
//
//! Fuzzer extension — parity with `ExtensionFuzz.java` +
//! `payloads/PayloadGenerator.java` (ZAP 2.14.0).
//!
//! Two-stage pipeline: a `PayloadGenerator` produces seed values, then
//! a chain of `PayloadProcessor`s transforms each value (URL-encode,
//! Base64, prefix/suffix, MD5, etc.). The final payload is substituted
//! into a `FuzzLocation` marker in a request.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum PayloadGenerator {
    /// Inline list of literal payloads.
    Strings { values: Vec<String> },
    /// Numeric range generator (inclusive start, exclusive end, step).
    NumberRange { start: i64, end: i64, step: i64 },
    /// Single-character bytes 0x00..=0xFF (for boundary-byte fuzzing).
    AllBytes,
    /// File contents read line-by-line. For tests we pass content
    /// inline rather than touching disk.
    FileContent { content: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum PayloadProcessor {
    UrlEncode,
    Base64Encode,
    Md5Hash,
    Prefix { value: String },
    Suffix { value: String },
    UpperCase,
    LowerCase,
    /// Reject (filter out) payloads not matching the substring.
    FilterContains { needle: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FuzzJob {
    pub generator: PayloadGenerator,
    pub processors: Vec<PayloadProcessor>,
}

pub fn generate(generator: &PayloadGenerator) -> Vec<String> {
    match generator {
        PayloadGenerator::Strings { values } => values.clone(),
        PayloadGenerator::NumberRange { start, end, step } => {
            if *step == 0 {
                return Vec::new();
            }
            let mut out = Vec::new();
            let mut i = *start;
            if *step > 0 {
                while i < *end {
                    out.push(i.to_string());
                    i += step;
                }
            } else {
                while i > *end {
                    out.push(i.to_string());
                    i += step;
                }
            }
            out
        }
        PayloadGenerator::AllBytes => (0u8..=255).map(|b| format!("\\x{:02x}", b)).collect(),
        PayloadGenerator::FileContent { content } => {
            content.lines().map(String::from).collect()
        }
    }
}

pub fn process(payload: String, processor: &PayloadProcessor) -> Option<String> {
    match processor {
        PayloadProcessor::UrlEncode => Some(url_encode(&payload)),
        PayloadProcessor::Base64Encode => Some(b64_encode(payload.as_bytes())),
        PayloadProcessor::Md5Hash => Some(md5_hex(payload.as_bytes())),
        PayloadProcessor::Prefix { value } => Some(format!("{}{}", value, payload)),
        PayloadProcessor::Suffix { value } => Some(format!("{}{}", payload, value)),
        PayloadProcessor::UpperCase => Some(payload.to_uppercase()),
        PayloadProcessor::LowerCase => Some(payload.to_lowercase()),
        PayloadProcessor::FilterContains { needle } => {
            if payload.contains(needle) {
                Some(payload)
            } else {
                None
            }
        }
    }
}

pub fn run_job(job: &FuzzJob) -> Vec<String> {
    let mut payloads = generate(&job.generator);
    for proc in job.processors.iter() {
        payloads = payloads
            .into_iter()
            .filter_map(|p| process(p, proc))
            .collect();
    }
    payloads
}

/// Replace the first occurrence of `{FUZZ}` in `template` with `payload`.
pub fn fuzz_substitute(template: &str, payload: &str) -> String {
    template.replacen("{FUZZ}", payload, 1)
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn b64_encode(input: &[u8]) -> String {
    const TBL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let chunks = input.chunks(3);
    for chunk in chunks {
        let n = chunk.len();
        let mut buf = [0u8; 3];
        buf[..n].copy_from_slice(chunk);
        let b = ((buf[0] as u32) << 16) | ((buf[1] as u32) << 8) | (buf[2] as u32);
        out.push(TBL[((b >> 18) & 0x3F) as usize] as char);
        out.push(TBL[((b >> 12) & 0x3F) as usize] as char);
        if n >= 2 {
            out.push(TBL[((b >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if n >= 3 {
            out.push(TBL[(b & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn md5_hex(input: &[u8]) -> String {
    // Minimal MD5 implementation — used for fuzzing payload obfuscation
    // (MD5 is **NOT** used for any security-sensitive purpose here).
    fn left_rotate(x: u32, c: u32) -> u32 {
        (x << c) | (x >> (32 - c))
    }
    let s: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    let mut msg = input.to_vec();
    let bit_len = (input.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());
    let (mut a0, mut b0, mut c0, mut d0) = (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    for chunk in msg.chunks(64) {
        let mut m = [0u32; 16];
        for i in 0..16 {
            m[i] = u32::from_le_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let temp = d;
            d = c;
            c = b;
            b = b.wrapping_add(left_rotate(
                a.wrapping_add(f).wrapping_add(k[i]).wrapping_add(m[g]),
                s[i],
            ));
            a = temp;
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = String::new();
    for v in [a0, b0, c0, d0].iter() {
        for b in v.to_le_bytes().iter() {
            out.push_str(&format!("{:02x}", b));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_generator() {
        let g = PayloadGenerator::Strings {
            values: vec!["a".into(), "b".into()],
        };
        assert_eq!(generate(&g), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn number_range_generator() {
        let g = PayloadGenerator::NumberRange {
            start: 0,
            end: 5,
            step: 2,
        };
        assert_eq!(generate(&g), vec!["0", "2", "4"]);
    }

    #[test]
    fn url_encode_processor() {
        let out = process("a b/c".into(), &PayloadProcessor::UrlEncode).unwrap();
        assert_eq!(out, "a%20b%2Fc");
    }

    #[test]
    fn base64_processor_known_vector() {
        let out = process("hello".into(), &PayloadProcessor::Base64Encode).unwrap();
        assert_eq!(out, "aGVsbG8=");
    }

    #[test]
    fn md5_processor_known_vector() {
        // MD5("") = d41d8cd98f00b204e9800998ecf8427e
        let out = process("".into(), &PayloadProcessor::Md5Hash).unwrap();
        assert_eq!(out, "d41d8cd98f00b204e9800998ecf8427e");
        let out2 = process("abc".into(), &PayloadProcessor::Md5Hash).unwrap();
        assert_eq!(out2, "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn prefix_suffix_processors() {
        let p1 = process("x".into(), &PayloadProcessor::Prefix { value: "pre-".into() }).unwrap();
        assert_eq!(p1, "pre-x");
        let p2 = process("y".into(), &PayloadProcessor::Suffix { value: "-end".into() }).unwrap();
        assert_eq!(p2, "y-end");
    }

    #[test]
    fn filter_drops_non_matching() {
        let out = process(
            "hello".into(),
            &PayloadProcessor::FilterContains {
                needle: "world".into(),
            },
        );
        assert!(out.is_none());
        let out2 = process(
            "hello world".into(),
            &PayloadProcessor::FilterContains {
                needle: "world".into(),
            },
        );
        assert_eq!(out2, Some("hello world".to_string()));
    }

    #[test]
    fn fuzz_substitute_replaces_marker() {
        let out = fuzz_substitute("GET /a?p={FUZZ}&q=1", "value");
        assert_eq!(out, "GET /a?p=value&q=1");
    }

    #[test]
    fn run_job_pipeline_applies_processors() {
        let job = FuzzJob {
            generator: PayloadGenerator::Strings {
                values: vec!["abc".into()],
            },
            processors: vec![
                PayloadProcessor::Base64Encode,
                PayloadProcessor::Suffix { value: "==pad".into() },
            ],
        };
        let out = run_job(&job);
        assert_eq!(out, vec!["YWJj==pad"]);
    }

    #[test]
    fn run_job_filter_drops_payload() {
        let job = FuzzJob {
            generator: PayloadGenerator::Strings {
                values: vec!["safe".into(), "evil".into()],
            },
            processors: vec![PayloadProcessor::FilterContains { needle: "evil".into() }],
        };
        assert_eq!(run_job(&job), vec!["evil"]);
    }

    #[test]
    fn upper_lower_processors() {
        assert_eq!(
            process("AbC".into(), &PayloadProcessor::UpperCase).unwrap(),
            "ABC"
        );
        assert_eq!(
            process("AbC".into(), &PayloadProcessor::LowerCase).unwrap(),
            "abc"
        );
    }
}
