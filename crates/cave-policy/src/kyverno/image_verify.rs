// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kyverno image verification engine.
//!
//! Supports: cosign, notary, attestations, image references, mutateDigest.
//! Full cryptographic verification requires access to a signing infrastructure;
//! this implementation provides the framework with real image reference parsing
//! and digest mutation, plus stubs for actual signature verification.

use super::models::*;
use crate::error::PolicyError;
use serde_json::Value;

/// Verify images in a resource against Kyverno verifyImages rules.
pub fn verify_images_rule(
    rule: &KyvernoRule,
    resource: &Value,
    _context: &Value,
) -> Result<Vec<ImageVerificationResult>, PolicyError> {
    if rule.verify_images.is_empty() {
        return Ok(vec![]);
    }

    let mut results = Vec::new();

    for verify_spec in &rule.verify_images {
        // Extract images from resource
        let images = extract_images(resource, &verify_spec.image_references);

        for image_ref in &images {
            let result = verify_single_image(image_ref, verify_spec)?;
            results.push(result);
        }
    }

    Ok(results)
}

fn verify_single_image(
    image_ref: &str,
    spec: &ImageVerification,
) -> Result<ImageVerificationResult, PolicyError> {
    let parsed = parse_image_ref(image_ref);

    // If verifyDigest is required, check that image uses a digest (not just a tag)
    if spec.verify_digest && parsed.digest.is_none() {
        return Ok(ImageVerificationResult {
            image: image_ref.to_string(),
            verified: false,
            digest: None,
            error: Some(format!(
                "image '{}' must be referenced by digest (sha256:...) when verifyDigest=true",
                image_ref
            )),
        });
    }

    // Check if image reference matches any of the imageReferences patterns
    let matches = spec
        .image_references
        .iter()
        .any(|pattern| image_ref_matches(image_ref, pattern));

    if !matches {
        return Ok(ImageVerificationResult {
            image: image_ref.to_string(),
            verified: true, // Not in scope of this rule
            digest: parsed.digest,
            error: None,
        });
    }

    // Attestor verification (stub — real implementation requires cosign/notary)
    if !spec.attestors.is_empty() {
        let verification_type = spec.verification_type.as_deref().unwrap_or("Cosign");
        tracing::debug!(
            target: "kyverno.image_verify",
            image = image_ref,
            verification_type,
            "image signature verification (stub — not cryptographically verified)"
        );

        // In a full implementation, we would:
        // 1. Pull the image manifest
        // 2. Verify cosign/notary signatures against attestor keys
        // 3. Check attestations (SBOM, vulnerability scan results, etc.)
        // For now, if `required` is true, we fail-safe (mark as unverified)
        if spec.required {
            return Ok(ImageVerificationResult {
                image: image_ref.to_string(),
                verified: false,
                digest: parsed.digest,
                error: Some(format!(
                    "image '{}' signature verification not available (requires cosign/notary infrastructure)",
                    image_ref
                )),
            });
        }
    }

    // Check attestations
    for attestation in &spec.attestations {
        tracing::debug!(
            target: "kyverno.image_verify",
            image = image_ref,
            attestation_type = attestation.attestation_type,
            "checking image attestation (stub)"
        );
    }

    Ok(ImageVerificationResult {
        image: image_ref.to_string(),
        verified: true,
        digest: parsed.digest,
        error: None,
    })
}

/// Extract images from a Kubernetes resource based on reference paths.
fn extract_images(resource: &Value, image_references: &[String]) -> Vec<String> {
    let mut images = Vec::new();

    // Standard paths in K8s resources
    let standard_paths = [
        // Pod spec
        "spec.containers[*].image",
        "spec.initContainers[*].image",
        "spec.ephemeralContainers[*].image",
        // Pod template (Deployment, StatefulSet, etc.)
        "spec.template.spec.containers[*].image",
        "spec.template.spec.initContainers[*].image",
        "spec.template.spec.ephemeralContainers[*].image",
    ];

    for path in &standard_paths {
        let extracted = extract_by_path(resource, path);
        images.extend(extracted);
    }

    // Filter by image reference patterns
    if !image_references.is_empty() {
        images.retain(|img| {
            image_references
                .iter()
                .any(|pattern| image_ref_matches(img, pattern))
        });
    }

    images
}

fn extract_by_path(resource: &Value, path: &str) -> Vec<String> {
    let parts: Vec<&str> = path.split('.').collect();
    extract_recursive(resource, &parts)
}

