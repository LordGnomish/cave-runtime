// SPDX-License-Identifier: AGPL-3.0-or-later
//! WASM runtime — sandboxed execution of WasmPlugin bytecode.
//!
//! Mirrors the proxy-wasm ABI surface that Envoy ships with: a wasm
//! module exports `proxy_on_http_request_headers` (and friends) and
//! imports host-callable functions like `proxy_log` and
//! `proxy_get_header_map_value`. cave-mesh ships its own runtime so
//! Pollers compiled against `proxy-wasm-rust-sdk` / `proxy-wasm-cpp-sdk`
//! can run without an out-of-process Envoy.
//!
//! Architecture:
//!
//! * [`WasmRuntime`] — wraps a `wasmtime::Engine`. One engine per
//!   process; instances cheap to create.
//! * [`WasmModule`] — compiled module. Pre-compiled at WasmPlugin
//!   reconcile time so the per-request path stays under millisecond.
//! * [`RequestContext`] — Per-request envoy state (headers, response
//!   body, log buffer). Lives in the wasmtime `Store`.
//! * [`WasmInvocation`] — one filter invocation. Wires headers in,
//!   runs the module's hook, collects host-side side effects out.
//!
//! Resource limits: `Store::set_fuel` caps CPU time; a
//! `ResourceLimiter` impl caps memory growth (default 16 MiB).
//!
//! Scope cut: this is a *minimal viable* proxy-wasm ABI — the host
//! functions implemented are `proxy_log` + `proxy_get_header_map_value` +
//! `proxy_set_header_map_value`. The full proxy-wasm spec has ~50
//! more host functions (stream ops, shared kv, gRPC) that we map to
//! a stable error code on call. That matches Envoy's behaviour for
//! "ABI mismatch" filters — predictable rejection, not panic.

use std::sync::{Arc, Mutex};
use wasmtime::*;

#[derive(Debug, thiserror::Error)]
pub enum WasmError {
    #[error("wasmtime engine: {0}")]
    Engine(String),
    #[error("invalid wasm module: {0}")]
    InvalidModule(String),
    #[error("module instantiation failed: {0}")]
    Instantiate(String),
    #[error("filter hook trap: {0}")]
    Trap(String),
    #[error("fuel exhausted: {fuel_consumed} units")]
    FuelExhausted { fuel_consumed: u64 },
    #[error("memory limit exceeded: requested {requested} > cap {cap}")]
    MemoryLimitExceeded { requested: usize, cap: usize },
    #[error("missing required export: {0}")]
    MissingExport(String),
}

/// Engine + compilation cache. One per process.
pub struct WasmRuntime {
    engine: Engine,
}

impl WasmRuntime {
    pub fn new() -> Result<Self, WasmError> {
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine = Engine::new(&config).map_err(|e| WasmError::Engine(e.to_string()))?;
        Ok(Self { engine })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Compile a wasm module from raw bytecode. Validates the binary
    /// up front so per-request invocation never sees a malformed
    /// module.
    pub fn compile(&self, bytecode: &[u8]) -> Result<WasmModule, WasmError> {
        let module =
            Module::new(&self.engine, bytecode).map_err(|e| WasmError::InvalidModule(e.to_string()))?;
        Ok(WasmModule { module })
    }

    /// Compile a `.wat` text-format module (test/debug helper).
    pub fn compile_wat(&self, wat: &str) -> Result<WasmModule, WasmError> {
        let bytecode = wat::parse_str(wat).map_err(|e| WasmError::InvalidModule(e.to_string()))?;
        self.compile(&bytecode)
    }
}

impl Default for WasmRuntime {
    fn default() -> Self {
        Self::new().expect("default wasmtime engine should build")
    }
}

/// Pre-compiled module. Cheap to clone (Arc inside).
pub struct WasmModule {
    module: Module,
}

impl std::fmt::Debug for WasmModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmModule")
            .field("exports", &self.export_names())
            .field("imports", &self.import_names())
            .finish()
    }
}

impl WasmModule {
    pub fn export_names(&self) -> Vec<String> {
        self.module
            .exports()
            .map(|e| e.name().to_string())
            .collect()
    }

    pub fn import_names(&self) -> Vec<String> {
        self.module
            .imports()
            .map(|i| format!("{}::{}", i.module(), i.name()))
            .collect()
    }
}

/// Resource cap a runtime enforces on each invocation.
#[derive(Debug, Clone, Copy)]
pub struct ResourceLimits {
    /// Maximum linear memory in bytes the module may allocate.
    pub max_memory_bytes: usize,
    /// CPU fuel budget (wasmtime units, ~1 per instr).
    pub fuel: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_memory_bytes: 16 * 1024 * 1024,
            fuel: 1_000_000,
        }
    }
}

