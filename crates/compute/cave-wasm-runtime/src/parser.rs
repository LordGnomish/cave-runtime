//! WebAssembly binary-format decoder.
//!
//! Ports the section-walking structure of wasmtime's `wasmparser` and wasmer's
//! `wasmer-compiler` front-ends: validate the 8-byte preamble, then iterate
//! length-prefixed sections decoding the ones the engine understands (type,
//! function, memory, export, code). LEB128 is decoded per the core spec.

use crate::error::{Result, WasmError};
use crate::types::{Export, ExternKind, FuncBody, FuncType, Limits, Module, ValType};

const WASM_MAGIC: [u8; 4] = [0x00, 0x61, 0x73, 0x6d];

/// A cursor over a byte slice with spec LEB128 / vector decoders.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    fn byte(&mut self) -> Result<u8> {
        let b = *self.buf.get(self.pos).ok_or(WasmError::UnexpectedEof)?;
        self.pos += 1;
        Ok(b)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(WasmError::UnexpectedEof);
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    /// Unsigned LEB128 (max 32-bit useful range, but decodes up to 5 bytes).
    fn u32(&mut self) -> Result<u32> {
        let mut result: u64 = 0;
        let mut shift = 0;
        loop {
            let b = self.byte()?;
            result |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
            if shift >= 35 {
                return Err(WasmError::InvalidLeb);
            }
        }
        Ok(result as u32)
    }

    fn valtype(&mut self) -> Result<ValType> {
        let b = self.byte()?;
        ValType::from_byte(b).ok_or(WasmError::InvalidValType(b))
    }

    fn name(&mut self) -> Result<String> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// Parse a WebAssembly binary module into the decoded [`Module`] model.
pub fn parse_module(bytes: &[u8]) -> Result<Module> {
    // RED stub — real decoder is implemented in the GREEN commit.
    let _ = bytes;
    Ok(Module::default())
}

#[cfg(test)]
pub(crate) fn add_module_bytes() -> Vec<u8> {
    // (module
    //   (type (func (param i32 i32) (result i32)))
    //   (func (type 0) local.get 0 local.get 1 i32.add)
    //   (memory 1)
    //   (export "add" (func 0)))
    vec![
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // preamble
        // type section: 1 functype (i32,i32)->(i32)
        0x01, 0x07, 0x01, 0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7f,
        // function section: func0 -> type0
        0x03, 0x02, 0x01, 0x00,
        // memory section: 1 memory, limits min=1
        0x05, 0x03, 0x01, 0x00, 0x01,
        // export section: "add" -> func 0
        0x07, 0x07, 0x01, 0x03, 0x61, 0x64, 0x64, 0x00, 0x00,
        // code section: func0 body: (local.get 0)(local.get 1)(i32.add)(end)
        0x0a, 0x09, 0x01, 0x07, 0x00, 0x20, 0x00, 0x20, 0x01, 0x6a, 0x0b,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_magic() {
        let err = parse_module(&[0, 0, 0, 0, 1, 0, 0, 0]).unwrap_err();
        assert_eq!(err, WasmError::BadMagic);
    }

    #[test]
    fn rejects_bad_version() {
        let mut b = add_module_bytes();
        b[4] = 0x02; // version 2
        assert_eq!(parse_module(&b).unwrap_err(), WasmError::BadVersion(2));
    }

    #[test]
    fn decodes_type_section() {
        let m = parse_module(&add_module_bytes()).unwrap();
        assert_eq!(m.types.len(), 1);
        assert_eq!(m.types[0].params, vec![ValType::I32, ValType::I32]);
        assert_eq!(m.types[0].results, vec![ValType::I32]);
    }

    #[test]
    fn decodes_functions_and_code() {
        let m = parse_module(&add_module_bytes()).unwrap();
        assert_eq!(m.functions, vec![0]);
        assert_eq!(m.code.len(), 1);
        // body: local.get 0, local.get 1, i32.add, end
        assert_eq!(m.code[0].code, vec![0x20, 0x00, 0x20, 0x01, 0x6a, 0x0b]);
        assert_eq!(m.code[0].local_count(), 0);
    }

    #[test]
    fn decodes_memory_limits() {
        let m = parse_module(&add_module_bytes()).unwrap();
        assert_eq!(m.memory, Some(Limits { min: 1, max: None }));
    }

    #[test]
    fn decodes_exports() {
        let m = parse_module(&add_module_bytes()).unwrap();
        assert_eq!(m.export_func("add"), Some(0));
        let _ = (Export {
            name: String::new(),
            kind: ExternKind::Func,
            index: 0,
        }, FuncBody::default(), FuncType::default());
    }
}
