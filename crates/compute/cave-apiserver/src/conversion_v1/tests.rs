// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! conversion_v1 tests — strategy validation, hub/spoke versioning, dispatch.

use super::*;
use crate::conversion::{ConversionRequest, ConversionResponse, ConvertibleObject};

fn obj(api_version: &str, kind: &str, tenant: &str) -> ConvertibleObject {
    let mut fields = serde_json::Map::new();
    fields.insert("foo".into(), serde_json::Value::String("bar".into()));
    ConvertibleObject {
        api_version: api_version.into(),
        kind: kind.into(),
        name: "obj1".into(),
        namespace: "default".into(),
        tenant_id: tenant.into(),
        fields,
    }
}

fn req(desired_api_version: &str, objects: Vec<ConvertibleObject>) -> ConversionRequest {
    ConversionRequest {
        uid: "uid".into(),
        desired_api_version: desired_api_version.into(),
        objects,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// validate_conversion — `validation_test.go::TestValidateCustomResourceConversion`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn validate_strategy_none_no_webhook_ok() {
    let cv = CustomResourceConversion {
        strategy: ConversionStrategyType::None,
        webhook: None,
    };
    assert_eq!(validate_conversion(&cv, "acme"), Ok(()));
}

#[test]
fn validate_strategy_none_with_webhook_errors() {
    let cv = CustomResourceConversion {
        strategy: ConversionStrategyType::None,
        webhook: Some(WebhookConversion::default()),
    };
    assert_eq!(
        validate_conversion(&cv, "acme"),
        Err(ConversionValidationError::WebhookForNone)
    );
}

#[test]
fn validate_strategy_webhook_missing_webhook_errors() {
    let cv = CustomResourceConversion {
        strategy: ConversionStrategyType::Webhook,
        webhook: None,
    };
    assert_eq!(
        validate_conversion(&cv, "acme"),
        Err(ConversionValidationError::WebhookMissing)
    );
}

#[test]
fn validate_strategy_webhook_review_versions_required() {
    let cv = CustomResourceConversion {
        strategy: ConversionStrategyType::Webhook,
        webhook: Some(WebhookConversion {
            client_config: WebhookClientConfig {
                url: Some("https://x".into()),
                service: None,
                ca_bundle: vec![],
            },
            conversion_review_versions: vec!["v3".into()],
        }),
    };
    assert_eq!(
        validate_conversion(&cv, "acme"),
        Err(ConversionValidationError::NoSupportedReviewVersion)
    );
}

#[test]
fn validate_strategy_webhook_v1_review_ok() {
    let cv = CustomResourceConversion {
        strategy: ConversionStrategyType::Webhook,
        webhook: Some(WebhookConversion {
            client_config: WebhookClientConfig {
                url: Some("https://x".into()),
                service: None,
                ca_bundle: vec![],
            },
            conversion_review_versions: vec!["v1".into()],
        }),
    };
    assert_eq!(validate_conversion(&cv, "acme"), Ok(()));
}

#[test]
fn validate_strategy_webhook_url_xor_service_required() {
    let cv = CustomResourceConversion {
        strategy: ConversionStrategyType::Webhook,
        webhook: Some(WebhookConversion {
            client_config: WebhookClientConfig::default(),
            conversion_review_versions: vec!["v1".into()],
        }),
    };
    assert_eq!(
        validate_conversion(&cv, "acme"),
        Err(ConversionValidationError::MissingClient)
    );
}

#[test]
fn validate_strategy_webhook_url_and_service_conflict() {
    let cv = CustomResourceConversion {
        strategy: ConversionStrategyType::Webhook,
        webhook: Some(WebhookConversion {
            client_config: WebhookClientConfig {
                url: Some("https://x".into()),
                service: Some(ServiceReference {
                    namespace: "ns".into(),
                    name: "svc".into(),
                    path: None,
                    port: None,
                    tenant_id: "acme".into(),
                }),
                ca_bundle: vec![],
            },
            conversion_review_versions: vec!["v1".into()],
        }),
    };
    assert_eq!(
        validate_conversion(&cv, "acme"),
        Err(ConversionValidationError::ConflictingClient)
    );
}

#[test]
fn validate_strategy_webhook_url_must_be_https() {
    let cv = CustomResourceConversion {
        strategy: ConversionStrategyType::Webhook,
        webhook: Some(WebhookConversion {
            client_config: WebhookClientConfig {
                url: Some("http://x".into()),
                service: None,
                ca_bundle: vec![],
            },
            conversion_review_versions: vec!["v1".into()],
        }),
    };
    assert_eq!(
        validate_conversion(&cv, "acme"),
        Err(ConversionValidationError::UrlNotHttps)
    );
}

#[test]
fn validate_cross_tenant_service_rejected() {
    let cv = CustomResourceConversion {
        strategy: ConversionStrategyType::Webhook,
        webhook: Some(WebhookConversion {
            client_config: WebhookClientConfig {
                url: None,
                service: Some(ServiceReference {
                    namespace: "ns".into(),
                    name: "svc".into(),
                    path: None,
                    port: None,
                    tenant_id: "globex".into(),
                }),
                ca_bundle: vec![],
            },
            conversion_review_versions: vec!["v1".into()],
        }),
    };
    let r = validate_conversion(&cv, "acme");
    assert!(
        matches!(r, Err(ConversionValidationError::CrossTenantService(_, _))),
        "globex service for acme conversion must be rejected"
    );
}

#[test]
fn validate_same_tenant_service_ok() {
    let cv = CustomResourceConversion {
        strategy: ConversionStrategyType::Webhook,
        webhook: Some(WebhookConversion {
            client_config: WebhookClientConfig {
                url: None,
                service: Some(ServiceReference {
                    namespace: "ns".into(),
                    name: "svc".into(),
                    path: None,
                    port: None,
                    tenant_id: "acme".into(),
                }),
                ca_bundle: vec![],
            },
            conversion_review_versions: vec!["v1".into()],
        }),
    };
    assert_eq!(validate_conversion(&cv, "acme"), Ok(()));
}

#[test]
fn validate_empty_tenant_service_ok_legacy() {
    // Legacy CRDs without tenant_id annotation get a free pass; the access
    // layer is responsible for tenant scoping in that path.
    let cv = CustomResourceConversion {
        strategy: ConversionStrategyType::Webhook,
        webhook: Some(WebhookConversion {
            client_config: WebhookClientConfig {
                url: None,
                service: Some(ServiceReference {
                    namespace: "ns".into(),
                    name: "svc".into(),
                    path: None,
                    port: None,
                    tenant_id: "".into(),
                }),
                ca_bundle: vec![],
            },
            conversion_review_versions: vec!["v1".into()],
        }),
    };
    assert_eq!(validate_conversion(&cv, "acme"), Ok(()));
}

// ─────────────────────────────────────────────────────────────────────────────
// CRDVersionSet — `validation_test.go::TestValidateCustomResourceDefinitionVersions`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn version_set_storage_version_returns_unique() {
    let s = CRDVersionSet {
        group: "x".into(),
        kind: "Y".into(),
        versions: vec![
            CRDVersion {
                name: "v1alpha1".into(),
                served: true,
                storage: false,
            },
            CRDVersion {
                name: "v1".into(),
                served: true,
                storage: true,
            },
        ],
    };
    assert_eq!(s.storage_version().unwrap().name, "v1");
}