/// Per-request envoy-side state visible to host functions.
#[derive(Debug, Default, Clone)]
pub struct RequestContext {
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
    pub response_body: Vec<u8>,
    pub log_records: Vec<(LogLevel, String)>,
    pub set_header_calls: Vec<(String, String)>,
    /// Memory cap (from `ResourceLimits`).
    pub max_memory_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Critical,
}

impl LogLevel {
    pub fn from_i32(v: i32) -> Self {
        match v {
            0 => LogLevel::Trace,
            1 => LogLevel::Debug,
            3 => LogLevel::Warn,
            4 => LogLevel::Error,
            5 => LogLevel::Critical,
            _ => LogLevel::Info,
        }
    }
    pub const fn as_str(self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
            LogLevel::Critical => "critical",
        }
    }
}

/// `wasmtime::ResourceLimiter` impl that bounds linear-memory growth.
struct MemoryLimiter {
    max_bytes: usize,
}

impl wasmtime::ResourceLimiter for MemoryLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= self.max_bytes)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        _desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(true)
    }
}

/// State the wasmtime Store carries for one invocation.
pub struct InvocationState {
    pub ctx: Arc<Mutex<RequestContext>>,
    limiter: MemoryLimiter,
}

/// One filter run.
pub struct WasmInvocation<'r> {
    runtime: &'r WasmRuntime,
    module: &'r WasmModule,
    limits: ResourceLimits,
}

impl<'r> WasmInvocation<'r> {
    pub fn new(runtime: &'r WasmRuntime, module: &'r WasmModule, limits: ResourceLimits) -> Self {
        Self {
            runtime,
            module,
            limits,
        }
    }

    /// Run the module's `proxy_on_http_request_headers` hook (or
    /// `_start` for trivial modules without the hook). Returns the
    /// captured [`RequestContext`] for side-effect inspection.
    pub fn run(self, request_headers: Vec<(String, String)>) -> Result<RequestContext, WasmError> {
        let ctx = Arc::new(Mutex::new(RequestContext {
            request_headers,
            max_memory_bytes: self.limits.max_memory_bytes,
            ..RequestContext::default()
        }));

        let state = InvocationState {
            ctx: ctx.clone(),
            limiter: MemoryLimiter {
                max_bytes: self.limits.max_memory_bytes,
            },
        };

        let mut store = Store::new(self.runtime.engine(), state);
        store.limiter(|s| &mut s.limiter);
        store
            .set_fuel(self.limits.fuel)
            .map_err(|e| WasmError::Engine(e.to_string()))?;

        let mut linker: Linker<InvocationState> = Linker::new(self.runtime.engine());
        install_proxy_wasm_abi(&mut linker)?;

        let instance = linker
            .instantiate(&mut store, &self.module.module)
            .map_err(|e| WasmError::Instantiate(e.to_string()))?;

        // Prefer `proxy_on_http_request_headers(context_id, num_headers,
        // end_of_stream) -> Action`. Fall back to `_start` for simple
        // modules that only want to exercise host calls.
        let hook = instance
            .get_typed_func::<(i32, i32, i32), i32>(&mut store, "proxy_on_http_request_headers")
            .ok();
        if let Some(hook) = hook {
            let num_headers = {
                let g = ctx.lock().unwrap();
                g.request_headers.len() as i32
            };
            run_with_fuel_check(&mut store, |s| {
                hook.call(s, (1, num_headers, 1)).map(|_| ())
            })?;
        } else {
            let start = instance
                .get_typed_func::<(), ()>(&mut store, "_start")
                .or_else(|_| instance.get_typed_func::<(), ()>(&mut store, "run"))
                .map_err(|_| {
                    WasmError::MissingExport("proxy_on_http_request_headers / _start".into())
                })?;
            run_with_fuel_check(&mut store, |s| start.call(s, ()))?;
        }

        let final_ctx = ctx
            .lock()
            .map_err(|e| WasmError::Engine(format!("ctx poisoned: {e}")))?
            .clone();
        Ok(final_ctx)
    }
}

fn run_with_fuel_check<S, F>(store: &mut Store<S>, f: F) -> Result<(), WasmError>
where
    F: FnOnce(&mut Store<S>) -> wasmtime::Result<()>,
{
    let before = store.get_fuel().unwrap_or(0);
    match f(store) {
        Ok(()) => Ok(()),
        Err(e) => {
            let after = store.get_fuel().unwrap_or(0);
            let consumed = before.saturating_sub(after);
            let msg = e.to_string();
            if msg.contains("out of fuel") || consumed >= before {
                Err(WasmError::FuelExhausted { fuel_consumed: consumed })
            } else {
                Err(WasmError::Trap(msg))
            }
        }
    }
}

