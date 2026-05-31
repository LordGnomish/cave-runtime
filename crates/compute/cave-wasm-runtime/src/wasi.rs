//! WASI preview 1 (`wasi_snapshot_preview1`) host shims.
//!
//! A focused, in-process subset of the preview-1 ABI sufficient to run typical
//! command guests: argument/environment introspection, `fd_write` to captured
//! stdout/stderr buffers, `clock_time_get`, and `proc_exit`. The shapes follow
//! the witx contract used by wasmtime-wasi and wasmer-wasi. Filesystem, sockets
//! and the rich clock/poll surface are tracked honestly as out-of-scope in the
//! parity manifest.

use crate::error::{Result, WasmError};
use crate::exec::Value;
use crate::limits::Store;

/// WASI errno: success.
pub const ERRNO_SUCCESS: i32 = 0;
/// WASI errno: bad file descriptor.
pub const ERRNO_BADF: i32 = 8;

/// The module name WASI functions are imported from.
pub const WASI_MODULE: &str = "wasi_snapshot_preview1";

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

    /// Builder: set command-line arguments.
    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// Builder: set environment variables.
    pub fn with_env<I, K, V>(mut self, env: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.env = env.into_iter().map(|(k, v)| (k.into(), v.into())).collect();
        self
    }

    /// stdout captured as a UTF-8 string (lossy).
    pub fn stdout_string(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// stderr captured as a UTF-8 string (lossy).
    pub fn stderr_string(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

fn arg(args: &[Value], i: usize) -> Result<i32> {
    args.get(i)
        .ok_or_else(|| WasmError::Trap("wasi: missing argument".into()))?
        .as_i32()
}

/// Dispatch a `wasi_snapshot_preview1` function by name.
///
/// Returns the WASI errno (or, for `proc_exit`, a trap that `run_wasi` treats
/// as a clean exit). Unknown functions trap rather than silently succeeding.
pub fn dispatch(
    name: &str,
    ctx: &mut WasiCtx,
    store: &mut Store,
    args: &[Value],
) -> Result<Vec<Value>> {
    match name {
        "fd_write" => fd_write(ctx, store, args),
        "fd_read" => {
            // No input streams wired; report zero bytes read (EOF).
            let nread_ptr = arg(args, 3)? as u32;
            store.write_i32(nread_ptr, 0, 0)?;
            Ok(vec![Value::I32(ERRNO_SUCCESS)])
        }
        "proc_exit" => {
            ctx.exit_code = Some(arg(args, 0)?);
            Err(WasmError::Trap("wasi proc_exit".into()))
        }
        "args_sizes_get" => sizes_get(store, args, &collect_args(ctx)),
        "args_get" => values_get(store, args, &collect_args(ctx)),
        "environ_sizes_get" => sizes_get(store, args, &collect_env(ctx)),
        "environ_get" => values_get(store, args, &collect_env(ctx)),
        "clock_time_get" => {
            // Deterministic: report epoch 0. (timestamp written as 8 bytes)
            let out_ptr = arg(args, 2)? as u32;
            store.write_i32(out_ptr, 0, 0)?;
            store.write_i32(out_ptr, 4, 0)?;
            Ok(vec![Value::I32(ERRNO_SUCCESS)])
        }
        other => Err(WasmError::Trap(format!("unknown wasi import: {other}"))),
    }
}

fn collect_args(ctx: &WasiCtx) -> Vec<Vec<u8>> {
    ctx.args
        .iter()
        .map(|a| {
            let mut b = a.clone().into_bytes();
            b.push(0);
            b
        })
        .collect()
}

fn collect_env(ctx: &WasiCtx) -> Vec<Vec<u8>> {
    ctx.env
        .iter()
        .map(|(k, v)| {
            let mut b = format!("{k}={v}").into_bytes();
            b.push(0);
            b
        })
        .collect()
}

/// `fd_write(fd, iovs, iovs_len, nwritten) -> errno`.
fn fd_write(ctx: &mut WasiCtx, store: &mut Store, args: &[Value]) -> Result<Vec<Value>> {
    let fd = arg(args, 0)?;
    let iovs = arg(args, 1)? as u32;
    let iovs_len = arg(args, 2)? as u32;
    let nwritten_ptr = arg(args, 3)? as u32;

    let sink: &mut Vec<u8> = match fd {
        1 => &mut ctx.stdout,
        2 => &mut ctx.stderr,
        _ => return Ok(vec![Value::I32(ERRNO_BADF)]),
    };

    let mut total: u32 = 0;
    for i in 0..iovs_len {
        let base = iovs + i * 8;
        let buf_ptr = store.read_i32(base, 0)? as u32;
        let buf_len = store.read_i32(base, 4)? as u32;
        let bytes = store.read_bytes(buf_ptr, buf_len)?;
        sink.extend_from_slice(bytes);
        total += buf_len;
    }
    store.write_i32(nwritten_ptr, 0, total as i32)?;
    Ok(vec![Value::I32(ERRNO_SUCCESS)])
}

/// `*_sizes_get(count_ptr, buf_size_ptr) -> errno`.
fn sizes_get(store: &mut Store, args: &[Value], items: &[Vec<u8>]) -> Result<Vec<Value>> {
    let count_ptr = arg(args, 0)? as u32;
    let buf_size_ptr = arg(args, 1)? as u32;
    let count = items.len() as i32;
    let buf_size: usize = items.iter().map(|b| b.len()).sum();
    store.write_i32(count_ptr, 0, count)?;
    store.write_i32(buf_size_ptr, 0, buf_size as i32)?;
    Ok(vec![Value::I32(ERRNO_SUCCESS)])
}

/// `*_get(ptrs_ptr, buf_ptr) -> errno` — writes the pointer table then the
/// NUL-terminated strings packed into the buffer.
fn values_get(store: &mut Store, args: &[Value], items: &[Vec<u8>]) -> Result<Vec<Value>> {
    let ptrs_ptr = arg(args, 0)? as u32;
    let buf_ptr = arg(args, 1)? as u32;
    let mut cursor = buf_ptr;
    for (i, item) in items.iter().enumerate() {
        store.write_i32(ptrs_ptr + (i as u32) * 4, 0, cursor as i32)?;
        store.write_bytes(cursor, item)?;
        cursor += item.len() as u32;
    }
    Ok(vec![Value::I32(ERRNO_SUCCESS)])
}