#[test]
fn version_set_served_versions() {
    let s = CRDVersionSet {
        group: "x".into(),
        kind: "Y".into(),
        versions: vec![
            CRDVersion {
                name: "v1alpha1".into(),
                served: false,
                storage: false,
            },
            CRDVersion {
                name: "v1beta1".into(),
                served: true,
                storage: false,
            },
            CRDVersion {
                name: "v1".into(),
                served: true,
                storage: true,
            },
        ],
    };
    assert_eq!(s.served_versions().len(), 2);
}

#[test]
fn validate_version_set_requires_one_storage() {
    let s = CRDVersionSet {
        group: "x".into(),
        kind: "Y".into(),
        versions: vec![CRDVersion {
            name: "v1alpha1".into(),
            served: true,
            storage: false,
        }],
    };
    assert_eq!(validate_version_set(&s), Err(CRDVersionError::NoStorage));
}

#[test]
fn validate_version_set_rejects_two_storage() {
    let s = CRDVersionSet {
        group: "x".into(),
        kind: "Y".into(),
        versions: vec![
            CRDVersion {
                name: "v1alpha1".into(),
                served: true,
                storage: true,
            },
            CRDVersion {
                name: "v1".into(),
                served: true,
                storage: true,
            },
        ],
    };
    assert_eq!(
        validate_version_set(&s),
        Err(CRDVersionError::MultipleStorage(2))
    );
}

