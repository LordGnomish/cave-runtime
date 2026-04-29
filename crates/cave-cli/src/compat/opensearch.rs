//! opensearch-cli compatible shim (cave-search).

use anyhow::Result;
use clap::Subcommand;

use crate::ApiClient;

#[derive(Subcommand)]
pub enum OpensearchCompatCmd {
    /// `curl -XGET /_cluster/health` — cluster health
    ClusterHealth,
    /// `curl -XGET /<index>` — get index info
    IndexGet { index: String },
    /// `curl -XPUT /<index>` — create index
    IndexCreate {
        index: String,
        #[arg(long)]
        body: Option<String>,
    },
    /// `curl -XDELETE /<index>` — delete index
    IndexDelete { index: String },
    /// `curl -XPOST /<index>/_search`
    Search {
        index: String,
        #[arg(long, default_value = "{\"query\": {\"match_all\": {}}}")]
        query: String,
    },
    /// `curl -XPOST /_bulk`
    Bulk {
        #[arg(long)]
        body: String,
    },
    /// `curl -XGET /_cat/indices`
    CatIndices,
}

pub async fn run(cmd: OpensearchCompatCmd, client: &ApiClient) -> Result<()> {
    match cmd {
        OpensearchCompatCmd::ClusterHealth => client.get("/_cluster/health").await,
        OpensearchCompatCmd::IndexGet { index } => client.get(&format!("/{}", index)).await,
        OpensearchCompatCmd::IndexCreate { index, body } => {
            let payload = body
                .and_then(|b| serde_json::from_str::<serde_json::Value>(&b).ok())
                .unwrap_or_else(|| serde_json::json!({}));
            client.post(&format!("/{}", index), payload).await
        }
        OpensearchCompatCmd::IndexDelete { index } => {
            client.get(&format!("/{}?_method=DELETE", index)).await
        }
        OpensearchCompatCmd::Search { index, query } => {
            let payload = serde_json::from_str::<serde_json::Value>(&query)
                .unwrap_or_else(|_| serde_json::json!({"query": {"match_all": {}}}));
            client.post(&format!("/{}/_search", index), payload).await
        }
        OpensearchCompatCmd::Bulk { body } => {
            let payload = serde_json::from_str::<serde_json::Value>(&body)
                .unwrap_or_else(|_| serde_json::json!([]));
            client.post("/_bulk", payload).await
        }
        OpensearchCompatCmd::CatIndices => client.get("/_cat/indices").await,
    }
}