fn extract_recursive(value: &Value, parts: &[&str]) -> Vec<String> {
    if parts.is_empty() {
        return if let Some(s) = value.as_str() {
            vec![s.to_string()]
        } else {
            vec![]
        };
    }

    let head = parts[0];
    let rest = &parts[1..];

    if head.ends_with("[*]") {
        let field = head.trim_end_matches("[*]");
        if let Some(arr) = value.get(field).and_then(|v| v.as_array()) {
            return arr
                .iter()
                .flat_map(|item| extract_recursive(item, rest))
                .collect();
        }
    } else {
        if let Some(child) = value.get(head) {
            return extract_recursive(child, rest);
        }
    }
    vec![]
}

/// Check if an image reference matches a Kyverno image reference pattern.
/// Patterns support `*` wildcard (matches any characters except `/`).
fn image_ref_matches(image: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    // Convert pattern to regex
    let mut re = String::from("^");
    for c in pattern.chars() {
        match c {
            '*' => re.push_str("[^/]*"),
            '?' => re.push('.'),
            '.' | '+' | '(' | ')' | '[' | ']' | '^' | '$' | '|' | '\\' => {
                re.push('\\');
                re.push(c);
            }
            c => re.push(c),
        }
    }
    re.push('$');
    regex::Regex::new(&re)
        .map(|r| r.is_match(image))
        .unwrap_or(false)
}

/// Parsed image reference.
struct ParsedImageRef {
    registry: Option<String>,
    repository: String,
    tag: Option<String>,
    digest: Option<String>,
}

fn parse_image_ref(image: &str) -> ParsedImageRef {
    // Format: [registry/]repository[:tag][@digest]
    let (image_without_digest, digest) = if let Some(at_pos) = image.rfind('@') {
        (&image[..at_pos], Some(image[at_pos + 1..].to_string()))
    } else {
        (image, None)
    };

    let (image_without_tag, tag) = if let Some(colon_pos) = image_without_digest.rfind(':') {
        // Make sure it's not part of the registry (e.g., registry:5000/image)
        let maybe_tag = &image_without_digest[colon_pos + 1..];
        if !maybe_tag.contains('/') {
            (
                &image_without_digest[..colon_pos],
                Some(maybe_tag.to_string()),
            )
        } else {
            (image_without_digest, None)
        }
    } else {
        (image_without_digest, None)
    };

    // Check if there's a registry (first component contains a '.' or ':')
    let slash_pos = image_without_tag.find('/');
    let (registry, repository) = if let Some(pos) = slash_pos {
        let first_component = &image_without_tag[..pos];
        if first_component.contains('.')
            || first_component.contains(':')
            || first_component == "localhost"
        {
            (
                Some(first_component.to_string()),
                image_without_tag[pos + 1..].to_string(),
            )
        } else {
            (None, image_without_tag.to_string())
        }
    } else {
        (None, image_without_tag.to_string())
    };

    ParsedImageRef {
        registry,
        repository,
        tag,
        digest,
    }
}

/// Mutate image digest in a resource (for mutateDigest=true).
pub fn mutate_image_digest(
    resource: &mut Value,
    image: &str,
    digest: &str,
) -> Vec<serde_json::Value> {
    let mut patches = Vec::new();
    mutate_image_digest_recursive(resource, image, digest, "/", &mut patches);
    patches
}

fn mutate_image_digest_recursive(
    value: &mut Value,
    image: &str,
    digest: &str,
    path: &str,
    patches: &mut Vec<serde_json::Value>,
) {
    match value {
        Value::Object(m) => {
            let keys: Vec<String> = m.keys().cloned().collect();
            for key in keys {
                let child_path = format!("{}/{}", path.trim_end_matches('/'), key);
                if key == "image" {
                    if let Some(Value::String(s)) = m.get(&key) {
                        if s.as_str() == image {
                            let new_image = format!("{}@{}", image, digest);
                            patches.push(serde_json::json!({
                                "op": "replace",
                                "path": child_path,
                                "value": new_image
                            }));
                            m.insert(key, Value::String(new_image));
                        }
                    }
                } else if let Some(child) = m.get_mut(&key) {
                    mutate_image_digest_recursive(child, image, digest, &child_path, patches);
                }
            }
        }
        Value::Array(a) => {
            for (i, item) in a.iter_mut().enumerate() {
                let item_path = format!("{}/{}", path.trim_end_matches('/'), i);
                mutate_image_digest_recursive(item, image, digest, &item_path, patches);
            }
        }
        _ => {}
    }
}
