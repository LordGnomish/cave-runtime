// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! End-to-end smoke — XRD → Composition → Claim → Composite render →
//! Function pipeline → XPKG install → cavectl dispatch.

use cave_crossplane::{
    cli::{dispatch, run_cli, InfraAction, InfraSubcommand},
    composition::pipeline::PipelineExecutor,
    composition::step::Step,
    conditions::{propagate_composed_to_xr, propagate_xr_to_claim},
    function::grpc_codec::RunFunctionRequest,
    models::{
        CompositionMode, CreateClaimRequest, CreateCompositionRequest, CreateProviderRequest,
        CreateXrdRequest, ProviderType, TypeRef, XrdScope, XrdVersion,
    },
    xpkg::{
        dependency::DependencyGraph,
        install::install_package,
        pull::{write_fixture_xpkg, pull_offline, PackageKind},
    },
    xr::{bind::bind_claim_to_xr, lifecycle::{plan_deletion, XrEvent, XrPhase}},
    xrd::{defaulting::apply_defaults, schema_validate::validate_spec},
    CrossplaneState,
};
use serde_json::json;
use std::sync::Arc;

#[test]
fn smoke_xrd_composition_claim_e2e() {
    let state = Arc::new(CrossplaneState::default());

    // 1. Create XRD
    let xrd = state
        .xrd_store
        .create(CreateXrdRequest {
            name: "xdb.ex.cave.io".into(),
            group: "ex.cave.io".into(),
            kind: "XDb".into(),
            claim_kind: Some("Db".into()),
            scope: XrdScope::Cluster,
            versions: vec![XrdVersion {
                name: "v1".into(),
                served: true,
                referenceable: true,
                schema: None,
            }],
        })
        .unwrap();
    assert_eq!(xrd.kind, "XDb");

    // 2. Create Composition (pipeline mode)
    let comp = state
        .composition_store
        .create(CreateCompositionRequest {
            name: "db-default".into(),
            composite_type_ref: TypeRef {
                api_version: "ex.cave.io/v1".into(),
                kind: "XDb".into(),
            },
            resources: vec![],
            pipeline: vec![],
            mode: CompositionMode::Pipeline,
            patch_sets: vec![],
        })
        .unwrap();
    assert_eq!(comp.name, "db-default");

    // 3. Install Provider + Function
    state
        .provider_store
        .install(CreateProviderRequest {
            name: "provider-kubernetes".into(),
            package: "xpkg.upbound.io/crossplane-contrib/provider-kubernetes:v0.1".into(),
            provider_type: ProviderType::Community,
        })
        .unwrap();
    state
        .function_store
        .install(
            "function-patch-and-transform",
            "v0.1.0",
            "xpkg.upbound.io/x/function-patch-and-transform:v0.1.0",
        )
        .unwrap();

    // 4. Create Claim
    let xrd_get = state.xrd_store.get_by_claim_kind("Db").unwrap();
    let (_claim, _comp_res) = state
        .claim_store
        .create_claim(
            CreateClaimRequest {
                name: "db1".into(),
                namespace: "default".into(),
                kind: "Db".into(),
                api_version: "ex.cave.io/v1".into(),
                spec: json!({"size": 10}),
            },
            &xrd_get,
            &comp,
            &state.engine,
        )
        .unwrap();
    let listed = state.claim_store.list_claims_for_namespace("default");
    assert_eq!(listed.len(), 1);

    // 5. Pipeline execution
    let exec = PipelineExecutor::new();
    let req = RunFunctionRequest::new("ctx", json!({"resources":[]}), json!({}));
    let result = exec
        .run_sync(
            &[Step::new("compose", "function-patch-and-transform")],
            &state.function_store,
            &req,
        )
        .unwrap();
    assert!(result.ok());

    // 6. cavectl health
    let v = run_cli(&state, &["xrd".into(), "health".into()]).unwrap();
    assert_eq!(v["xrds"], json!(1));
    assert_eq!(v["compositions"], json!(1));

    // 7. cavectl provider catalog
    let cat = dispatch(&state, InfraSubcommand::Provider, InfraAction::Catalog).unwrap();
    assert!(cat["items"].as_array().unwrap().len() >= 3);

    // 8. Condition propagation
    let composed = vec![json!({"status":{"conditions":[{"type":"Ready","status":"True"},{"type":"Synced","status":"True"},{"type":"Healthy","status":"True"}]}})];
    let xr = propagate_composed_to_xr(&json!({}), &composed);
    let claim = propagate_xr_to_claim(&json!({}), &xr);
    assert_eq!(
        claim["status"]["conditions"][0]["type"],
        json!("Ready")
    );
}

