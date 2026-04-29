//! mongosh-compatible shim (cave-docdb).

use anyhow::Result;
use clap::Subcommand;

use crate::ApiClient;

#[derive(Subcommand)]
pub enum MongoCompatCmd {
    /// `db.<collection>.find(<filter>)`
    Find {
        #[arg(long)]
        database: String,
        #[arg(long)]
        collection: String,
        #[arg(long, default_value = "{}")]
        filter: String,
    },
    /// `db.<collection>.insertOne(<doc>)`
    InsertOne {
        #[arg(long)]
        database: String,
        #[arg(long)]
        collection: String,
        #[arg(long)]
        document: String,
    },
    /// `db.adminCommand('listDatabases')`
    ListDatabases,
    /// `db.<collection>.aggregate(<pipeline>)`
    Aggregate {
        #[arg(long)]
        database: String,
        #[arg(long)]
        collection: String,
        #[arg(long)]
        pipeline: String,
    },
    /// `db.runCommand({ping: 1})`
    Ping,
    /// `db.runCommand({hello: 1})`
    Hello,
    /// `rs.status()`
    ReplSetStatus,
}

pub async fn run(cmd: MongoCompatCmd, client: &ApiClient) -> Result<()> {
    match cmd {
        MongoCompatCmd::Find { database, collection, filter } => {
            let body = serde_json::json!({"filter": filter});
            client.post(&format!("/api/docdb/{}/{}/find", database, collection), body).await
        }
        MongoCompatCmd::InsertOne { database, collection, document } => {
            let body = serde_json::json!({"document": document});
            client.post(&format!("/api/docdb/{}/{}/insertOne", database, collection), body).await
        }
        MongoCompatCmd::ListDatabases => client.get("/api/docdb/databases").await,
        MongoCompatCmd::Aggregate { database, collection, pipeline } => {
            let body = serde_json::json!({"pipeline": pipeline});
            client.post(&format!("/api/docdb/{}/{}/aggregate", database, collection), body).await
        }
        MongoCompatCmd::Ping => client.get("/api/docdb/ping").await,
        MongoCompatCmd::Hello => client.get("/api/docdb/hello").await,
        MongoCompatCmd::ReplSetStatus => client.get("/api/docdb/replSetGetStatus").await,
    }
}
