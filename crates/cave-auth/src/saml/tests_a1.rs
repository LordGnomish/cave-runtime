// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! Wrapper module that hosts the mission-required
//! `saml/tests/upstream_port.rs` file as a test sub-module.
//! Compiled only under `cfg(test)`.

#[path = "tests/upstream_port.rs"]
mod upstream_port;
