// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Built-in runbook templates — operators instantiate these into live runbooks.

use crate::models::{ActionType, OnFailure, RunbookStep, RunbookTemplate, TriggerKind};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Return all predefined runbook templates.
pub fn predefined_templates() -> Vec<RunbookTemplate> {
    vec![
        restart_service(),
        scale_up(),
        failover_db(),
        rollback_deploy(),
        rotate_secrets(),
        ssl_cert_renewal(),
    ]
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn params(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

fn shell(id: &str, name: &str, cmd: &str, timeout: u64, on_fail: OnFailure) -> RunbookStep {
    RunbookStep {
        id: id.to_string(),
        name: name.to_string(),
        action: ActionType::ShellCommand,
        params: params(&[("command", json!(cmd))]),
        timeout_secs: timeout,
        on_failure: on_fail,
        retry_count: None,
    }
}

fn cave(
    id: &str,
    name: &str,
    module: &str,
    action: &str,
    extra: &[(&str, Value)],
    timeout: u64,
    on_fail: OnFailure,
) -> RunbookStep {
    let mut p = vec![("module", json!(module)), ("action", json!(action))];
    p.extend_from_slice(extra);
    RunbookStep {
        id: id.to_string(),
        name: name.to_string(),
        action: ActionType::CaveModuleAction,
        params: params(&p),
        timeout_secs: timeout,
        on_failure: on_fail,
        retry_count: None,
    }
}

fn approval(id: &str, name: &str, message: &str) -> RunbookStep {
    RunbookStep {
        id: id.to_string(),
        name: name.to_string(),
        action: ActionType::HumanApproval,
        params: params(&[("message", json!(message))]),
        timeout_secs: 3600, // 1-hour window
        on_failure: OnFailure::Abort,
        retry_count: None,
    }
}

fn notify(id: &str, name: &str, url: &str, body: &str) -> RunbookStep {
    RunbookStep {
        id: id.to_string(),
        name: name.to_string(),
        action: ActionType::ApiCall,
        params: params(&[
            ("method", json!("POST")),
            ("url", json!(url)),
            ("body", json!(body)),
        ]),
        timeout_secs: 10,
        on_failure: OnFailure::Skip, // notification failure shouldn't block
        retry_count: None,
    }
}

// ── Template definitions ──────────────────────────────────────────────────────

/// restart_service: find pod → drain → restart → verify health.
fn restart_service() -> RunbookTemplate {
    RunbookTemplate {
        id: "restart-service".to_string(),
        name: "Restart Service".to_string(),
        description: "Gracefully drain and restart a Kubernetes pod, then verify health.".to_string(),
        tags: vec!["kubernetes".to_string(), "restart".to_string(), "recovery".to_string()],
        default_trigger: TriggerKind::Incident,
        steps: vec![
            shell(
                "find-pod",
                "Find pod",
                "kubectl get pods -l app=${SERVICE_NAME} -o name | head -1",
                30,
                OnFailure::Abort,
            ),
            shell(
                "drain-pod",
                "Drain connections",
                "kubectl drain ${POD_NAME} --ignore-daemonsets --delete-emptydir-data --grace-period=30",
                120,
                OnFailure::Abort,
            ),
            approval("approve-restart", "Approve restart", "Confirm restart of ${SERVICE_NAME}"),
            shell(
                "restart-pod",
                "Restart pod",
                "kubectl rollout restart deployment/${SERVICE_NAME}",
                60,
                OnFailure::Abort,
            ),
            shell(
                "verify-health",
                "Verify health",
                "kubectl rollout status deployment/${SERVICE_NAME} --timeout=120s",
                150,
                OnFailure::Abort,
            ),
            notify(
                "notify-complete",
                "Notify completion",
                "${SLACK_WEBHOOK_URL}",
                r#"{"text": "✅ Service ${SERVICE_NAME} restarted successfully"}"#,
            ),
        ],
    }
}

/// scale_up: check metrics → increase replicas → verify scaling → notify.
fn scale_up() -> RunbookTemplate {
    RunbookTemplate {
        id: "scale-up".to_string(),
        name: "Scale Up".to_string(),
        description: "Check current load metrics, scale out replicas, and verify the deployment settled.".to_string(),
        tags: vec!["kubernetes".to_string(), "scaling".to_string(), "capacity".to_string()],
        default_trigger: TriggerKind::Alert,
        steps: vec![
            shell(
                "check-metrics",
                "Check current metrics",
                "kubectl top pods -l app=${SERVICE_NAME}",
                30,
                OnFailure::Skip,
            ),
            shell(
                "get-replicas",
                "Get current replica count",
                "kubectl get deployment/${SERVICE_NAME} -o jsonpath='{.spec.replicas}'",
                15,
                OnFailure::Abort,
            ),
            shell(
                "scale-out",
                "Scale to target replicas",
                "kubectl scale deployment/${SERVICE_NAME} --replicas=${TARGET_REPLICAS}",
                30,
                OnFailure::Abort,
            ),
            shell(
                "wait-rollout",
                "Wait for rollout",
                "kubectl rollout status deployment/${SERVICE_NAME} --timeout=180s",
                200,
                OnFailure::Abort,
            ),
            shell(
                "verify-scaling",
                "Verify replica count",
                "kubectl get deployment/${SERVICE_NAME} -o jsonpath='{.status.availableReplicas}'",
                15,
                OnFailure::Abort,
            ),
            notify(
                "notify-scaled",
                "Notify scale-up",
                "${SLACK_WEBHOOK_URL}",
                r#"{"text": "📈 ${SERVICE_NAME} scaled to ${TARGET_REPLICAS} replicas"}"#,
            ),
        ],
    }
}

/// failover_db: check primary → promote replica → update DNS → verify → notify.
fn failover_db() -> RunbookTemplate {
    RunbookTemplate {
        id: "failover-db".to_string(),
        name: "Database Failover".to_string(),
        description:
            "Promote a read replica to primary, update DNS, and verify connectivity.".to_string(),
        tags: vec![
            "database".to_string(),
            "failover".to_string(),
            "high-availability".to_string(),
        ],
        default_trigger: TriggerKind::Incident,
        steps: vec![
            shell(
                "check-primary",
                "Check primary DB health",
                "pg_isready -h ${DB_PRIMARY_HOST} -p 5432",
                15,
                OnFailure::Skip,
            ),
            approval(
                "approve-failover",
                "Approve database failover",
                "Promote ${DB_REPLICA_HOST} to primary? This will cause a brief write outage.",
            ),
            shell(
                "promote-replica",
                "Promote replica to primary",
                "psql -h ${DB_REPLICA_HOST} -c 'SELECT pg_promote();'",
                60,
                OnFailure::Abort,
            ),
            shell(
                "update-dns",
                "Update DB DNS record",
                "aws route53 change-resource-record-sets --hosted-zone-id ${ZONE_ID} \
                 --change-batch '{\"Changes\":[{\"Action\":\"UPSERT\",\"ResourceRecordSet\":\
                 {\"Name\":\"${DB_DNS}\",\"Type\":\"CNAME\",\"TTL\":60,\"ResourceRecords\":\
                 [{\"Value\":\"${DB_REPLICA_HOST}\"}]}}]}'",
                30,
                OnFailure::Abort,
            ),
            shell(
                "verify-connectivity",
                "Verify new primary connectivity",
                "pg_isready -h ${DB_DNS} -p 5432",
                30,
                OnFailure::Abort,
            ),
            notify(
                "notify-failover",
                "Notify failover complete",
                "${SLACK_WEBHOOK_URL}",
                r#"{"text": "🔄 DB failover complete — new primary: ${DB_REPLICA_HOST}"}"#,
            ),
        ],
    }
}

/// rollback_deploy: get last good deploy → cave-deploy rollback → verify → close incident.
fn rollback_deploy() -> RunbookTemplate {
    RunbookTemplate {
        id: "rollback-deploy".to_string(),
        name: "Rollback Deployment".to_string(),
        description:
            "Roll back to the last known-good deployment using cave-deploy, then close the triggering incident."
            .to_string(),
        tags: vec![
            "deployment".to_string(),
            "rollback".to_string(),
            "recovery".to_string(),
        ],
        default_trigger: TriggerKind::Incident,
        steps: vec![
            shell(
                "get-history",
                "Get deployment history",
                "kubectl rollout history deployment/${SERVICE_NAME}",
                15,
                OnFailure::Skip,
            ),
            cave(
                "do-rollback",
                "Roll back via cave-deploy",
                "cave-deploy",
                "rollback",
                &[("target", json!("previous"))],
                120,
                OnFailure::Abort,
            ),
            shell(
                "verify-rollback",
                "Verify rollback healthy",
                "kubectl rollout status deployment/${SERVICE_NAME} --timeout=120s",
                150,
                OnFailure::Abort,
            ),
            cave(
                "close-incident",
                "Close triggering incident",
                "cave-incidents",
                "update",
                &[
                    ("incident_id", json!("${INCIDENT_ID}")),
                    ("status", json!("resolved")),
                    ("resolution", json!("Rolled back to previous deployment")),
                ],
                15,
                OnFailure::Skip,
            ),
            notify(
                "notify-rollback",
                "Notify rollback",
                "${SLACK_WEBHOOK_URL}",
                r#"{"text": "⏪ ${SERVICE_NAME} rolled back — incident closed"}"#,
            ),
        ],
    }
}

/// rotate_secrets: cave-vault rotate → update deployments → verify connectivity.
fn rotate_secrets() -> RunbookTemplate {
    RunbookTemplate {
        id: "rotate-secrets".to_string(),
        name: "Rotate Secrets".to_string(),
        description:
            "Rotate credentials in cave-vault, update affected deployments, and verify connectivity."
            .to_string(),
        tags: vec![
            "security".to_string(),
            "secrets".to_string(),
            "compliance".to_string(),
        ],
        default_trigger: TriggerKind::Schedule,
        steps: vec![
            approval(
                "approve-rotation",
                "Approve secret rotation",
                "Rotate ${SECRET_NAME}? Affected services will restart.",
            ),
            cave(
                "rotate-vault",
                "Rotate secret in cave-vault",
                "cave-vault",
                "rotate",
                &[("secret", json!("${SECRET_NAME}"))],
                60,
                OnFailure::Abort,
            ),
            shell(
                "update-deployments",
                "Trigger rolling restart of affected deployments",
                "kubectl rollout restart deployment -l uses-secret=${SECRET_NAME}",
                60,
                OnFailure::Abort,
            ),
            shell(
                "verify-connectivity",
                "Verify services healthy after rotation",
                "kubectl rollout status deployment -l uses-secret=${SECRET_NAME} --timeout=180s",
                200,
                OnFailure::Abort,
            ),
            notify(
                "notify-rotation",
                "Notify rotation complete",
                "${SLACK_WEBHOOK_URL}",
                r#"{"text": "🔑 Secret ${SECRET_NAME} rotated and deployments updated"}"#,
            ),
        ],
    }
}

/// ssl_cert_renewal: cave-certs renew → deploy → verify TLS.
fn ssl_cert_renewal() -> RunbookTemplate {
    RunbookTemplate {
        id: "ssl-cert-renewal".to_string(),
        name: "SSL Certificate Renewal".to_string(),
        description:
            "Renew TLS certificates via cave-certs, deploy updated secrets, and verify TLS handshake."
            .to_string(),
        tags: vec![
            "tls".to_string(),
            "certificates".to_string(),
            "security".to_string(),
        ],
        default_trigger: TriggerKind::Schedule,
        steps: vec![
            shell(
                "check-expiry",
                "Check certificate expiry",
                "openssl s_client -connect ${DOMAIN}:443 -servername ${DOMAIN} </dev/null 2>/dev/null \
                 | openssl x509 -noout -dates",
                15,
                OnFailure::Skip,
            ),
            cave(
                "renew-cert",
                "Renew certificate via cave-certs",
                "cave-certs",
                "renew",
                &[("domain", json!("${DOMAIN}"))],
                120,
                OnFailure::Abort,
            ),
            shell(
                "deploy-cert",
                "Update Kubernetes TLS secret",
                "kubectl create secret tls ${DOMAIN}-tls \
                 --cert=/tmp/tls.crt --key=/tmp/tls.key \
                 --dry-run=client -o yaml | kubectl apply -f -",
                30,
                OnFailure::Abort,
            ),
            shell(
                "reload-ingress",
                "Reload ingress controller",
                "kubectl rollout restart deployment/ingress-nginx-controller -n ingress-nginx",
                60,
                OnFailure::Abort,
            ),
            shell(
                "verify-tls",
                "Verify TLS handshake",
                "curl -sv https://${DOMAIN} 2>&1 | grep -E '(SSL certificate verify|subject:|issuer:)'",
                20,
                OnFailure::Abort,
            ),
            notify(
                "notify-renewal",
                "Notify certificate renewed",
                "${SLACK_WEBHOOK_URL}",
                r#"{"text": "🔒 TLS certificate for ${DOMAIN} renewed and deployed"}"#,
            ),
        ],
    }
}
