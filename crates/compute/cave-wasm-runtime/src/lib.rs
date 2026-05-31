//! cave-wasm-runtime — pure-Rust WebAssembly execution engine.
//!
//! A real interpreter-based WebAssembly runtime written in safe Rust, plus the
//! host surfaces around it (WASI preview1 shims, resource limits, a module
//! registry and a cave-sandbox capability bridge). It ports *concepts and
//! behaviour* from the reference implementations rather than vendoring them:
//!
//! * `bytecodealliance/wasmtime` (Apache-2.0) — primary semantics reference
//! * `wasmerio/wasmer`           (MIT)        — secondary reference
//! * `spinframework/spin`        (Apache-2.0) — serverless trigger reference
//!
//! See `parity.manifest.toml` for the honest feature map. The Cranelift JIT
//! backend, the component model, WIT bindgen, the wasmCloud actor model and the
//! Spin trigger framework are tracked honestly as out-of-engine scope, not
//! claimed as implemented.

#![forbid(unsafe_code)]

pub mod error;
pub mod exec;
pub mod parser;
pub mod types;

pub use error::{Result, WasmError};
pub use exec::{Instance, Value};
pub use parser::parse_module;
pub use types::{Export, ExternKind, FuncBody, FuncType, Limits, Module, ValType};

/// Crate version string, surfaced by the CLI / portal.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod smoke_tests {
    use super::*;

    #[test]
    fn version_is_exposed() {
        assert_eq!(VERSION, "0.1.0");
    }
}
