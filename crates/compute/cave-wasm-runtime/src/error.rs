//! Error type for the wasm runtime. Hand-rolled (no thiserror dep) to keep the
//! crate's dependency surface minimal.

use std::fmt;

/// All failures surfaced by parsing, validation and execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WasmError {
    /// Module did not begin with the `\0asm` magic.
    BadMagic,
    /// Unsupported binary format version (only v1 is accepted).
    BadVersion(u32),
    /// Ran off the end of the byte stream while decoding.
    UnexpectedEof,
    /// A LEB128 integer was malformed (too long / unterminated).
    InvalidLeb,
    /// Unknown value-type byte in the type section.
    InvalidValType(u8),
    /// Unknown section id.
    InvalidSection(u8),
    /// Reference to a function/type/export index that does not exist.
    IndexOutOfBounds(u32),
    /// An unknown or unsupported opcode was reached during execution.
    UnsupportedOpcode(u8),
    /// The operand stack underflowed (malformed code).
    StackUnderflow,
    /// A guest-triggered trap (e.g. unreachable, divide-by-zero).
    Trap(String),
    /// Fuel was exhausted before the program finished.
    FuelExhausted,
    /// A linear-memory access fell outside the allocated bounds.
    MemoryOutOfBounds { addr: u32, len: u32 },
    /// A WASI host call was denied by the sandbox capability set.
    CapabilityDenied(String),
    /// Requested export was not found in the module.
    ExportNotFound(String),
}

impl fmt::Display for WasmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WasmError::BadMagic => write!(f, "not a wasm module (bad magic)"),
            WasmError::BadVersion(v) => write!(f, "unsupported wasm version {v}"),
            WasmError::UnexpectedEof => write!(f, "unexpected end of input"),
            WasmError::InvalidLeb => write!(f, "malformed LEB128 integer"),
            WasmError::InvalidValType(b) => write!(f, "invalid value type 0x{b:02x}"),
            WasmError::InvalidSection(b) => write!(f, "invalid section id {b}"),
            WasmError::IndexOutOfBounds(i) => write!(f, "index {i} out of bounds"),
            WasmError::UnsupportedOpcode(op) => write!(f, "unsupported opcode 0x{op:02x}"),
            WasmError::StackUnderflow => write!(f, "operand stack underflow"),
            WasmError::Trap(m) => write!(f, "wasm trap: {m}"),
            WasmError::FuelExhausted => write!(f, "fuel exhausted"),
            WasmError::MemoryOutOfBounds { addr, len } => {
                write!(f, "memory access out of bounds at {addr}+{len}")
            }
            WasmError::CapabilityDenied(c) => write!(f, "capability denied: {c}"),
            WasmError::ExportNotFound(n) => write!(f, "export not found: {n}"),
        }
    }
}

impl std::error::Error for WasmError {}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, WasmError>;
