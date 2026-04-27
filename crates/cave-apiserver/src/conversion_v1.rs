//! CustomResourceConversion (apiextensions.k8s.io/v1) — types and strategy
//! dispatch for CRD version conversion. Layered atop `conversion.rs`
//! which already has the converter trait + tenant-invariant enforcement.
//!
//! Upstream sources (kubernetes/kubernetes v1.31):
//!   * `staging/src/k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1/types.go`
//!     (`CustomResourceConversion`, `WebhookConversion`,
//!      `ConversionStrategyType`, `ServiceReference`, `WebhookClientConfig`).
//!   * `staging/src/k8s.io/apiextensions-apiserver/pkg/apiserver/conversion/converter.go`
//!     (strategy dispatch, hub/spoke selection).
//!
//! ## Strategy semantics
//!
//! - `None`: no conversion — input version must equal storage version, else
//!   the conversion fails. Mirrors upstream NopConverter.
//! - `Webhook`: dispatch a `ConversionReview` to the configured webhook.
//!
//! ## Tenant invariant
//!
//! Conversion of a tenant-T object MUST yield a tenant-T object. The
//! existing `WebhookConverter` re-checks tenant_id; here we add static
//! validation that a `CustomResourceConversion.webhook` cannot reference a
//! cross-tenant Service.

