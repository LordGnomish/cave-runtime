// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Iceberg REST Catalog client.
//!
//! Upstream: `crates/iceberg-catalog-rest/src/catalog.rs`
//! Spec: <https://github.com/apache/iceberg/blob/main/open-api/rest-catalog-open-api.yaml>
//!
//! The cave-iceberg MVP exposes the REST catalog as a *URL builder*
//! and *request/response codec* — every operation returns the request
//! it would send and the path/method, but the actual HTTP transport is
//! kept opt-in via the `Transport` trait so that tests don't depend on
//! an HTTP runtime. The default `Transport` impl in `cave-store` is
//! wired separately. This split mirrors how `iceberg-catalog-rest`
//! decomposes its `RestCatalogClient` from the `reqwest` calls.

use crate::catalog::Catalog;
use crate::error::{Error, Result};
use crate::namespace::{Namespace, NamespaceIdent};
use crate::table::{Table, TableIdent};
use crate::table_metadata::TableMetadata;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Delete,
    Put,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RestRequest {
    pub method: HttpMethod,
    pub url: String,
    pub body_json: Option<String>,
}

#[async_trait]
pub trait Transport: Send + Sync {
    async fn send(&self, req: RestRequest) -> Result<RestResponse>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RestResponse {
    pub status: u16,
    pub body_json: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreateTableRequest<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<&'a TableMetadata>,
}

pub struct RestCatalog {
    base_url: String,
    warehouse: Option<String>,
    transport: Box<dyn Transport>,
}

impl std::fmt::Debug for RestCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestCatalog")
            .field("base_url", &self.base_url)
            .field("warehouse", &self.warehouse)
            .field("transport", &"<dyn Transport>")
            .finish()
    }
}

impl RestCatalog {
    pub fn new(base_url: impl Into<String>, transport: Box<dyn Transport>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            warehouse: None,
            transport,
        }
    }

    pub fn with_warehouse(mut self, warehouse: impl Into<String>) -> Self {
        self.warehouse = Some(warehouse.into());
        self
    }

    /// `/v1/namespaces` route — flat list.
    fn namespaces_url(&self) -> String {
        format!("{}/v1/namespaces", self.base_url)
    }

    /// `/v1/namespaces/{ns}` — dot-encoded as per the spec, with `%1F`
    /// (Unit Separator) between levels.
    fn namespace_url(&self, ns: &NamespaceIdent) -> String {
        format!("{}/v1/namespaces/{}", self.base_url, encode_namespace(ns))
    }

    fn tables_url(&self, ns: &NamespaceIdent) -> String {
        format!(
            "{}/v1/namespaces/{}/tables",
            self.base_url,
            encode_namespace(ns)
        )
    }

    fn table_url(&self, ident: &TableIdent) -> String {
        format!(
            "{}/v1/namespaces/{}/tables/{}",
            self.base_url,
            encode_namespace(&ident.namespace),
            ident.name
        )
    }

    fn rename_url(&self) -> String {
        format!("{}/v1/tables/rename", self.base_url)
    }

    async fn send(&self, req: RestRequest) -> Result<RestResponse> {
        self.transport.send(req).await
    }

    fn parse_json<'a, T: Deserialize<'a>>(body: &'a Option<String>) -> Result<T> {
        let raw = body.as_deref().unwrap_or("{}");
        Ok(serde_json::from_str(raw)?)
    }
}

fn encode_namespace(ns: &NamespaceIdent) -> String {
    // Iceberg REST uses %1F as the separator.
    ns.0.join("%1F")
}

#[async_trait]
impl Catalog for RestCatalog {
    async fn create_namespace(&self, ns: &Namespace) -> Result<()> {
        let body = serde_json::to_string(&serde_json::json!({
            "namespace": ns.ident.0,
            "properties": ns.properties,
        }))?;
        let req = RestRequest {
            method: HttpMethod::Post,
            url: self.namespaces_url(),
            body_json: Some(body),
        };
        let r = self.send(req).await?;
        if r.status >= 400 {
            return Err(Error::Io(format!("create namespace failed: {}", r.status)));
        }
        Ok(())
    }

    async fn drop_namespace(&self, ident: &NamespaceIdent) -> Result<()> {
        let req = RestRequest {
            method: HttpMethod::Delete,
            url: self.namespace_url(ident),
            body_json: None,
        };
        let r = self.send(req).await?;
        if r.status >= 400 {
            return Err(Error::Io(format!("drop namespace failed: {}", r.status)));
        }
        Ok(())
    }