#[test]
fn validate_version_set_requires_one_served() {
    let s = CRDVersionSet {
        group: "x".into(),
        kind: "Y".into(),
        versions: vec![CRDVersion {
            name: "v1".into(),
            served: false,
            storage: true,
        }],
    };
    assert_eq!(validate_version_set(&s), Err(CRDVersionError::NoServed));
}

#[test]
fn validate_version_set_minimal_ok() {
    let s = CRDVersionSet {
        group: "x".into(),
        kind: "Y".into(),
        versions: vec![CRDVersion {
            name: "v1".into(),
            served: true,
            storage: true,
        }],
    };
    assert_eq!(validate_version_set(&s), Ok(()));
}

// ─────────────────────────────────────────────────────────────────────────────
// NopConverter — strategy=None semantics
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn nop_converter_succeeds_when_versions_match() {
    let nop = NopConverter;
    let r = nop.convert(req(
        "widgets.acme.io/v1",
        vec![obj("widgets.acme.io/v1", "Widget", "acme")],
    ));
    assert_eq!(r.result_status, "Success");
    assert_eq!(r.converted_objects.len(), 1);
}

#[test]
fn nop_converter_fails_when_input_version_differs() {
    let nop = NopConverter;
    let r = nop.convert(req(
        "widgets.acme.io/v2",
        vec![obj("widgets.acme.io/v1", "Widget", "acme")],
    ));
    assert_eq!(r.result_status, "Failure");
    assert!(r.result_message.contains("strategy=None"));
}

#[test]
fn nop_converter_handles_empty_input() {
    let nop = NopConverter;
    let r = nop.convert(req("v1", vec![]));
    assert_eq!(r.result_status, "Success");
    assert!(r.converted_objects.is_empty());
}

// ─────────────────────────────────────────────────────────────────────────────
// dispatch_conversion — strategy selection
// ─────────────────────────────────────────────────────────────────────────────

fn fake_success(target: &str) -> FakeConversionClient {
    let target = target.to_string();
    FakeConversionClient {
        respond: Box::new(move |req| {
            let converted = req
                .objects
                .iter()
                .cloned()
                .map(|mut o| {
                    o.api_version = target.clone();
                    o
                })
                .collect();
            ConversionResponse {
                uid: req.uid,
                converted_objects: converted,
                result_status: "Success".into(),
                result_message: String::new(),
            }
        }),
    }
}

#[test]
fn dispatch_strategy_none_uses_nop() {
    let cv = CustomResourceConversion {
        strategy: ConversionStrategyType::None,
        webhook: None,
    };
    let nop = NopConverter;
    let webhook: Option<FakeConversionClient> = None;
    let r = dispatch_conversion(
        &cv,
        "wh",
        &nop,
        webhook,
        req("v1", vec![obj("v1", "X", "acme")]),
    );
    assert_eq!(r.result_status, "Success");
}

#[test]
fn dispatch_strategy_webhook_runs_webhook() {
    let cv = CustomResourceConversion {
        strategy: ConversionStrategyType::Webhook,
        webhook: Some(WebhookConversion::default()),
    };
    let nop = NopConverter;
    let webhook = Some(fake_success("v2"));
    let r = dispatch_conversion(
        &cv,
        "wh",
        &nop,
        webhook,
        req("v2", vec![obj("v1", "X", "acme")]),
    );
    assert_eq!(r.result_status, "Success");
    assert_eq!(r.converted_objects[0].api_version, "v2");
}

#[test]
fn dispatch_strategy_webhook_without_client_fails() {
    let cv = CustomResourceConversion {
        strategy: ConversionStrategyType::Webhook,
        webhook: Some(WebhookConversion::default()),
    };
    let nop = NopConverter;
    let webhook: Option<FakeConversionClient> = None;
    let r = dispatch_conversion(
        &cv,
        "wh",
        &nop,
        webhook,
        req("v2", vec![obj("v1", "X", "acme")]),
    );
    assert_eq!(r.result_status, "Failure");
    assert!(r.result_message.contains("no client configured"));
}

