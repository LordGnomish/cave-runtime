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
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    match cmd {
        "version" => Ok(VERSION.to_string()),
        "info" => Ok(info_text()),
        "digest" => {
            let path = args.get(1).ok_or_else(|| USAGE.to_string())?;
            let bytes = read_file(path)?;
            Ok(format!("sha256:{}", sha256_hex(&bytes)))
        }
        "run" => run_cmd(&args[1..]),
        "wasi" => wasi_cmd(&args[1..]),
        "help" | "--help" | "-h" => Ok(USAGE.to_string()),
        _ => Err(USAGE.to_string()),
    }
}

fn info_text() -> String {
    format!(
        "cave-wasm-runtime {VERSION}\n\
         pure-Rust WebAssembly runtime (interpreter)\n\
         features: parser, interpreter, linear-memory, fuel, imports,\n\
         WASI preview1 (subset), content-addressed registry, capability sandbox\n\
         upstreams (concept port): wasmtime (Apache-2.0), wasmer (MIT), spin (Apache-2.0)"
    )
}

fn run_cmd(args: &[String]) -> std::result::Result<String, String> {
    let path = args.first().ok_or_else(|| USAGE.to_string())?;
    let module = parse_module(&read_file(path)?).map_err(|e| e.to_string())?;
    let instance = Instance::new(module);

    let mut name = "main".to_string();
    let mut call_args: Vec<Value> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--invoke" => {
                name = args
                    .get(i + 1)
                    .ok_or_else(|| "--invoke needs a function name".to_string())?
                    .clone();
                i += 2;
            }
            other => {
                let n: i32 = other
                    .parse()
                    .map_err(|_| format!("bad i32 argument: {other}"))?;
                call_args.push(Value::I32(n));
                i += 1;
            }
        }
    }

    let results = instance.invoke(&name, &call_args).map_err(|e| e.to_string())?;
    Ok(format!("{} => [{}]", name, fmt_values(&results)))
}

fn wasi_cmd(args: &[String]) -> std::result::Result<String, String> {
    let path = args.first().ok_or_else(|| USAGE.to_string())?;
    let module = parse_module(&read_file(path)?).map_err(|e| e.to_string())?;

    let mut limits = ResourceLimits::default();
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--fuel" {
            let n: u64 = args
                .get(i + 1)
                .ok_or_else(|| "--fuel needs a number".to_string())?
                .parse()
                .map_err(|_| "bad fuel value".to_string())?;
            limits.fuel = Some(n);
            i += 2;
        } else {
            i += 1;
        }
    }

    let out = Instance::new(module)
        .run_wasi("_start", &limits, WasiCtx::new())
        .map_err(|e| e.to_string())?;
    let mut s = out.stdout_string();
    if let Some(code) = out.exit_code {
        s.push_str(&format!("\n[exit {code}]"));
    }
    Ok(s)
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