#[test]
fn smoke_xrd_defaulting_and_validation() {
    let schema = json!({
        "type":"object",
        "required":["name"],
        "properties":{
            "name":{"type":"string","minLength":1},
            "size":{"type":"integer","default":10,"minimum":1,"maximum":100}
        }
    });
    let mut spec = json!({"name":"db1"});
    apply_defaults(&schema, &mut spec);
    assert_eq!(spec["size"], json!(10));
    validate_spec(&schema, &spec).unwrap();

    let bad = json!({"name":""});
    assert!(validate_spec(&schema, &bad).is_err());
}

#[test]
fn smoke_xpkg_pull_install_and_dependency() {
    let state = Arc::new(CrossplaneState::default());
    let tmp = std::env::temp_dir().join(format!("cave-xpkg-smoke-{}", uuid::Uuid::new_v4()));
    write_fixture_xpkg(
        &tmp,
        "function-noop",
        PackageKind::Function,
        &["kind: Function\nname: function-noop\n"],
    )
    .unwrap();
    let bundle = pull_offline(&tmp).unwrap();
    let plan = install_package(&bundle, &state).unwrap();
    assert!(plan.functions.contains(&"function-noop".to_string()));
    assert!(state.function_store.contains("function-noop"));

    // Dependency DAG
    let mut g = DependencyGraph::new();
    for n in ["function-a", "function-b", "function-c"] {
        g.add_node(n);
    }
    g.add_edge("function-a", "function-b").unwrap();
    g.add_edge("function-b", "function-c").unwrap();
    let order = g.topo_sort().unwrap();
    assert_eq!(
        order.iter().position(|n| n == "function-c").unwrap()
            < order.iter().position(|n| n == "function-a").unwrap(),
        true
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn smoke_xr_lifecycle_fsm() {
    let phase = XrPhase::Pending;
    let p1 = phase.next(XrEvent::ComposeStarted).unwrap();
    assert_eq!(p1, XrPhase::Creating);
    let p2 = p1.next(XrEvent::ComposeReady).unwrap();
    assert_eq!(p2, XrPhase::Ready);
    let p3 = p2.next(XrEvent::DeletionRequested).unwrap();
    assert_eq!(p3, XrPhase::Deleting);

    // Deletion plan
    let plan = plan_deletion(cave_crossplane::models::DeletionPolicy::Delete, 3);
    assert_eq!(plan.composed_to_delete, 3);
}

#[test]
fn smoke_xr_bind() {
    let claim = json!({"metadata":{"namespace":"ns","name":"c1"}});
    let xr = json!({"metadata":{"name":"x1"}});
    let (c, x) = bind_claim_to_xr(&claim, &xr);
    assert_eq!(x["spec"]["claimRef"]["name"], json!("c1"));
    assert_eq!(c["spec"]["resourceRef"]["name"], json!("x1"));
}

#[test]
fn smoke_router_constructs() {
    let state = Arc::new(CrossplaneState::default());
    let _r = cave_crossplane::router(state);
    // Construction without panic is sufficient — HTTP path-level coverage is
    // exercised through cave-portal-api integration tests in the workspace.
}
