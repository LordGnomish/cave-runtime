//! End-to-end integration tests for the `cavectl` binary.
//!
//! These tests exercise the real binary via Cargo's
//! `CARGO_BIN_EXE_cavectl` env var, against an in-process `mockito`
//! HTTP server. Per Burak's Integration Sprint C mandate the full
//! pipeline is wired: clap parse → lib `prepare()` → reqwest send →
//! response parse → stdout.
//!
//! Each test case:
//!   * starts a `mockito::Server` in this thread
//!   * spawns `cavectl` with `--server <mock-url>` and the verb
//!   * asserts both that the mock saw the expected call AND that
//!     stdout/exit code match the contract.

use std::process::Command;

fn cavectl_path() -> &'static str {
    env!("CARGO_BIN_EXE_cavectl")
}

fn run(args: &[&str]) -> (std::process::Output, String) {
    let out = Command::new(cavectl_path())
        .args(args)
        .output()
        .expect("failed to spawn cavectl");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    (out, stdout)
}

// ── help / version (no HTTP) ────────────────────────────────────────────────

#[test]
fn help_lists_native_and_compat_surfaces() {
    let (out, stdout) = run(&["--help"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    // Native
    assert!(stdout.contains("deploy"), "deploy verb missing in --help");
    assert!(stdout.contains("get"), "get verb missing");
    assert!(stdout.contains("describe"), "describe verb missing");
    assert!(stdout.contains("logs"), "logs verb missing");
    assert!(stdout.contains("events"), "events verb missing");
    assert!(stdout.contains("secrets"), "secrets verb missing");
    assert!(stdout.contains("flag"), "flag verb missing");
    assert!(stdout.contains("chaos"), "chaos verb missing");
    assert!(stdout.contains("topology"), "topology verb missing");
    // All nine compat shims
    for shim in [
        "kubectl", "etcdctl", "cilium", "helm", "argocd", "vault", "harbor", "strimzi", "talosctl",
    ] {
        assert!(stdout.contains(shim), "compat shim `{}` missing in --help", shim);
    }
    // Utility
    assert!(stdout.contains("completion"));
}

#[test]
fn kubectl_subcommand_help() {
    let (out, stdout) = run(&["kubectl", "--help"]);
    assert!(out.status.success());
    for verb in [
        "get",
        "describe",
        "create",
        "apply",
        "delete",
        "exec",
        "logs",
        "port-forward",
        "cp",
    ] {
        assert!(
            stdout.contains(verb),
            "kubectl subcommand `{}` missing in `kubectl --help`",
            verb
        );
    }
}

#[test]
fn version_prints() {
    let (out, stdout) = run(&["--version"]);
    assert!(out.status.success());
    assert!(stdout.contains("cavectl"));
}

#[test]
fn completion_bash_is_real_script() {
    let (out, stdout) = run(&["completion", "bash"]);
    assert!(out.status.success());
    assert!(stdout.contains("complete -F"));
    assert!(stdout.contains("kubectl"));
    assert!(stdout.contains("etcdctl"));
}

// ── kubectl get against mock ────────────────────────────────────────────────

#[tokio::test]
async fn kubectl_get_pods_json_against_mock() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/compat/kubectl/v1/namespaces/default/pods")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"items":[{"metadata":{"name":"nginx"}},{"metadata":{"name":"redis"}}]}"#)
        .create_async()
        .await;

    let (out, stdout) = run(&[
        "--server",
        &server.url(),
        "kubectl",
        "get",
        "pods",
        "-o",
        "json",
    ]);
    assert!(
        out.status.success(),
        "exit code {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    // Output must be a valid JSON object with `items`
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout is not valid JSON");
    let items = parsed["items"].as_array().expect("items[] missing");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["metadata"]["name"], "nginx");
    mock.assert_async().await;
}

#[tokio::test]
async fn kubectl_get_pods_yaml_against_mock() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/compat/kubectl/v1/namespaces/default/pods")
        .with_status(200)
        .with_body(r#"{"items":[{"metadata":{"name":"nginx"}}]}"#)
        .create_async()
        .await;

    let (out, stdout) = run(&[
        "--server",
        &server.url(),
        "-o",
        "yaml",
        "kubectl",
        "get",
        "pods",
    ]);
    assert!(out.status.success());
    assert!(stdout.contains("items:"));
    assert!(stdout.contains("nginx"));
    mock.assert_async().await;
}

#[tokio::test]
async fn kubectl_get_pods_name_format() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/compat/kubectl/v1/namespaces/default/pods")
        .with_status(200)
        .with_body(r#"{"items":[{"metadata":{"name":"a"}},{"metadata":{"name":"b"}}]}"#)
        .create_async()
        .await;

    let (out, stdout) = run(&[
        "--server",
        &server.url(),
        "-o",
        "name",
        "kubectl",
        "get",
        "pods",
    ]);
    assert!(out.status.success());
    let names: Vec<&str> = stdout.lines().collect();
    assert_eq!(names, vec!["a", "b"]);
    mock.assert_async().await;
}

#[tokio::test]
async fn kubectl_get_pods_table_default() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/compat/kubectl/v1/namespaces/default/pods")
        .with_status(200)
        // Table renderer formats arrays of plain objects nicely.
        .with_body(r#"[{"name":"nginx","status":"Running"}]"#)
        .create_async()
        .await;

    let (out, stdout) = run(&[
        "--server",
        &server.url(),
        "kubectl",
        "get",
        "pods",
    ]);
    assert!(out.status.success());
    assert!(stdout.contains("NAME"));
    assert!(stdout.contains("STATUS"));
    assert!(stdout.contains("nginx"));
    mock.assert_async().await;
}

#[tokio::test]
async fn kubectl_get_with_namespace_routes_correctly() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/compat/kubectl/v1/namespaces/kube-system/pods")
        .with_status(200)
        .with_body(r#"{"items":[]}"#)
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "kubectl",
        "get",
        "pods",
        "-n",
        "kube-system",
        "-o",
        "json",
    ]);
    assert!(out.status.success());
    mock.assert_async().await;
}

