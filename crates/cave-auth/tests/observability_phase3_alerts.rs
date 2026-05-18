// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// FINALIZE smoke: the Prometheus rule file at
// observability/alerts/cave-auth.yml must declare a `cave-auth.phase3`
// rule group that covers every Phase 3 protocol surface. The cave-auth
// crate owns this assertion because it is the SLO contract — every
// metric named here is instrumented in cave-auth/src/<proto>/.

use std::path::PathBuf;

fn alerts_yaml() -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let path: PathBuf = [manifest_dir, "..", "..", "observability", "alerts", "cave-auth.yml"]
        .iter()
        .collect();
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("cannot read {} — {e}", path.display());
    })
}

#[test]
fn phase3_rule_group_exists() {
    let yml = alerts_yaml();
    assert!(
        yml.contains("- name: cave-auth.phase3"),
        "Phase 3 rule group missing from observability/alerts/cave-auth.yml"
    );
}

#[test]
fn ldap_federation_sync_alert_exists() {
    assert!(alerts_yaml().contains("- alert: LdapFederationSyncFailures"));
}

#[test]
fn kerberos_gssapi_accept_failure_alert_exists() {
    assert!(alerts_yaml().contains("- alert: KerberosGssapiAcceptFailures"));
}

#[test]
fn oauth_device_grant_anomaly_alert_exists() {
    assert!(alerts_yaml().contains("- alert: OAuthDeviceGrantAnomaly"));
}

#[test]
fn uma_rpt_issuance_failure_alert_exists() {
    assert!(alerts_yaml().contains("- alert: UmaRptIssuanceFailures"));
}

#[test]
fn oid4vc_issuance_failure_alert_exists() {
    assert!(alerts_yaml().contains("- alert: Oid4vcIssuanceFailures"));
}

#[test]
fn wsfed_signin_failure_alert_exists() {
    assert!(alerts_yaml().contains("- alert: WsFedSigninFailures"));
}

#[test]
fn email_outbox_stalled_alert_exists() {
    assert!(alerts_yaml().contains("- alert: EmailOutboxStalled"));
}

#[test]
fn persistence_migration_failure_alert_exists() {
    assert!(alerts_yaml().contains("- alert: PersistenceMigrationFailure"));
}
