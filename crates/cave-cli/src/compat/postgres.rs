//! psql-compatible Postgres shim (cave-pg).

use anyhow::Result;
use clap::Subcommand;

use crate::ApiClient;

#[derive(Subcommand)]
pub enum PgCompatCmd {
    /// `psql -c "SELECT ..."` — execute a single SQL string
    Exec {
        #[arg(long)]
        sql: String,
        #[arg(long)]
        database: Option<String>,
    },
    /// `psql -l` — list databases
    Databases,
    /// `\du` — list roles
    Roles,
    /// `\dt` — list tables in a database
    Tables {
        #[arg(long)]
        database: String,
    },
    /// `pg_isready` — TCP-level readiness probe
    IsReady,
    /// `pg_dump` — dump a database (delegates to runtime backup endpoint)
    Dump {
        #[arg(long)]
        database: String,
    },
}

pub async fn run(cmd: PgCompatCmd, client: &ApiClient) -> Result<()> {
    match cmd {
        PgCompatCmd::Exec { sql, database } => {
            let body = serde_json::json!({"sql": sql, "database": database});
            client.post("/api/pg/exec", body).await
        }
        PgCompatCmd::Databases => client.get("/api/pg/databases").await,
        PgCompatCmd::Roles => client.get("/api/pg/roles").await,
        PgCompatCmd::Tables { database } => {
            client.get(&format!("/api/pg/databases/{}/tables", database)).await
        }
        PgCompatCmd::IsReady => client.get("/api/pg/ready").await,
        PgCompatCmd::Dump { database } => {
            let body = serde_json::json!({"database": database});
            client.post("/api/pg/backup", body).await
        }
    }
}