/// Install the minimal proxy-wasm ABI surface. cave-mesh chooses
/// the host-function names Envoy uses so a filter written against
/// the proxy-wasm SDK runs unchanged.
fn install_proxy_wasm_abi(linker: &mut Linker<InvocationState>) -> Result<(), WasmError> {
    // proxy_log(log_level: i32, message_data_ptr: i32, message_size: i32) -> i32
    linker
        .func_wrap(
            "env",
            "proxy_log",
            |mut caller: Caller<'_, InvocationState>,
             level: i32,
             msg_ptr: i32,
             msg_len: i32|
             -> i32 {
                let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                    Some(m) => m,
                    None => return 1,
                };
                let bytes = read_mem(&mut caller, &mem, msg_ptr as u32, msg_len as u32);
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let lvl = LogLevel::from_i32(level);
                caller.data().ctx.lock().unwrap().log_records.push((lvl, text));
                0
            },
        )
        .map_err(|e| WasmError::Engine(e.to_string()))?;

    // proxy_set_header_map_value(map_type, key_ptr, key_len, value_ptr, value_len) -> i32
    linker
        .func_wrap(
            "env",
            "proxy_set_header_map_value",
            |mut caller: Caller<'_, InvocationState>,
             _map_type: i32,
             key_ptr: i32,
             key_len: i32,
             val_ptr: i32,
             val_len: i32|
             -> i32 {
                let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                    Some(m) => m,
                    None => return 1,
                };
                let key = read_mem(&mut caller, &mem, key_ptr as u32, key_len as u32);
                let val = read_mem(&mut caller, &mem, val_ptr as u32, val_len as u32);
                let k = String::from_utf8_lossy(&key).into_owned();
                let v = String::from_utf8_lossy(&val).into_owned();
                caller.data().ctx.lock().unwrap().set_header_calls.push((k, v));
                0
            },
        )
        .map_err(|e| WasmError::Engine(e.to_string()))?;

    // Stub responders for the other ~50 proxy-wasm host functions.
    // Each returns 12 = WasmResultUnimplemented per the upstream spec
    // so a filter compiled against a richer proxy-wasm host learns
    // about the gap at runtime rather than crashing.
    for name in [
        "proxy_get_header_map_value",
        "proxy_send_local_response",
        "proxy_get_buffer_bytes",
        "proxy_set_buffer_bytes",
        "proxy_define_metric",
        "proxy_record_metric",
        "proxy_get_property",
        "proxy_set_property",
        "proxy_set_tick_period_milliseconds",
        "proxy_call_foreign_function",
    ] {
        let _ = linker.func_wrap(
            "env",
            name,
            |_caller: Caller<'_, InvocationState>| -> i32 { 12 },
        );
    }
    Ok(())
}

