// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: aquasecurity/trivy@8a3177a pkg/fanal/analyzer/executable/executable.go

//! Executable / binary recognition by magic bytes (ELF / PE / Mach-O).

use super::{Analyzer, AnalyzerType};

pub struct BinaryAnalyzer;

impl BinaryAnalyzer {
    /// True if the first few bytes match a known executable magic.
    pub fn is_executable(&self, bytes: &[u8]) -> bool {
        if bytes.len() >= 4 && &bytes[..4] == b"\x7fELF" {
            return true;
        }
        if bytes.len() >= 2 && &bytes[..2] == b"MZ" {
            return true;
        }
        if bytes.len() >= 4 {
            let m = &bytes[..4];
            if m == [0xfe, 0xed, 0xfa, 0xce]
                || m == [0xfe, 0xed, 0xfa, 0xcf]
                || m == [0xcf, 0xfa, 0xed, 0xfe]
                || m == [0xce, 0xfa, 0xed, 0xfe]
            {
                return true;
            }
        }
        false
    }
}

impl Analyzer for BinaryAnalyzer {
    fn kind(&self) -> AnalyzerType {
        AnalyzerType::Binary
    }
    // Binary detection is dispatched by reading file magic, not by path
    // patterns. The registry skips path-based dispatch for executables.
    fn required(&self, _path: &str) -> bool {
        false
    }
}
