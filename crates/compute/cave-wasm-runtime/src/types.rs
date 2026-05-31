//! Core WebAssembly module data model (decoded form).

use serde::{Deserialize, Serialize};

/// WebAssembly value types (the numeric subset this engine executes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValType {
    I32,
    I64,
    F32,
    F64,
}

impl ValType {
    /// Decode the single-byte value-type encoding.
    pub fn from_byte(b: u8) -> Option<ValType> {
        match b {
            0x7f => Some(ValType::I32),
            0x7e => Some(ValType::I64),
            0x7d => Some(ValType::F32),
            0x7c => Some(ValType::F64),
            _ => None,
        }
    }
}

/// A function signature: parameter and result value types.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FuncType {
    pub params: Vec<ValType>,
    pub results: Vec<ValType>,
}

/// The kind of entity an export refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExternKind {
    Func,
    Table,
    Memory,
    Global,
}

impl ExternKind {
    pub fn from_byte(b: u8) -> Option<ExternKind> {
        match b {
            0x00 => Some(ExternKind::Func),
            0x01 => Some(ExternKind::Table),
            0x02 => Some(ExternKind::Memory),
            0x03 => Some(ExternKind::Global),
            _ => None,
        }
    }
}

/// A named export and the index it points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Export {
    pub name: String,
    pub kind: ExternKind,
    pub index: u32,
}

/// What an import resolves to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportKind {
    /// Imported function with the given type index.
    Func(u32),
    Table,
    Memory(Limits),
    Global,
}

/// A `(module, name)` import declaration. WASI functions arrive as function
/// imports from the `wasi_snapshot_preview1` module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Import {
    pub module: String,
    pub name: String,
    pub kind: ImportKind,
}

/// Min/max limits (pages for memory, elements for tables).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub min: u32,
    pub max: Option<u32>,
}

/// A function body: declared locals (run-length encoded) plus raw instruction
/// bytes. The instruction stream is decoded lazily by the interpreter.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FuncBody {
    /// Run-length encoded locals: (count, type).
    pub locals: Vec<(u32, ValType)>,
    /// Raw instruction bytes (without the trailing `end`-terminated wrapper size).
    pub code: Vec<u8>,
}

impl FuncBody {
    /// Total number of local slots declared (excluding parameters).
    pub fn local_count(&self) -> u32 {
        self.locals.iter().map(|(n, _)| *n).sum()
    }
}

/// A fully decoded module.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Module {
    pub types: Vec<FuncType>,
    pub imports: Vec<Import>,
    /// type index for each *defined* function (imports are not included).
    pub functions: Vec<u32>,
    pub exports: Vec<Export>,
    pub code: Vec<FuncBody>,
    pub memory: Option<Limits>,
}

impl Module {
    /// Look up an exported function index by name.
    pub fn export_func(&self, name: &str) -> Option<u32> {
        self.exports
            .iter()
            .find(|e| e.kind == ExternKind::Func && e.name == name)
            .map(|e| e.index)
    }

    /// Type indices of imported functions, in import order. These occupy the
    /// low end of the function index space.
    pub fn imported_func_type_indices(&self) -> Vec<u32> {
        self.imports
            .iter()
            .filter_map(|i| match i.kind {
                ImportKind::Func(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    /// Number of imported functions (the offset at which defined functions
    /// begin in the combined index space).
    pub fn imported_func_count(&self) -> usize {
        self.imports
            .iter()
            .filter(|i| matches!(i.kind, ImportKind::Func(_)))
            .count()
    }

    /// Signature of any function (imported or defined) by combined index.
    pub fn func_type(&self, func_idx: u32) -> Option<&FuncType> {
        let imp = self.imported_func_type_indices();
        let idx = func_idx as usize;
        if idx < imp.len() {
            self.types.get(imp[idx] as usize)
        } else {
            let tidx = *self.functions.get(idx - imp.len())?;
            self.types.get(tidx as usize)
        }
    }

    /// The defined-function body for a combined index, or `None` if the index
    /// refers to an imported (host) function.
    pub fn defined_body(&self, func_idx: u32) -> Option<&FuncBody> {
        let n = self.imported_func_count();
        let idx = func_idx as usize;
        if idx < n {
            None
        } else {
            self.code.get(idx - n)
        }
    }
}
