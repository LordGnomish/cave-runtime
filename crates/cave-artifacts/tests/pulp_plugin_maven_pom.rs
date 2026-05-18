// SPDX-License-Identifier: AGPL-3.0-or-later
//! RED tests for pulp_maven POM XML parser + maven-metadata.xml generator.

use cave_artifacts::pulp::plugins::maven::{
    generate_maven_metadata_xml, parse_pom_xml, MavenDependency, MavenPom, SnapshotInfo,
};

const POM_BASIC: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>my-lib</artifactId>
  <version>1.2.3</version>
  <packaging>jar</packaging>
  <name>My Library</name>
  <dependencies>
    <dependency>
      <groupId>org.slf4j</groupId>
      <artifactId>slf4j-api</artifactId>
      <version>2.0.9</version>
    </dependency>
    <dependency>
      <groupId>com.google.guava</groupId>
      <artifactId>guava</artifactId>
      <version>32.1.3-jre</version>
      <scope>compile</scope>
    </dependency>
  </dependencies>
</project>
"#;

const POM_WITH_PARENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <parent>
    <groupId>com.example.parent</groupId>
    <artifactId>parent-pom</artifactId>
    <version>1.0.0</version>
  </parent>
  <artifactId>child</artifactId>
  <version>2.0.0</version>
</project>
"#;

#[test]
fn parse_pom_basic_gav() {
    let pom: MavenPom = parse_pom_xml(POM_BASIC).unwrap();
    assert_eq!(pom.group_id.as_deref(), Some("com.example"));
    assert_eq!(pom.artifact_id, "my-lib");
    assert_eq!(pom.version.as_deref(), Some("1.2.3"));
    assert_eq!(pom.packaging.as_deref(), Some("jar"));
    assert_eq!(pom.name.as_deref(), Some("My Library"));
}

#[test]
fn parse_pom_dependencies() {
    let pom = parse_pom_xml(POM_BASIC).unwrap();
    assert_eq!(pom.dependencies.len(), 2);
    let d0: &MavenDependency = &pom.dependencies[0];
    assert_eq!(d0.group_id, "org.slf4j");
    assert_eq!(d0.artifact_id, "slf4j-api");
    assert_eq!(d0.version.as_deref(), Some("2.0.9"));
    assert_eq!(pom.dependencies[1].scope.as_deref(), Some("compile"));
}

#[test]
fn parse_pom_with_parent_inherits_group() {
    let pom = parse_pom_xml(POM_WITH_PARENT).unwrap();
    // groupId not declared on child; inherit from parent.
    assert_eq!(pom.group_id.as_deref(), Some("com.example.parent"));
    assert_eq!(pom.artifact_id, "child");
    assert_eq!(pom.version.as_deref(), Some("2.0.0"));
    assert!(pom.parent.is_some());
    assert_eq!(pom.parent.as_ref().unwrap().artifact_id, "parent-pom");
}

#[test]
fn parse_pom_rejects_missing_artifact_id() {
    let bad = r#"<project><version>1.0</version></project>"#;
    assert!(parse_pom_xml(bad).is_err());
}

#[test]
fn generate_metadata_xml_release_and_versions() {
    let xml = generate_maven_metadata_xml(
        "com.example",
        "my-lib",
        &["1.0.0".into(), "1.1.0".into(), "1.2.3".into()],
        Some("1.2.3"),
        Some("1.2.3"),
        20260518121530,
    );
    assert!(xml.contains("<groupId>com.example</groupId>"));
    assert!(xml.contains("<artifactId>my-lib</artifactId>"));
    assert!(xml.contains("<release>1.2.3</release>"));
    assert!(xml.contains("<latest>1.2.3</latest>"));
    assert!(xml.contains("<version>1.0.0</version>"));
    assert!(xml.contains("<version>1.2.3</version>"));
    assert!(xml.contains("<lastUpdated>20260518121530</lastUpdated>"));
}

#[test]
fn snapshot_info_parses_timestamp_and_build_number() {
    let info = SnapshotInfo::from_filename("my-lib-1.0.0-20260518.121530-3.jar")
        .expect("snapshot filename parses");
    assert_eq!(info.timestamp, "20260518.121530");
    assert_eq!(info.build_number, 3);
}

#[test]
fn snapshot_info_returns_none_for_release() {
    assert!(SnapshotInfo::from_filename("my-lib-1.0.0.jar").is_none());
}