#[tokio::test]
async fn kubectl_get_all_namespaces_routes_correctly() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/compat/kubectl/v1/pods")
        .with_status(200)
        .with_body(r#"{"items":[]}"#)
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "kubectl",
        "get",
        "pods",
        "-A",
    ]);
    assert!(out.status.success());
    mock.assert_async().await;
}

// ── kubectl delete (DELETE method) ──────────────────────────────────────────

#[tokio::test]
async fn kubectl_delete_pod_routes_correctly() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("DELETE", "/api/compat/kubectl/v1/namespaces/default/pods/nginx")
        .with_status(200)
        .with_body("")
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "kubectl",
        "delete",
        "pods",
        "nginx",
    ]);
    assert!(out.status.success());
    mock.assert_async().await;
}

// ── kubectl create against mock (POST + body) ──────────────────────────────

#[tokio::test]
async fn kubectl_create_from_yaml_file() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/api/compat/kubectl/v1/namespaces/default/pods")
        .match_body(mockito::Matcher::PartialJsonString(
            r#"{"kind":"Pod"}"#.to_string(),
        ))
        .with_status(201)
        .with_body(r#"{"metadata":{"name":"nginx"}}"#)
        .create_async()
        .await;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pod.yaml");
    std::fs::write(&path, "kind: Pod\nmetadata:\n  name: nginx\n").unwrap();

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "kubectl",
        "create",
        "-f",
        path.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    mock.assert_async().await;
}

// ── auth header ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn token_is_sent_as_bearer() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/compat/kubectl/v1/namespaces/default/pods")
        .match_header("authorization", "Bearer abc123")
        .with_status(200)
        .with_body(r#"{"items":[]}"#)
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "--token",
        "abc123",
        "kubectl",
        "get",
        "pods",
        "-o",
        "json",
    ]);
    assert!(out.status.success());
    mock.assert_async().await;
}

// ── error mapping ───────────────────────────────────────────────────────────

#[tokio::test]
async fn server_404_exits_non_zero() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/api/compat/kubectl/v1/namespaces/default/pods")
        .with_status(404)
        .with_body("not found")
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "kubectl",
        "get",
        "pods",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("404"));
}

#[tokio::test]
async fn server_401_hints_auth() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/api/compat/kubectl/v1/namespaces/default/pods")
        .with_status(401)
        .with_body("auth required")
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "kubectl",
        "get",
        "pods",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("401"));
    assert!(stderr.contains("CAVE_TOKEN"));
}

// ── etcdctl ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn etcdctl_get_routes_to_kv_range() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/api/compat/etcd/v3/kv/range")
        .match_body(mockito::Matcher::PartialJsonString(
            r#"{"key":"/foo"}"#.to_string(),
        ))
        .with_status(200)
        .with_body(r#"{"kvs":[{"key":"/foo","value":"bar"}]}"#)
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "etcdctl",
        "get",
        "/foo",
        "-o",
        "json",
    ]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    mock.assert_async().await;
}

#[tokio::test]
async fn etcdctl_put_sends_value() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/api/compat/etcd/v3/kv/put")
        .match_body(mockito::Matcher::PartialJsonString(
            r#"{"key":"/x","value":"y"}"#.to_string(),
        ))
        .with_status(200)
        .with_body("{}")
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "etcdctl",
        "put",
        "/x",
        "y",
    ]);
    assert!(out.status.success());
    mock.assert_async().await;
}

// ── helm ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn helm_install_routes_correctly() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock(
            "POST",
            "/api/compat/helm/v3/namespaces/default/releases/install",
        )
        .with_status(200)
        .with_body(r#"{"name":"redis","status":"deployed"}"#)
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "helm",
        "install",
        "redis",
        "bitnami/redis",
        "-o",
        "json",
    ]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    mock.assert_async().await;
}

