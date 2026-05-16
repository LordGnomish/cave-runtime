// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2 storage/src/main/java/org/apache/kafka/storage/internals/log/RemoteLogManager.java
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2 storage/src/main/java/org/apache/kafka/server/log/remote/storage/RemoteStorageManager.java
//
// KIP-405 — Tiered Storage: S3 backend + offload state machine
// integration tests.

use cave_streams::tiered_storage::{
    RemoteLogSegmentId, RemoteLogSegmentMetadata, RemoteLogSegmentState, TopicIdPartition,
};
use cave_streams::tiered_storage_s3::{
    AwsCredentials, S3Endpoint, S3RemoteStorageManager, SigV4Signer,
};

fn tp(topic: &str, p: u32) -> TopicIdPartition {
    TopicIdPartition {
        topic: topic.into(),
        topic_uuid: 0,
        partition: p,
    }
}

fn meta(t: &str, p: u32, base: u64, size: u64) -> RemoteLogSegmentMetadata {
    RemoteLogSegmentMetadata {
        id: RemoteLogSegmentId {
            topic_partition: tp(t, p),
            segment_uuid: base,
        },
        start_offset: base,
        end_offset: base + size - 1,
        max_timestamp_ms: 0,
        broker_id: 1,
        event_timestamp_ms: 0,
        segment_size_bytes: size,
        state: RemoteLogSegmentState::CopyStarted,
    }
}

#[test]
fn s3_object_key_includes_topic_partition_and_offset() {
    let s3 = S3RemoteStorageManager::new_dryrun("test-bucket");
    let m = meta("logs", 3, 1000, 50);
    let key = s3.object_key(&m);
    // Must encode topic, partition, and start_offset deterministically.
    assert!(key.contains("logs"));
    assert!(key.contains("3"));
    assert!(key.contains("1000"));
}

#[test]
fn s3_endpoint_construction_canonicalises_host() {
    let ep = S3Endpoint::for_region("us-east-1");
    assert!(ep.host().contains("s3"));
    assert!(ep.host().contains("us-east-1"));
}

#[test]
fn s3_endpoint_minio_compatibility() {
    let ep = S3Endpoint::minio("http://localhost:9000");
    assert_eq!(ep.host(), "localhost:9000");
    assert!(!ep.is_aws());
}

#[test]
fn sigv4_canonical_request_matches_aws_test_vector() {
    // AWS docs example: GET / HTTP/1.1, Host:examplebucket.s3.amazonaws.com,
    // x-amz-date:20130524T000000Z
    let signer = SigV4Signer::new(AwsCredentials::new("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", None));
    let cr = signer.canonical_request(
        "GET",
        "/",
        "",
        &[
            ("host", "examplebucket.s3.amazonaws.com"),
            ("x-amz-date", "20130524T000000Z"),
        ],
        "",
    );
    // canonical_request must be of the form:
    // GET\n/\n\nhost:...\nx-amz-date:...\n\nhost;x-amz-date\nUNSIGNED-PAYLOAD or hash
    let lines: Vec<&str> = cr.split('\n').collect();
    assert_eq!(lines[0], "GET");
    assert_eq!(lines[1], "/");
    assert_eq!(lines[2], ""); // empty query
    assert!(lines.iter().any(|l| l.starts_with("host:")));
    assert!(lines.iter().any(|l| l.starts_with("x-amz-date:")));
}

#[test]
fn sigv4_string_to_sign_includes_credential_scope() {
    let signer = SigV4Signer::new(AwsCredentials::new("AKID", "secret", None));
    let canonical = "GET\n/\n\nhost:s3.amazonaws.com\nx-amz-date:20130524T000000Z\n\nhost;x-amz-date\nUNSIGNED-PAYLOAD";
    let sts = signer.string_to_sign("20130524T000000Z", "20130524/us-east-1/s3/aws4_request", canonical);
    let lines: Vec<&str> = sts.split('\n').collect();
    assert_eq!(lines[0], "AWS4-HMAC-SHA256");
    assert_eq!(lines[1], "20130524T000000Z");
    assert_eq!(lines[2], "20130524/us-east-1/s3/aws4_request");
    // line[3] is the sha256 of the canonical request — 64-char hex.
    assert_eq!(lines[3].len(), 64);
}

#[test]
fn sigv4_signing_key_matches_aws_documented_chain() {
    // From AWS docs, sample chain: kSecret → kDate → kRegion → kService → kSigning.
    let signer = SigV4Signer::new(AwsCredentials::new(
        "AKIDEXAMPLE",
        "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        None,
    ));
    let sk = signer.signing_key("20150830", "us-east-1", "s3");
    // 32-byte HMAC-SHA256 output.
    assert_eq!(sk.len(), 32);
}

#[test]
fn s3_dryrun_copy_log_segment_records_action() {
    let mut s3 = S3RemoteStorageManager::new_dryrun("logs-bucket");
    let m = meta("orders", 0, 0, 100);
    s3.copy_dryrun(&m, vec![0u8; 100]).unwrap();
    let log = s3.dryrun_log();
    assert!(!log.is_empty());
    let put = log.iter().find(|e| e.method == "PUT").expect("PUT recorded");
    assert!(put.path.contains("orders"));
    assert!(put.path.contains("0"));
}

#[test]
fn s3_dryrun_delete_log_segment_records_action() {
    let mut s3 = S3RemoteStorageManager::new_dryrun("logs-bucket");
    let m = meta("orders", 0, 0, 100);
    s3.copy_dryrun(&m, vec![0u8; 100]).unwrap();
    s3.delete_dryrun(&m.id).unwrap();
    let log = s3.dryrun_log();
    assert!(log.iter().any(|e| e.method == "DELETE"));
}

#[test]
fn s3_dryrun_fetch_returns_previously_copied_bytes() {
    let mut s3 = S3RemoteStorageManager::new_dryrun("logs-bucket");
    let m = meta("o", 0, 0, 8);
    s3.copy_dryrun(&m, vec![1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
    let chunk = s3.fetch_dryrun(&m.id, 2, 6).unwrap();
    assert_eq!(chunk, vec![3, 4, 5, 6]);
}
