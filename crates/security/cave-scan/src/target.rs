// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: aquasecurity/trivy@8a3177a pkg/fanal/artifact/local/fs.go (kind detection)

//! Target classification — image tar, image reference, filesystem, SBOM.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    ImageTar,
    ImageReference,
    Filesystem,
    Sbom,
}

pub fn detect_target(input: &str) -> TargetKind {
    let lower = input.to_ascii_lowercase();

    if lower.ends_with(".cdx.json")
        || lower.ends_with(".spdx.json")
        || lower.ends_with(".spdx")
        || lower.ends_with(".cdx")
    {
        return TargetKind::Sbom;
    }

    if lower.ends_with(".tar")
        || lower.ends_with(".tar.gz")
        || lower.ends_with(".tgz")
        || lower.ends_with(".oci.tar")
    {
        return TargetKind::ImageTar;
    }

    // Filesystem paths typically begin with `/`, `./`, `../`, or a Windows drive.
    if input.starts_with('/')
        || input.starts_with("./")
        || input.starts_with("../")
        || (input.len() >= 3
            && input.as_bytes()[1] == b':'
            && (input.as_bytes()[2] == b'\\' || input.as_bytes()[2] == b'/'))
    {
        return TargetKind::Filesystem;
    }

    // Heuristic: `name:tag` or `registry/repo:tag` → image reference.
    if input.contains(':') {
        return TargetKind::ImageReference;
    }
    if input.contains('/') {
        return TargetKind::ImageReference;
    }

    // Fallback: treat as filesystem path.
    TargetKind::Filesystem
}
