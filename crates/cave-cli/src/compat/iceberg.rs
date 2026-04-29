//! iceberg-cli / pyiceberg-compatible shim (cave-iceberg).

use anyhow::Result;
use clap::Subcommand;

use crate::ApiClient;

#[derive(Subcommand)]
pub enum IcebergCompatCmd {
    /// `pyiceberg list namespaces`
    ListNamespaces,
    /// `pyiceberg create namespace <namespace>`
    CreateNamespace { namespace: String },
    /// `pyiceberg drop namespace <namespace>`
    DropNamespace { namespace: String },
    /// `pyiceberg list tables <namespace>`
    ListTables { namespace: String },
    /// `pyiceberg load table <namespace>.<table>`
    LoadTable {
        namespace: String,
        table: String,
    },
    /// `pyiceberg create table <namespace>.<table>` (with JSON schema body)
    CreateTable {
        namespace: String,
        table: String,
        #[arg(long)]
        schema: String,
    },
    /// `pyiceberg drop table <namespace>.<table>`
    DropTable {
        namespace: String,
        table: String,
    },
    /// `pyiceberg get config`
    Config,
}

pub async fn run(cmd: IcebergCompatCmd, client: &ApiClient) -> Result<()> {
    match cmd {
        IcebergCompatCmd::ListNamespaces => client.get("/v1/namespaces").await,
        IcebergCompatCmd::CreateNamespace { namespace } => {
            let body = serde_json::json!({"namespace": [namespace]});
            client.post("/v1/namespaces", body).await
        }
        IcebergCompatCmd::DropNamespace { namespace } => {
            client.get(&format!("/v1/namespaces/{}/delete", namespace)).await
        }
        IcebergCompatCmd::ListTables { namespace } => {
            client.get(&format!("/v1/namespaces/{}/tables", namespace)).await
        }
        IcebergCompatCmd::LoadTable { namespace, table } => {
            client.get(&format!("/v1/namespaces/{}/tables/{}", namespace, table)).await
        }
        IcebergCompatCmd::CreateTable { namespace, table, schema } => {
            let body = serde_json::json!({"name": table, "schema": schema});
            client.post(&format!("/v1/namespaces/{}/tables", namespace), body).await
        }
        IcebergCompatCmd::DropTable { namespace, table } => {
            client.get(&format!("/v1/namespaces/{}/tables/{}/delete", namespace, table)).await
        }
        IcebergCompatCmd::Config => client.get("/v1/config").await,
    }
}
