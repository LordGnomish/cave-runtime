//! cave-blob portal wrap.

use super::Wrap;

pub fn wrap() -> Wrap {
    Wrap {
        module: "cave-blob".to_string(),
        title: "Object Storage (S3 / MinIO-compatible)".to_string(),
        upstream: "minio/minio".to_string(),
        upstream_commit: "7aac2a2".to_string(),
        description: "S3-compatible object store with multipart uploads, \
                      object lock, lifecycle, replication, KMS, and erasure coding."
            .to_string(),
        dashboard_path: "observability/dashboards/cave-blob.json".to_string(),
        alerts_path: "observability/alerts/cave-blob.yml".to_string(),
        live_data_endpoint: "/api/blob/admin/info".to_string(),
    }
}
