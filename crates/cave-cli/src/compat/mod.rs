//! Compatibility shims that mimic upstream CLI surfaces (psql, mongosh,
//! redis-cli, mc, opensearch-cli, iceberg-cli, datafusion-cli) so that
//! existing automation can talk to a CAVE deployment without rewriting
//! commands.
//!
//! Each submodule registers a clap [`Subcommand`] that the root CLI in
//! `main.rs` wires under `cave compat <name> ...`. A shim is intentionally
//! minimal — `get / list / exec` (or the equivalent upstream verbs) — and
//! delegates to the runtime HTTP API. Upstream-CLI flag-by-flag parity is
//! out of scope; the goal is "commands a curious operator would type".

pub mod postgres;
pub mod redis;
pub mod mongo;
pub mod s3;
pub mod opensearch;
pub mod iceberg;
pub mod datafusion;

use anyhow::Result;
use clap::Subcommand;

use crate::ApiClient;

#[derive(Subcommand)]
pub enum CompatCmd {
    /// psql-compatible Postgres shim (cave-pg)
    Pg {
        #[command(subcommand)]
        cmd: postgres::PgCompatCmd,
    },
    /// redis-cli / valkey-cli compatible shim (cave-cache)
    Redis {
        #[command(subcommand)]
        cmd: redis::RedisCompatCmd,
    },
    /// mongosh-compatible shim (cave-docdb)
    Mongo {
        #[command(subcommand)]
        cmd: mongo::MongoCompatCmd,
    },
    /// mc / s3cmd-compatible shim (cave-blob)
    S3 {
        #[command(subcommand)]
        cmd: s3::S3CompatCmd,
    },
    /// opensearch-cli compatible shim (cave-search)
    Opensearch {
        #[command(subcommand)]
        cmd: opensearch::OpensearchCompatCmd,
    },
    /// iceberg-cli compatible shim (cave-iceberg)
    Iceberg {
        #[command(subcommand)]
        cmd: iceberg::IcebergCompatCmd,
    },
    /// datafusion-cli compatible shim (cave-datafusion)
    Datafusion {
        #[command(subcommand)]
        cmd: datafusion::DatafusionCompatCmd,
    },
}

pub async fn run(cmd: CompatCmd, client: &ApiClient) -> Result<()> {
    match cmd {
        CompatCmd::Pg { cmd } => postgres::run(cmd, client).await,
        CompatCmd::Redis { cmd } => redis::run(cmd, client).await,
        CompatCmd::Mongo { cmd } => mongo::run(cmd, client).await,
        CompatCmd::S3 { cmd } => s3::run(cmd, client).await,
        CompatCmd::Opensearch { cmd } => opensearch::run(cmd, client).await,
        CompatCmd::Iceberg { cmd } => iceberg::run(cmd, client).await,
        CompatCmd::Datafusion { cmd } => datafusion::run(cmd, client).await,
    }
}