#[test]
fn dispatch_webhook_preserves_tenant_id_via_underlying_converter() {
    // The wrapped WebhookConverter re-checks tenant_id; here we sanity-check
    // that the dispatch path doesn't strip it.
    let cv = CustomResourceConversion {
        strategy: ConversionStrategyType::Webhook,
        webhook: Some(WebhookConversion::default()),
    };
    let nop = NopConverter;
    let webhook = Some(fake_success("v2"));
    let r = dispatch_conversion(
        &cv,
        "wh",
        &nop,
        webhook,
        req("v2", vec![obj("v1", "X", "acme")]),
    );
    assert_eq!(r.converted_objects[0].tenant_id, "acme");
}

#[test]
fn dispatch_webhook_rejects_tenant_id_mutation() {
    let cv = CustomResourceConversion {
        strategy: ConversionStrategyType::Webhook,
        webhook: Some(WebhookConversion::default()),
    };
    let nop = NopConverter;
    let evil = FakeConversionClient {
        respond: Box::new(|req| {
            let converted = req
                .objects
                .iter()
                .cloned()
                .map(|mut o| {
                    o.api_version = "v2".into();
                    o.tenant_id = "globex".into(); // rogue
                    o
                })
                .collect();
            ConversionResponse {
                uid: req.uid,
                converted_objects: converted,
                result_status: "Success".into(),
                result_message: String::new(),
            }
        }),
    };
    let r = dispatch_conversion(
        &cv,
        "wh",
        &nop,
        Some(evil),
        req("v2", vec![obj("v1", "X", "acme")]),
    );
    assert_eq!(
        r.result_status, "Failure",
        "tenant_id mutation must flip the outcome to Failure"
    );
    assert!(r.result_message.contains("tenant_id"));
}

// ─────────────────────────────────────────────────────────────────────────────
// Type round-trip
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn strategy_default_is_none() {
    assert_eq!(
        ConversionStrategyType::default(),
        ConversionStrategyType::None
    );
}

#[test]
fn conversion_struct_roundtrip_none() {
    let cv = CustomResourceConversion {
        strategy: ConversionStrategyType::None,
        webhook: None,
    };
    let s = serde_json::to_string(&cv).unwrap();
    let cv2: CustomResourceConversion = serde_json::from_str(&s).unwrap();
    assert_eq!(cv2.strategy, ConversionStrategyType::None);
}

#[test]
fn conversion_struct_roundtrip_webhook() {
    let cv = CustomResourceConversion {
        strategy: ConversionStrategyType::Webhook,
        webhook: Some(WebhookConversion {
            client_config: WebhookClientConfig {
                url: Some("https://x".into()),
                service: None,
                ca_bundle: vec![],
            },
            conversion_review_versions: vec!["v1".into()],
        }),
    };
    let s = serde_json::to_string(&cv).unwrap();
    let cv2: CustomResourceConversion = serde_json::from_str(&s).unwrap();
    assert_eq!(cv2.strategy, ConversionStrategyType::Webhook);
    assert!(cv2.webhook.is_some());
}

// ─────────────────────────────────────────────────────────────────────────────
// `#[ignore]` — gated on real HTTP layer + ConversionReview body parser
// ─────────────────────────────────────────────────────────────────────────────

#[test]
#[cfg(feature = "live-integration")]
fn webhook_real_https_round_trip() {
    // pending: requires real TLS dial against fixture conversion server
}

#[test]
#[cfg(feature = "live-integration")]
fn conversion_review_v1_body_parser() {
    // pending: requires apiextensions.k8s.io/v1.ConversionReview body parsing
}

#[test]
#[cfg(feature = "live-integration")]
fn conversion_review_v1beta1_compat() {
    // pending: requires v1beta1 conversion review shim
}

#[test]
#[cfg(feature = "live-integration")]
fn hub_spoke_chains_three_versions() {
    // pending: requires explicit hub/spoke routing — convert v1alpha1 → v1beta1 → v1 in two steps
}