use crate::conversion::{
    ConversionRequest, ConversionResponse, ConvertibleObject,
    ConversionWebhookClient, WebhookConverter,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ConversionStrategyType {
    None,
    Webhook,
}

impl Default for ConversionStrategyType {
    fn default() -> Self { ConversionStrategyType::None }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceReference {
    pub namespace: String,
    pub name: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub port: Option<i32>,
    /// Tenant scope check — services in another tenant are rejected at
    /// validation time. NOT in upstream; cave-runtime invariant.
    #[serde(default)]
    pub tenant_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebhookClientConfig {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub service: Option<ServiceReference>,
    #[serde(default)]
    pub ca_bundle: Vec<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebhookConversion {
    pub client_config: WebhookClientConfig,
    /// Versions this webhook understands; e.g. `["v1"]`.
    pub conversion_review_versions: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomResourceConversion {
    pub strategy: ConversionStrategyType,
    #[serde(default)]
    pub webhook: Option<WebhookConversion>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Validation — `apiextensions/validation/validation.go::validateCustomResource
// Conversion`. Errors mirror the upstream field-validation strings as
// closely as possible.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConversionValidationError {
    #[error("conversion.webhook is required when strategy=Webhook")]
    WebhookMissing,
    #[error("conversion.webhook must be unset when strategy=None")]
    WebhookForNone,
    #[error("conversionReviewVersions must include at least one of [v1, v1beta1]")]
    NoSupportedReviewVersion,
    #[error("clientConfig.url and clientConfig.service are mutually exclusive")]
    ConflictingClient,
    #[error("clientConfig must specify either url or service")]
    MissingClient,
    #[error("clientConfig.url must be https://")]
    UrlNotHttps,
    #[error("conversion.webhook.clientConfig.service refers to tenant `{0}`, expected `{1}` (cross-tenant invariant)")]
    CrossTenantService(String, String),
}

pub fn validate_conversion(
    cv: &CustomResourceConversion, tenant_id: &str,
) -> Result<(), ConversionValidationError> {
    match cv.strategy {
        ConversionStrategyType::None => {
            if cv.webhook.is_some() {
                return Err(ConversionValidationError::WebhookForNone);
            }
            Ok(())
        }
        ConversionStrategyType::Webhook => {
            let Some(w) = &cv.webhook else {
                return Err(ConversionValidationError::WebhookMissing);
            };
            if !w.conversion_review_versions.iter().any(|v| v == "v1" || v == "v1beta1") {
                return Err(ConversionValidationError::NoSupportedReviewVersion);
            }
            match (&w.client_config.url, &w.client_config.service) {
                (None, None) => Err(ConversionValidationError::MissingClient),
                (Some(_), Some(_)) => Err(ConversionValidationError::ConflictingClient),
                (Some(u), None) => {
                    if !u.starts_with("https://") {
                        return Err(ConversionValidationError::UrlNotHttps);
                    }
                    Ok(())
                }
                (None, Some(s)) => {
                    if !s.tenant_id.is_empty() && s.tenant_id != tenant_id {
                        return Err(ConversionValidationError::CrossTenantService(
                            s.tenant_id.clone(), tenant_id.into(),
                        ));
                    }
                    Ok(())
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CRDVersionSet — track served + storage versions for a CRD. Used by the
// strategy dispatcher to pick a hub for hub-spoke conversion.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct CRDVersion {
    pub name: String,
    pub served: bool,
    pub storage: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CRDVersionSet {
    pub group: String,
    pub kind: String,
    pub versions: Vec<CRDVersion>,
}

impl CRDVersionSet {
    pub fn storage_version(&self) -> Option<&CRDVersion> {
        self.versions.iter().find(|v| v.storage)
    }
    pub fn served_versions(&self) -> Vec<&CRDVersion> {
        self.versions.iter().filter(|v| v.served).collect()
    }
    pub fn version(&self, name: &str) -> Option<&CRDVersion> {
        self.versions.iter().find(|v| v.name == name)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CRDVersionError {
    #[error("at most one version may be storage=true; found {0}")]
    MultipleStorage(usize),
    #[error("at least one version must be storage=true")]
    NoStorage,
    #[error("at least one version must be served=true")]
    NoServed,
}

pub fn validate_version_set(set: &CRDVersionSet) -> Result<(), CRDVersionError> {
    let storage_count = set.versions.iter().filter(|v| v.storage).count();
    if storage_count > 1 { return Err(CRDVersionError::MultipleStorage(storage_count)); }
    if storage_count == 0 { return Err(CRDVersionError::NoStorage); }
    if !set.versions.iter().any(|v| v.served) { return Err(CRDVersionError::NoServed); }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// ConversionDispatcher — picks the right converter based on strategy and
// runs it. NopConverter for `None`, WebhookConverter for `Webhook`.
// ─────────────────────────────────────────────────────────────────────────────

pub struct NopConverter;

impl ConversionWebhookClient for NopConverter {
    fn convert(&self, req: ConversionRequest) -> ConversionResponse {
        // Strategy=None: convert is only valid when desired API version
        // equals every input's API version. Otherwise upstream returns a
        // failure with a kubectl-friendly message.
        let target = req.desired_api_version.clone();
        for o in &req.objects {
            if o.api_version != target {
                return ConversionResponse {
                    uid: req.uid,
                    converted_objects: vec![],
                    result_status: "Failure".into(),
                    result_message: format!(
                        "no conversion configured (strategy=None) but input apiVersion {} != desired {}",
                        o.api_version, target),
                };
            }
        }
        let converted = req.objects.iter().cloned().map(|mut o| {
            o.api_version = target.clone();
            o
        }).collect();
        ConversionResponse {
            uid: req.uid,
            converted_objects: converted,
            result_status: "Success".into(),
            result_message: String::new(),
        }
    }
}

pub fn dispatch_conversion<C: ConversionWebhookClient>(
    cv: &CustomResourceConversion, webhook_name: &str,
    nop: &NopConverter, webhook_client: Option<C>,
    req: ConversionRequest,
) -> ConversionResponse {
    match cv.strategy {
        ConversionStrategyType::None => nop.convert(req),
        ConversionStrategyType::Webhook => {
            let Some(client) = webhook_client else {
                return ConversionResponse {
                    uid: req.uid,
                    converted_objects: vec![],
                    result_status: "Failure".into(),
                    result_message: "strategy=Webhook but no client configured".into(),
                };
            };
            let conv = WebhookConverter::new(webhook_name, client);
            conv.convert(req)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// FakeConversionClient — drives dispatch tests without actually calling out.
// ─────────────────────────────────────────────────────────────────────────────

pub struct FakeConversionClient {
    pub respond: Box<dyn Fn(ConversionRequest) -> ConversionResponse + Send + Sync>,
}

impl ConversionWebhookClient for FakeConversionClient {
    fn convert(&self, req: ConversionRequest) -> ConversionResponse {
        (self.respond)(req)
    }
}

#[allow(dead_code)]
fn unused_obj_fields() -> HashMap<String, serde_json::Value> { HashMap::new() }

#[allow(dead_code)]
fn unused_obj() -> ConvertibleObject {
    ConvertibleObject {
        api_version: "".into(), kind: "".into(),
        name: "".into(), namespace: "".into(), tenant_id: "".into(),
        fields: serde_json::Map::new(),
    }
}

#[cfg(test)]
mod tests;
