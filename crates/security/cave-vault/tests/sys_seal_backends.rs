// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/v1/sys/seal-backends` surfaces the supported seal barriers, including the
//! post-quantum ML-KEM-768 seal-wrap, for the portal / cavectl capability view.

use axum::extract::State;
use cave_vault::VaultState;
use cave_vault::api::sys::seal_backends;

#[tokio::test]
async fn seal_backends_advertises_pqc_mlkem768() {
    let state = VaultState::new();
    let resp = seal_backends(State(state)).await.unwrap();
    let data = resp.data.expect("data");

    assert_eq!(data["active"], "uninitialized");
    let backends = data["backends"].as_array().expect("backends array");

    // The classic Shamir + KMS backends are present and NOT quantum resistant.
    let shamir = backends.iter().find(|b| b["type"] == "shamir").unwrap();
    assert_eq!(shamir["quantum_resistant"], false);

    // The PQC barrier is present, quantum-resistant, with FIPS-203 parameters.
    let pqc = backends
        .iter()
        .find(|b| b["type"] == "mlkem768")
        .expect("mlkem768 backend");
    assert_eq!(pqc["quantum_resistant"], true);
    assert_eq!(pqc["recovery_key"], true);
    assert_eq!(pqc["kem"], "ML-KEM-768");
    assert_eq!(pqc["standard"], "NIST FIPS 203");
    assert_eq!(pqc["nist_category"], 3);
    assert_eq!(pqc["encapsulation_key_bytes"], 1184);
    assert_eq!(pqc["ciphertext_bytes"], 1088);
}
