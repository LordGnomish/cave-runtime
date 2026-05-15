// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! cavectl `auth` library-side surface. Houses the SAML 2.0
//! parse helpers `cavectl auth saml {parse-metadata,sign-request,
//! verify-response}` invokes. Kept here (not under `native/`) so
//! library callers can drive the parsers from tests without
//! pulling in the full CLI binary.

pub mod saml;
