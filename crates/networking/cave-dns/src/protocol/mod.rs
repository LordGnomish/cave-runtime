// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
pub mod dnssec;
pub mod edns;
pub mod message;
pub mod records;

pub use edns::EdnsOptions;
pub use message::{encode_message, make_error_response, make_response, parse_message};
