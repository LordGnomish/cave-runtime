// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Self-improvement layer — ADR-SELF-IMPROVE-001.
//!
//! Charter ADR-CHARTER-001 promises a "self-improving" runtime. This layer
//! makes that truthful at the runtime-agent altitude: it reads Cave's own
//! observability streams ([`observe`]), turns detected anomalies into
//! *suggested* operational tunings that are never applied without explicit
//! opt-in ([`tune`]), and watches upstream releases to propose hot-patch
//! ports as new versions land ([`upstream`]).
//!
//! Safety rails (ADR-SELF-IMPROVE-001 §3–4): the tuning engine produces
//! proposals over a constrained change surface and gates application behind
//! an explicit approval flag — there is no autonomous live mutation.
//!
//! | responsibility (ADR-SELF-IMPROVE-001)        | module          |
//! |----------------------------------------------|-----------------|
//! | runtime observability data analysis          | [`observe`]     |
//! | LLM-driven self-tuning suggestions + opt-in   | [`tune`]        |
//! | upstream changelog watch + hot-patch ingest   | [`upstream`]    |

pub mod observe;
