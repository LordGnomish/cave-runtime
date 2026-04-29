//! datafusion-cli compatible shim (cave-datafusion).

use anyhow::Result;
use clap::Subcommand;

use crate::ApiClient;

#[derive(Subcommand)]
pub enum DatafusionCompatCmd {
    /// `datafusion-cli -c "<sql>"`
    Sql {
        #[arg(long)]
        query: String,
    },
    /// `\d` — list registered tables
    ShowTables,
    /// `\d <table>` — describe a table
    Describe { table: String },
    /// Register a Parquet file as a table
    RegisterParquet {
        #[arg(long)]
        name: String,
        #[arg(long)]
        path: String,
    },
    /// Register a CSV file as a table
    RegisterCsv {
        #[arg(long)]
        name: String,
        #[arg(long)]
        path: String,
    },
    /// `EXPLAIN <sql>` — return query plan
    Explain {
        #[arg(long)]
        query: String,
    },
}

pub async fn run(cmd: DatafusionCompatCmd, client: &ApiClient) -> Result<()> {
    match cmd {
        DatafusionCompatCmd::Sql { query } => {
            let body = serde_json::json!({"query": query});
            client.post("/api/datafusion/sql", body).await
        }
        DatafusionCompatCmd::ShowTables => client.get("/api/datafusion/tables").await,
        DatafusionCompatCmd::Describe { table } => {
            client.get(&format!("/api/datafusion/tables/{}", table)).await
        }
        DatafusionCompatCmd::RegisterParquet { name, path } => {
            let body = serde_json::json!({"name": name, "path": path, "format": "parquet"});
            client.post("/api/datafusion/tables/register", body).await
        }
        DatafusionCompatCmd::RegisterCsv { name, path } => {
            let body = serde_json::json!({"name": name, "path": path, "format": "csv"});
            client.post("/api/datafusion/tables/register", body).await
        }
        DatafusionCompatCmd::Explain { query } => {
            let body = serde_json::json!({"query": query, "explain": true});
            client.post("/api/datafusion/sql", body).await
        }
    }
}
