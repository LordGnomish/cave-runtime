// SPDX-License-Identifier: AGPL-3.0-or-later
//! RED tests for the pulp_container OCI / Docker manifest parser.

use cave_artifacts::pulp::plugins::container::{
    parse_oci_manifest, parse_oci_manifest_list, ManifestKind, OciDescriptor,
    OciManifest, OciManifestIndex,
};

const DOCKER_MANIFEST_V2: &str = r#"{
  "schemaVersion": 2,
  "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
  "config": {
    "mediaType": "application/vnd.docker.container.image.v1+json",
    "size": 1234,
    "digest": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
  },
  "layers": [
    {
      "mediaType": "application/vnd.docker.image.rootfs.diff.tar.gzip",
      "size": 4096,
      "digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111"
    },
    {
      "mediaType": "application/vnd.docker.image.rootfs.diff.tar.gzip",
      "size": 8192,
      "digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222"
    }
  ]
}"#;

const OCI_MANIFEST_V1: &str = r#"{
  "schemaVersion": 2,
  "mediaType": "application/vnd.oci.image.manifest.v1+json",
  "config": {
    "mediaType": "application/vnd.oci.image.config.v1+json",
    "size": 567,
    "digest": "sha256:abcdef0000000000000000000000000000000000000000000000000000000000"
  },
  "layers": [
    {
      "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
      "size": 100,
      "digest": "sha256:fefefefefefefefefefefefefefefefefefefefefefefefefefefefefefefefe"
    }
  ],
  "annotations": {
    "org.opencontainers.image.created": "2026-05-18T00:00:00Z"
  }
}"#;

const OCI_INDEX: &str = r#"{
  "schemaVersion": 2,
  "mediaType": "application/vnd.oci.image.index.v1+json",
  "manifests": [
    {
      "mediaType": "application/vnd.oci.image.manifest.v1+json",
      "size": 800,
      "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "platform": { "architecture": "amd64", "os": "linux" }
    },
    {
      "mediaType": "application/vnd.oci.image.manifest.v1+json",
      "size": 800,
      "digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      "platform": { "architecture": "arm64", "os": "linux" }
    }
  ]
}"#;

#[test]
fn parse_docker_v2_manifest() {
    let m: OciManifest = parse_oci_manifest(DOCKER_MANIFEST_V2.as_bytes()).unwrap();
    assert_eq!(m.kind, ManifestKind::DockerV2);
    assert_eq!(m.schema_version, 2);
    assert_eq!(m.config.size, 1234);
    assert!(m.config.digest.starts_with("sha256:c"));
    assert_eq!(m.layers.len(), 2);
    assert_eq!(m.layers[0].size, 4096);
    assert_eq!(m.layers[1].size, 8192);
}

#[test]
fn parse_oci_v1_manifest() {
    let m = parse_oci_manifest(OCI_MANIFEST_V1.as_bytes()).unwrap();
    assert_eq!(m.kind, ManifestKind::OciV1);
    assert_eq!(m.config.size, 567);
    assert_eq!(m.layers.len(), 1);
    assert_eq!(
        m.annotations.get("org.opencontainers.image.created").map(|s| s.as_str()),
        Some("2026-05-18T00:00:00Z")
    );
}

#[test]
fn parse_oci_index_with_platforms() {
    let idx: OciManifestIndex = parse_oci_manifest_list(OCI_INDEX.as_bytes()).unwrap();
    assert_eq!(idx.manifests.len(), 2);
    assert_eq!(idx.manifests[0].platform.architecture, "amd64");
    assert_eq!(idx.manifests[1].platform.architecture, "arm64");
}

#[test]
fn parse_rejects_invalid_json() {
    assert!(parse_oci_manifest(b"not json").is_err());
}

#[test]
fn parse_rejects_wrong_schema_version() {
    let bad = r#"{ "schemaVersion": 1 }"#;
    assert!(parse_oci_manifest(bad.as_bytes()).is_err());
}

#[test]
fn descriptor_digest_validation() {
    // Helper: every descriptor digest must look like `<algo>:<hex>` with
    // sufficient hex length for the named algorithm.
    let d = OciDescriptor {
        media_type: "application/vnd.oci.image.layer.v1.tar+gzip".into(),
        size: 100,
        digest: "sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789".into(),
        annotations: Default::default(),
    };
    assert!(d.validate_digest().is_ok());
    let bad = OciDescriptor {
        digest: "sha256:short".into(),
        ..d.clone()
    };
    assert!(bad.validate_digest().is_err());
    let badalgo = OciDescriptor {
        digest: "md5:0123456789abcdef0123456789abcdef".into(),
        ..d
    };
    assert!(badalgo.validate_digest().is_err());
}
