//! Command dispatch for the `cave-wasm` binary, factored into the library so it
//! is unit-testable. `main.rs` is a thin wrapper that prints the result and
//! sets the exit code.
#![allow(unused_imports, dead_code)]

use crate::digest::sha256_hex;
use crate::exec::{Instance, Value};
use crate::limits::ResourceLimits;
use crate::parser::parse_module;
use crate::wasi::WasiCtx;
use crate::VERSION;

const USAGE: &str = "\
cave-wasm — pure-Rust WebAssembly runtime

USAGE:
  cave-wasm version
  cave-wasm info
  cave-wasm digest <file>
  cave-wasm run <file.wasm> --invoke <name> [i32 args...]
  cave-wasm wasi <file.wasm> [--fuel <n>]
";

/// Dispatch a parsed argument vector (excluding argv[0]).
pub fn dispatch(args: &[String]) -> std::result::Result<String, String> {
    // RED stub — real dispatch implemented in the GREEN commit.
    let _ = args;
    Err("unimplemented".to_string())
}

/// Read a wasm/file argument from disk.
#[allow(dead_code)]
fn read_file(path: &str) -> std::result::Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))
}

#[allow(dead_code)]
fn fmt_values(vs: &[Value]) -> String {
    vs.iter()
        .map(|v| match v {
            Value::I32(x) => format!("i32:{x}"),
            Value::I64(x) => format!("i64:{x}"),
            Value::F32(x) => format!("f32:{x}"),
            Value::F64(x) => format!("f64:{x}"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_subcommand() {
        assert_eq!(dispatch(&["version".to_string()]).unwrap(), VERSION);
    }

    #[test]
    fn info_subcommand() {
        let out = dispatch(&["info".to_string()]).unwrap();
        assert!(out.contains("WebAssembly"));
        assert!(out.contains(VERSION));
    }

    #[test]
    fn unknown_returns_usage() {
        let err = dispatch(&["frobnicate".to_string()]).unwrap_err();
        assert!(err.contains("USAGE"));
    }
}
