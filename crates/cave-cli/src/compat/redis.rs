//! redis-cli / valkey-cli compatible shim (cave-cache).

use anyhow::Result;
use clap::Subcommand;

use crate::ApiClient;

#[derive(Subcommand)]
pub enum RedisCompatCmd {
    /// `redis-cli GET <key>`
    Get { key: String },
    /// `redis-cli SET <key> <value>`
    Set { key: String, value: String },
    /// `redis-cli DEL <key>`
    Del { key: String },
    /// `redis-cli KEYS <pattern>`
    Keys { pattern: String },
    /// `redis-cli PING`
    Ping,
    /// `redis-cli INFO [section]`
    Info {
        #[arg(long)]
        section: Option<String>,
    },
    /// `redis-cli CLUSTER INFO`
    ClusterInfo,
    /// `redis-cli MONITOR` — stream commands (not supported, returns 501)
    Monitor,
}

pub async fn run(cmd: RedisCompatCmd, client: &ApiClient) -> Result<()> {
    match cmd {
        RedisCompatCmd::Get { key } => client.get(&format!("/api/cache/keys/{}", key)).await,
        RedisCompatCmd::Set { key, value } => {
            let body = serde_json::json!({"value": value});
            client.post(&format!("/api/cache/keys/{}", key), body).await
        }
        RedisCompatCmd::Del { key } => client.get(&format!("/api/cache/keys/{}/delete", key)).await,
        RedisCompatCmd::Keys { pattern } => {
            client.get(&format!("/api/cache/keys?pattern={}", pattern)).await
        }
        RedisCompatCmd::Ping => client.get("/api/cache/ping").await,
        RedisCompatCmd::Info { section } => {
            let path = match section {
                Some(s) => format!("/api/cache/info?section={}", s),
                None => "/api/cache/info".to_string(),
            };
            client.get(&path).await
        }
        RedisCompatCmd::ClusterInfo => client.get("/api/cache/cluster/info").await,
        RedisCompatCmd::Monitor => client.get("/api/cache/monitor").await,
    }
}
