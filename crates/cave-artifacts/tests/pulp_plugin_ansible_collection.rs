// SPDX-License-Identifier: AGPL-3.0-or-later
//! RED tests for the pulp_ansible MANIFEST.json + FILES.json + role
//! meta/main.yml readers.

use cave_artifacts::pulp::plugins::ansible::{
    galaxy_v3_response, parse_collection_files, parse_collection_manifest,
    parse_role_meta, CollectionFiles, CollectionInfo, CollectionManifest, RoleMeta,
};

const MANIFEST_JSON: &str = r#"{
  "collection_info": {
    "namespace": "community",
    "name": "general",
    "version": "7.3.0",
    "authors": ["Ansible Community"],
    "readme": "README.md",
    "tags": ["network", "monitoring"],
    "description": "Community general collection",
    "license": ["AGPL-3.0-or-later"],
    "dependencies": {"ansible.netcommon": ">=2.0.0"}
  },
  "file_manifest_file": {
    "name": "FILES.json",
    "ftype": "file",
    "chksum_type": "sha256",
    "chksum_sha256": "deadbeef00000000000000000000000000000000000000000000000000000000",
    "format": 1
  },
  "format": 1
}"#;

const FILES_JSON: &str = r#"{
  "files": [
    { "name": ".", "ftype": "dir", "chksum_type": null, "chksum_sha256": null, "format": 1 },
    { "name": "plugins/modules/example.py", "ftype": "file", "chksum_type": "sha256", "chksum_sha256": "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899", "format": 1 },
    { "name": "README.md", "ftype": "file", "chksum_type": "sha256", "chksum_sha256": "1111111111111111111111111111111111111111111111111111111111111111", "format": 1 }
  ],
  "format": 1
}"#;

const ROLE_META_YAML: &str = r#"
galaxy_info:
  author: Alice
  description: A test role
  license: AGPL-3.0-or-later
  min_ansible_version: "2.10"
  platforms:
    - name: Ubuntu
      versions:
        - 22.04
        - 24.04
    - name: EL
      versions:
        - 9
  galaxy_tags:
    - networking
    - monitoring
dependencies:
  - role: geerlingguy.docker
"#;

#[test]
fn parse_manifest_v1_full() {
    let m: CollectionManifest = parse_collection_manifest(MANIFEST_JSON).unwrap();
    let info: &CollectionInfo = &m.collection_info;
    assert_eq!(info.namespace, "community");
    assert_eq!(info.name, "general");
    assert_eq!(info.version, "7.3.0");
    assert_eq!(info.license, vec!["AGPL-3.0-or-later"]);
    assert_eq!(info.tags, vec!["network", "monitoring"]);
    assert_eq!(info.dependencies.get("ansible.netcommon").map(|s| s.as_str()), Some(">=2.0.0"));
    assert_eq!(m.format, 1);
}

#[test]
fn parse_manifest_rejects_missing_namespace_or_name() {
    let bad = r#"{ "collection_info": {"version": "1.0"}, "format": 1 }"#;
    assert!(parse_collection_manifest(bad).is_err());
}

#[test]
fn parse_files_three_entries() {
    let f: CollectionFiles = parse_collection_files(FILES_JSON).unwrap();
    assert_eq!(f.files.len(), 3);
    // ftype "file" + a sha256 means we can verify it.
    let real_files: Vec<_> = f.files.iter().filter(|e| e.ftype == "file").collect();
    assert_eq!(real_files.len(), 2);
    assert!(real_files[0].chksum_sha256.is_some());
}

#[test]
fn galaxy_v3_response_shape() {
    let m = parse_collection_manifest(MANIFEST_JSON).unwrap();
    let body = galaxy_v3_response(&m, "https://galaxy.example.com");
    assert!(body.contains("\"namespace\""));
    assert!(body.contains("\"community\""));
    assert!(body.contains("\"general\""));
    assert!(body.contains("\"7.3.0\""));
    // Download URL is composed from base_url + namespace + name + version.
    assert!(body.contains("https://galaxy.example.com/download/community-general-7.3.0.tar.gz"));
}

#[test]
fn parse_role_meta_with_platforms() {
    let r: RoleMeta = parse_role_meta(ROLE_META_YAML).unwrap();
    assert_eq!(r.galaxy_info.author.as_deref(), Some("Alice"));
    assert_eq!(r.galaxy_info.license.as_deref(), Some("AGPL-3.0-or-later"));
    assert_eq!(r.galaxy_info.min_ansible_version.as_deref(), Some("2.10"));
    assert_eq!(r.galaxy_info.platforms.len(), 2);
    assert_eq!(r.galaxy_info.platforms[0].name, "Ubuntu");
    assert_eq!(r.galaxy_info.platforms[0].versions, vec!["22.04", "24.04"]);
    assert_eq!(r.dependencies.len(), 1);
}
