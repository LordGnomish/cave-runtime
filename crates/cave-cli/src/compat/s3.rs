//! mc / s3cmd-compatible shim (cave-blob).

use anyhow::Result;
use clap::Subcommand;

use crate::ApiClient;

#[derive(Subcommand)]
pub enum S3CompatCmd {
    /// `mc ls <bucket>` — list objects in a bucket
    Ls {
        bucket: String,
    },
    /// `mc mb <bucket>` — create bucket
    Mb {
        bucket: String,
    },
    /// `mc rb <bucket>` — remove bucket
    Rb {
        bucket: String,
    },
    /// `mc cp <local> s3://<bucket>/<key>` — upload object
    Put {
        #[arg(long)]
        bucket: String,
        #[arg(long)]
        key: String,
        #[arg(long)]
        body: String,
    },
    /// `mc cat s3://<bucket>/<key>` — download object
    Cat {
        #[arg(long)]
        bucket: String,
        #[arg(long)]
        key: String,
    },
    /// `mc rm s3://<bucket>/<key>` — delete object
    Rm {
        #[arg(long)]
        bucket: String,
        #[arg(long)]
        key: String,
    },
    /// `mc admin info` — server info
    AdminInfo,
}

pub async fn run(cmd: S3CompatCmd, client: &ApiClient) -> Result<()> {
    match cmd {
        S3CompatCmd::Ls { bucket } => {
            client.get(&format!("/api/blob/buckets/{}/objects", bucket)).await
        }
        S3CompatCmd::Mb { bucket } => {
            let body = serde_json::json!({});
            client.post(&format!("/api/blob/buckets/{}", bucket), body).await
        }
        S3CompatCmd::Rb { bucket } => {
            client.get(&format!("/api/blob/buckets/{}/delete", bucket)).await
        }
        S3CompatCmd::Put { bucket, key, body } => {
            let payload = serde_json::json!({"body": body});
            client.post(&format!("/api/blob/buckets/{}/objects/{}", bucket, key), payload).await
        }
        S3CompatCmd::Cat { bucket, key } => {
            client.get(&format!("/api/blob/buckets/{}/objects/{}", bucket, key)).await
        }
        S3CompatCmd::Rm { bucket, key } => {
            client.get(&format!("/api/blob/buckets/{}/objects/{}/delete", bucket, key)).await
        }
        S3CompatCmd::AdminInfo => client.get("/api/blob/admin/info").await,
    }
}