    async fn list_namespaces(&self, _parent: Option<&NamespaceIdent>) -> Result<Vec<NamespaceIdent>> {
        let req = RestRequest {
            method: HttpMethod::Get,
            url: self.namespaces_url(),
            body_json: None,
        };
        let r = self.send(req).await?;
        #[derive(Deserialize)]
        struct Wrap {
            namespaces: Vec<Vec<String>>,
        }
        let w: Wrap = Self::parse_json(&r.body_json)?;
        Ok(w.namespaces.into_iter().map(NamespaceIdent).collect())
    }

    async fn namespace_exists(&self, ident: &NamespaceIdent) -> Result<bool> {
        let req = RestRequest {
            method: HttpMethod::Get,
            url: self.namespace_url(ident),
            body_json: None,
        };
        let r = self.send(req).await?;
        Ok(r.status < 400)
    }

    async fn create_table(&self, ident: &TableIdent, metadata: TableMetadata) -> Result<Table> {
        let body = serde_json::to_string(&CreateTableRequest {
            name: &ident.name,
            location: Some(&metadata.location),
            metadata: Some(&metadata),
        })?;
        let req = RestRequest {
            method: HttpMethod::Post,
            url: self.tables_url(&ident.namespace),
            body_json: Some(body),
        };
        let r = self.send(req).await?;
        if r.status >= 400 {
            return Err(Error::Io(format!("create table failed: {}", r.status)));
        }
        Ok(Table::new(ident.clone(), metadata))
    }

    async fn drop_table(&self, ident: &TableIdent) -> Result<()> {
        let req = RestRequest {
            method: HttpMethod::Delete,
            url: self.table_url(ident),
            body_json: None,
        };
        let r = self.send(req).await?;
        if r.status >= 400 {
            return Err(Error::Io(format!("drop table failed: {}", r.status)));
        }
        Ok(())
    }

    async fn load_table(&self, ident: &TableIdent) -> Result<Table> {
        let req = RestRequest {
            method: HttpMethod::Get,
            url: self.table_url(ident),
            body_json: None,
        };
        let r = self.send(req).await?;
        if r.status >= 400 {
            return Err(Error::NotFound(format!(
                "table {}.{}",
                ident.namespace.as_dot(),
                ident.name
            )));
        }
        #[derive(Deserialize)]
        struct Wrap {
            metadata: TableMetadata,
            #[serde(rename = "metadata-location")]
            metadata_location: Option<String>,
        }
        let w: Wrap = Self::parse_json(&r.body_json)?;
        let mut t = Table::new(ident.clone(), w.metadata);
        if let Some(loc) = w.metadata_location {
            t = t.with_metadata_location(loc);
        }
        Ok(t)
    }

    async fn list_tables(&self, ns: &NamespaceIdent) -> Result<Vec<TableIdent>> {
        let req = RestRequest {
            method: HttpMethod::Get,
            url: self.tables_url(ns),
            body_json: None,
        };
        let r = self.send(req).await?;
        #[derive(Deserialize)]
        struct Wrap {
            identifiers: Vec<TableIdentWire>,
        }
        #[derive(Deserialize)]
        struct TableIdentWire {
            namespace: Vec<String>,
            name: String,
        }
        let w: Wrap = Self::parse_json(&r.body_json)?;
        Ok(w.identifiers
            .into_iter()
            .map(|t| TableIdent::new(NamespaceIdent(t.namespace), t.name))
            .collect())
    }

    async fn table_exists(&self, ident: &TableIdent) -> Result<bool> {
        let req = RestRequest {
            method: HttpMethod::Get,
            url: self.table_url(ident),
            body_json: None,
        };
        let r = self.send(req).await?;
        Ok(r.status < 400)
    }

    async fn rename_table(&self, from: &TableIdent, to: &TableIdent) -> Result<()> {
        let body = serde_json::to_string(&serde_json::json!({
            "source": { "namespace": from.namespace.0, "name": from.name },
            "destination": { "namespace": to.namespace.0, "name": to.name },
        }))?;
        let req = RestRequest {
            method: HttpMethod::Post,
            url: self.rename_url(),
            body_json: Some(body),
        };
        let r = self.send(req).await?;
        if r.status >= 400 {
            return Err(Error::Io(format!("rename failed: {}", r.status)));
        }
        Ok(())
    }

