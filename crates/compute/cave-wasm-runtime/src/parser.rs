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
    let mut r = Reader::new(bytes);

    // Preamble: magic + version.
    if r.take(4)? != WASM_MAGIC {
        return Err(WasmError::BadMagic);
    }
    let ver_bytes = r.take(4)?;
    let version = u32::from_le_bytes([ver_bytes[0], ver_bytes[1], ver_bytes[2], ver_bytes[3]]);
    if version != 1 {
        return Err(WasmError::BadVersion(version));
    }

    let mut module = Module::default();

    // Section loop.
    while r.remaining() > 0 {
        let id = r.byte()?;
        let size = r.u32()? as usize;
        let payload = r.take(size)?;
        let mut s = Reader::new(payload);
        match id {
            1 => module.types = parse_type_section(&mut s)?,
            3 => module.functions = parse_function_section(&mut s)?,
            5 => module.memory = parse_memory_section(&mut s)?,
            7 => module.exports = parse_export_section(&mut s)?,
            10 => module.code = parse_code_section(&mut s)?,
            // Custom (0) and other known-but-unmodelled sections are skipped.
            0 | 2 | 4 | 6 | 8 | 9 | 11 | 12 | 13 => {}
            other => return Err(WasmError::InvalidSection(other)),
        }
    }

    Ok(module)
}

fn parse_type_section(s: &mut Reader<'_>) -> Result<Vec<FuncType>> {
    let count = s.u32()?;
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let form = s.byte()?;
        if form != 0x60 {
            return Err(WasmError::InvalidValType(form));
        }
        let nparams = s.u32()?;
        let mut params = Vec::with_capacity(nparams as usize);
        for _ in 0..nparams {
            params.push(s.valtype()?);
        }
        let nresults = s.u32()?;
        let mut results = Vec::with_capacity(nresults as usize);
        for _ in 0..nresults {
            results.push(s.valtype()?);
        }
        out.push(FuncType { params, results });
    }
    Ok(out)
}

fn parse_function_section(s: &mut Reader<'_>) -> Result<Vec<u32>> {
    let count = s.u32()?;
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        out.push(s.u32()?);
    }
    Ok(out)
}

fn parse_limits(s: &mut Reader<'_>) -> Result<Limits> {
    let flag = s.byte()?;
    let min = s.u32()?;
    let max = if flag & 0x01 != 0 { Some(s.u32()?) } else { None };
    Ok(Limits { min, max })
}

fn parse_memory_section(s: &mut Reader<'_>) -> Result<Option<Limits>> {
    let count = s.u32()?;
    let mut first = None;
    for i in 0..count {
        let lim = parse_limits(s)?;
        if i == 0 {
            first = Some(lim);
        }
    }
    Ok(first)
}

fn parse_export_section(s: &mut Reader<'_>) -> Result<Vec<Export>> {
    let count = s.u32()?;
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let name = s.name()?;
        let kb = s.byte()?;
        let kind = ExternKind::from_byte(kb).ok_or(WasmError::InvalidSection(kb))?;
        let index = s.u32()?;
        out.push(Export { name, kind, index });
    }
    Ok(out)
}

fn parse_code_section(s: &mut Reader<'_>) -> Result<Vec<FuncBody>> {
    let count = s.u32()?;
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let body_size = s.u32()? as usize;
        let body = s.take(body_size)?;
        let mut b = Reader::new(body);
        let nlocal_decls = b.u32()?;
        let mut locals = Vec::with_capacity(nlocal_decls as usize);
        for _ in 0..nlocal_decls {
            let n = b.u32()?;
            let ty = b.valtype()?;
            locals.push((n, ty));
        }
        let code = body[b.pos..].to_vec();
        out.push(FuncBody { locals, code });
    }
    Ok(out)
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