#[tokio::test]
async fn helm_list_routes_correctly() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/compat/helm/v3/namespaces/default/releases")
        .with_status(200)
        .with_body(r#"[{"name":"redis"}]"#)
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "helm",
        "list",
        "-o",
        "json",
    ]);
    assert!(out.status.success());
    mock.assert_async().await;
}

// ── cilium ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cilium_status_routes_correctly() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/compat/cilium/v1/status")
        .with_status(200)
        .with_body(r#"{"agent":"ok"}"#)
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "cilium",
        "status",
    ]);
    assert!(out.status.success());
    mock.assert_async().await;
}

// ── vault ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn vault_kv_get_routes_to_data_path() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/compat/vault/v1/secret/data/app/db")
        .with_status(200)
        .with_body(r#"{"data":{"data":{"user":"admin"}}}"#)
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "vault",
        "kv",
        "get",
        "secret/app/db",
        "-o",
        "json",
    ]);
    assert!(out.status.success());
    mock.assert_async().await;
}

// ── harbor ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn harbor_project_list_routes_correctly() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/compat/harbor/v2/projects")
        .with_status(200)
        .with_body(r#"[{"project_name":"library"}]"#)
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "harbor",
        "project",
        "list",
        "-o",
        "json",
    ]);
    assert!(out.status.success());
    mock.assert_async().await;
}

// ── strimzi ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn strimzi_topics_list_routes_correctly() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/compat/strimzi/v1/topics")
        .with_status(200)
        .with_body(r#"[{"name":"orders.v1"}]"#)
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "strimzi",
        "topics",
        "list",
        "-o",
        "json",
    ]);
    assert!(out.status.success());
    mock.assert_async().await;
}

// ── talosctl ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn talosctl_version_routes_correctly() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/compat/talos/v1/version")
        .with_status(200)
        .with_body(r#"{"talos":"v1.7.0","cavectl":"0.1.0"}"#)
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "talosctl",
        "version",
    ]);
    assert!(out.status.success());
    mock.assert_async().await;
}

#[tokio::test]
async fn talosctl_service_restart_uses_post() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/api/compat/talos/v1/nodes/n1/services/etcd/restart")
        .with_status(200)
        .with_body("{}")
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "talosctl",
        "service",
        "restart",
        "--node",
        "n1",
        "etcd",
    ]);
    assert!(out.status.success());
    mock.assert_async().await;
}

// ── argocd ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn argocd_app_sync_uses_post() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/api/compat/argocd/v1/applications/guestbook/sync")
        .with_status(200)
        .with_body(r#"{"status":"Synced"}"#)
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "argocd",
        "app",
        "sync",
        "guestbook",
    ]);
    assert!(out.status.success());
    mock.assert_async().await;
}

// ── native verbs ────────────────────────────────────────────────────────────

#[tokio::test]
async fn native_get_pods_routes_correctly() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/native/pods")
        .with_status(200)
        .with_body(r#"[{"name":"nginx"}]"#)
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "get",
        "pods",
        "-o",
        "json",
    ]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    mock.assert_async().await;
}

#[tokio::test]
async fn native_get_with_tenant_scopes_path() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/native/tenants/acme/pods")
        .with_status(200)
        .with_body(r#"[]"#)
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "get",
        "pods",
        "-t",
        "acme",
    ]);
    assert!(out.status.success());
    mock.assert_async().await;
}

#[tokio::test]
async fn native_deploy_module() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/api/native/deploy")
        .match_body(mockito::Matcher::PartialJsonString(
            r#"{"module":"cave-apiserver"}"#.to_string(),
        ))
        .with_status(200)
        .with_body(r#"{"deployment":"started"}"#)
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "deploy",
        "cave-apiserver",
    ]);
    assert!(out.status.success());
    mock.assert_async().await;
}

#[tokio::test]
async fn native_chaos_run_with_kind() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/api/native/chaos")
        .match_body(mockito::Matcher::PartialJsonString(
            r#"{"kind":"pod","target":"app=api"}"#.to_string(),
        ))
        .with_status(200)
        .with_body(r#"{"id":"exp-1"}"#)
        .create_async()
        .await;

    let (out, _) = run(&[
        "--server",
        &server.url(),
        "chaos",
        "run",
        "--kind",
        "pod",
        "--target",
        "app=api",
        "--duration",
        "30s",
    ]);
    assert!(out.status.success());
    mock.assert_async().await;
}

// ── invalid input ──────────────────────────────────────────────────────────

#[test]
fn unknown_subcommand_fails_clap_parse() {
    let (out, _) = run(&["wibble"]);
    assert!(!out.status.success());
}

#[test]
fn missing_required_arg_fails() {
    // `cavectl deploy` requires a module argument.
    let (out, _) = run(&["deploy"]);
    assert!(!out.status.success());
}