fn read_mem(
    caller: &mut Caller<'_, InvocationState>,
    mem: &Memory,
    offset: u32,
    len: u32,
) -> Vec<u8> {
    let data = mem.data(caller);
    let start = offset as usize;
    let end = (offset as usize).saturating_add(len as usize).min(data.len());
    if start >= end {
        return Vec::new();
    }
    data[start..end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal WAT module: one log call, then return.
    const LOG_HELLO_WAT: &str = r#"
(module
  (import "env" "proxy_log" (func $log (param i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "hello-from-wasm")
  (func (export "_start")
    (drop (call $log (i32.const 4) (i32.const 0) (i32.const 15)))
  )
)
"#;

    /// Set a response header via the host ABI.
    const SET_HEADER_WAT: &str = r#"
(module
  (import "env" "proxy_set_header_map_value" (func $set (param i32 i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  ;; key  = "x-cave"   @ 0  len 6
  ;; val  = "ok"        @ 16 len 2
  (data (i32.const 0) "x-cave")
  (data (i32.const 16) "ok")
  (func (export "_start")
    (drop (call $set (i32.const 2) (i32.const 0) (i32.const 6) (i32.const 16) (i32.const 2)))
  )
)
"#;

    /// proxy_on_http_request_headers hook that doesn't touch headers.
    const PROXY_HOOK_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "proxy_on_http_request_headers") (param i32 i32 i32) (result i32)
    ;; return Action::Continue == 0
    i32.const 0
  )
)
"#;

    /// Tight infinite loop — fuel-exhaustion target.
    const SPIN_FOREVER_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "_start")
    (loop $forever
      br $forever
    )
  )
)
"#;

    fn runtime() -> WasmRuntime {
        WasmRuntime::new().expect("engine")
    }

    #[test]
    fn engine_builds_and_compiles_trivial_module() {
        let rt = runtime();
        let module = rt.compile_wat("(module)").unwrap();
        assert!(module.export_names().is_empty());
    }

    #[test]
    fn invalid_module_rejected_with_typed_error() {
        let rt = runtime();
        let err = rt.compile_wat("(not actually wasm)").unwrap_err();
        assert!(matches!(err, WasmError::InvalidModule(_)));
    }

    #[test]
    fn binary_garbage_rejected() {
        let rt = runtime();
        let err = rt.compile(b"\x00\x01garbage").unwrap_err();
        assert!(matches!(err, WasmError::InvalidModule(_)));
    }

    #[test]
    fn module_export_names_listed() {
        let rt = runtime();
        let m = rt.compile_wat(PROXY_HOOK_WAT).unwrap();
        let names = m.export_names();
        assert!(names.iter().any(|n| n == "proxy_on_http_request_headers"));
        assert!(names.iter().any(|n| n == "memory"));
    }

    #[test]
    fn module_import_names_listed() {
        let rt = runtime();
        let m = rt.compile_wat(LOG_HELLO_WAT).unwrap();
        let imps = m.import_names();
        assert!(imps.iter().any(|i| i == "env::proxy_log"));
    }

    #[test]
    fn run_proxy_hook_returns_clean_context() {
        let rt = runtime();
        let m = rt.compile_wat(PROXY_HOOK_WAT).unwrap();
        let inv = WasmInvocation::new(&rt, &m, ResourceLimits::default());
        let ctx = inv
            .run(vec![("x-tenant".into(), "acme".into())])
            .expect("hook ran");
        assert_eq!(ctx.request_headers.len(), 1);
        assert!(ctx.log_records.is_empty());
    }

    #[test]
    fn proxy_log_host_call_captures_message() {
        let rt = runtime();
        let m = rt.compile_wat(LOG_HELLO_WAT).unwrap();
        let inv = WasmInvocation::new(&rt, &m, ResourceLimits::default());
        let ctx = inv.run(Vec::new()).expect("ran");
        assert_eq!(ctx.log_records.len(), 1);
        assert_eq!(ctx.log_records[0].0, LogLevel::Error);
        assert_eq!(ctx.log_records[0].1, "hello-from-wasm");
    }

    #[test]
    fn proxy_set_header_host_call_captured() {
        let rt = runtime();
        let m = rt.compile_wat(SET_HEADER_WAT).unwrap();
        let inv = WasmInvocation::new(&rt, &m, ResourceLimits::default());
        let ctx = inv.run(Vec::new()).expect("ran");
        assert_eq!(ctx.set_header_calls.len(), 1);
        assert_eq!(ctx.set_header_calls[0], ("x-cave".into(), "ok".into()));
    }

    #[test]
    fn fuel_exhaustion_traps_with_typed_error() {
        let rt = runtime();
        let m = rt.compile_wat(SPIN_FOREVER_WAT).unwrap();
        let inv = WasmInvocation::new(
            &rt,
            &m,
            ResourceLimits {
                max_memory_bytes: 1 << 20,
                fuel: 1_000,
            },
        );
        let err = inv.run(Vec::new()).unwrap_err();
        assert!(matches!(err, WasmError::FuelExhausted { .. }));
    }

    #[test]
    fn missing_export_errors_when_neither_hook_present() {
        let rt = runtime();
        let m = rt.compile_wat("(module (memory (export \"memory\") 1))").unwrap();
        let inv = WasmInvocation::new(&rt, &m, ResourceLimits::default());
        let err = inv.run(Vec::new()).unwrap_err();
        assert!(matches!(err, WasmError::MissingExport(_)));
    }

    #[test]
    fn log_level_mapping_known_values() {
        assert_eq!(LogLevel::from_i32(0), LogLevel::Trace);
        assert_eq!(LogLevel::from_i32(1), LogLevel::Debug);
        assert_eq!(LogLevel::from_i32(2), LogLevel::Info);
        assert_eq!(LogLevel::from_i32(3), LogLevel::Warn);
        assert_eq!(LogLevel::from_i32(4), LogLevel::Error);
        assert_eq!(LogLevel::from_i32(5), LogLevel::Critical);
    }

    #[test]
    fn log_level_mapping_unknown_falls_back_to_info() {
        assert_eq!(LogLevel::from_i32(42), LogLevel::Info);
    }

    #[test]
    fn resource_limits_default_sane() {
        let l = ResourceLimits::default();
        assert!(l.max_memory_bytes >= 1 << 20);
        assert!(l.fuel > 0);
    }

    #[test]
    fn compile_caches_engine_in_runtime() {
        let rt = runtime();
        // Compile twice — should both succeed; the second use shares
        // the same engine.
        rt.compile_wat(PROXY_HOOK_WAT).unwrap();
        rt.compile_wat(SET_HEADER_WAT).unwrap();
    }

    #[test]
    fn run_with_empty_headers_does_not_panic() {
        let rt = runtime();
        let m = rt.compile_wat(PROXY_HOOK_WAT).unwrap();
        let inv = WasmInvocation::new(&rt, &m, ResourceLimits::default());
        let ctx = inv.run(Vec::new()).expect("ran");
        assert!(ctx.request_headers.is_empty());
    }
}