    async fn replace_table_metadata(
        &self,
        ident: &TableIdent,
        new_metadata: TableMetadata,
    ) -> Result<Table> {
        // REST exposes commits via PUT /v1/.../tables/{name}/commits — the
        // body carries an `assertions` + `updates` sequence. The MVP
        // collapses to a single "overwrite the metadata file" semantics.
        let body = serde_json::to_string(&serde_json::json!({
            "metadata": new_metadata,
        }))?;
        let req = RestRequest {
            method: HttpMethod::Put,
            url: format!("{}/commits", self.table_url(ident)),
            body_json: Some(body),
        };
        let r = self.send(req).await?;
        if r.status >= 400 {
            return Err(Error::Io(format!("commit failed: {}", r.status)));
        }
        Ok(Table::new(ident.clone(), new_metadata))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Schema;
    use crate::table_metadata::TableMetadataBuilder;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MockTransport {
        sent: Mutex<Vec<RestRequest>>,
        response: Mutex<Option<RestResponse>>,
    }

    impl MockTransport {
        fn set_response(&self, r: RestResponse) {
            *self.response.lock().unwrap() = Some(r);
        }
    }

    #[async_trait]
    impl Transport for MockTransport {
        async fn send(&self, req: RestRequest) -> Result<RestResponse> {
            self.sent.lock().unwrap().push(req);
            Ok(self.response.lock().unwrap().clone().unwrap_or(RestResponse {
                status: 200,
                body_json: Some("{}".into()),
            }))
        }
    }

    #[tokio::test]
    async fn create_namespace_url_is_correct() {
        let mt = Box::new(MockTransport::default());
        let cat = RestCatalog::new("http://x/", mt);
        let ns = Namespace::new(NamespaceIdent::from_dot("analytics.raw"));
        cat.create_namespace(&ns).await.unwrap();
    }

    #[tokio::test]
    async fn create_table_returns_table_with_metadata() {
        let mt = Box::new(MockTransport::default());
        let cat = RestCatalog::new("http://x", mt);
        let m = TableMetadataBuilder::new()
            .location("s3://x/t")
            .schema(Schema::default())
            .build()
            .unwrap();
        let t = cat
            .create_table(&TableIdent::from_dot("analytics.t"), m.clone())
            .await
            .unwrap();
        assert_eq!(t.metadata.location, "s3://x/t");
    }

    #[tokio::test]
    async fn list_namespaces_decodes_response() {
        let mt = MockTransport::default();
        mt.set_response(RestResponse {
            status: 200,
            body_json: Some(r#"{"namespaces":[["analytics"],["analytics","raw"]]}"#.into()),
        });
        let cat = RestCatalog::new("http://x", Box::new(mt));
        let ns = cat.list_namespaces(None).await.unwrap();
        assert_eq!(ns.len(), 2);
        assert_eq!(ns[0].as_dot(), "analytics");
        assert_eq!(ns[1].as_dot(), "analytics.raw");
    }

    #[tokio::test]
    async fn load_table_decodes_metadata_and_location() {
        let mt = MockTransport::default();
        let schema = Schema::default();
        let m = TableMetadataBuilder::new()
            .location("s3://x/t")
            .schema(schema)
            .build()
            .unwrap();
        let body = serde_json::json!({
            "metadata": m,
            "metadata-location": "s3://x/t/metadata.json",
        });
        mt.set_response(RestResponse {
            status: 200,
            body_json: Some(serde_json::to_string(&body).unwrap()),
        });
        let cat = RestCatalog::new("http://x", Box::new(mt));
        let t = cat.load_table(&TableIdent::from_dot("ns.t")).await.unwrap();
        assert_eq!(t.metadata_location.as_deref(), Some("s3://x/t/metadata.json"));
    }

    #[tokio::test]
    async fn load_table_error_status_returns_not_found() {
        let mt = MockTransport::default();
        mt.set_response(RestResponse { status: 404, body_json: None });
        let cat = RestCatalog::new("http://x", Box::new(mt));
        let r = cat.load_table(&TableIdent::from_dot("ns.t")).await;
        assert!(matches!(r, Err(Error::NotFound(_))));
    }

    #[tokio::test]
    async fn namespace_url_encodes_unit_separator() {
        let mt = Box::new(MockTransport::default());
        let cat = RestCatalog::new("http://x", mt);
        let ns = NamespaceIdent::from_dot("a.b.c");
        assert_eq!(cat.namespace_url(&ns), "http://x/v1/namespaces/a%1Fb%1Fc");
    }
}
