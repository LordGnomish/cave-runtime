//! WASI preview 1 (`wasi_snapshot_preview1`) host shims.
//!
//! A focused, in-process subset of the preview-1 ABI sufficient to run typical
//! command guests: argument/environment introspection, `fd_write` to captured
//! stdout/stderr buffers, and `proc_exit`. The shapes follow the witx contract
//! used by wasmtime-wasi and wasmer-wasi. Filesystem, sockets and the full
//! clock surface are tracked honestly as out-of-scope in the parity manifest.

use crate::error::{Result, WasmError};
use crate::exec::Value;
use crate::limits::Store;

/// WASI errno: success.
pub const ERRNO_SUCCESS: i32 = 0;

/// Captured host state for a WASI guest run.
#[derive(Debug, Clone, Default)]
pub struct WasiCtx {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
}

impl WasiCtx {
    pub fn new() -> Self {
        WasiCtx::default()
    }

    /// stdout captured as a UTF-8 string (lossy).
    pub fn stdout_string(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

/// Dispatch a `wasi_snapshot_preview1` function by name.
///
/// RED stub — real shims are implemented in the GREEN commit.
pub fn dispatch(
    name: &str,
    _ctx: &mut WasiCtx,
    _store: &mut Store,
    _args: &[Value],
) -> Result<Vec<Value>> {
    let _ = name;
    Ok(vec![Value::I32(ERRNO_SUCCESS)])
}

/// Read a little-endian u32 pair (the iovec layout) — used by `fd_write`.
pub(crate) fn _unused() -> std::result::Result<(), WasmError> {
    Ok(())
}
