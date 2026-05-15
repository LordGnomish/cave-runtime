// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: aquasecurity/trivy@8a3177a pkg/fanal/image/img.go (compression detect)

//! Layer compression detection by magic bytes.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerCompression {
    None,
    Gzip,
    Zstd,
    Bzip2,
    Xz,
}

pub fn detect_layer_compression(bytes: &[u8]) -> LayerCompression {
    if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        return LayerCompression::Gzip;
    }
    if bytes.len() >= 4
        && bytes[0] == 0x28
        && bytes[1] == 0xb5
        && bytes[2] == 0x2f
        && bytes[3] == 0xfd
    {
        return LayerCompression::Zstd;
    }
    if bytes.len() >= 3 && bytes[0] == b'B' && bytes[1] == b'Z' && bytes[2] == b'h' {
        return LayerCompression::Bzip2;
    }
    if bytes.len() >= 6
        && bytes[0] == 0xfd
        && bytes[1] == b'7'
        && bytes[2] == b'z'
        && bytes[3] == b'X'
        && bytes[4] == b'Z'
        && bytes[5] == 0x00
    {
        return LayerCompression::Xz;
    }
    LayerCompression::None
}
