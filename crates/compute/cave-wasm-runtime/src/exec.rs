//! Stack-based WebAssembly interpreter.
//!
//! A straight tree-walking interpreter over the decoded instruction bytes,
//! modelled on the execution semantics documented by wasmtime's
//! `wasmtime-runtime` and the core spec. It implements the numeric/control
//! subset the engine targets; a Cranelift JIT backend is tracked honestly as
//! out-of-scope in the parity manifest.

use crate::error::{Result, WasmError};
use crate::types::{Module, ValType};
use serde::{Deserialize, Serialize};

/// A runtime value on the operand stack / in a local slot.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Value {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
}

impl Value {
    pub fn as_i32(&self) -> Result<i32> {
        match self {
            Value::I32(v) => Ok(*v),
            _ => Err(WasmError::Trap("expected i32".into())),
        }
    }

    pub fn as_i64(&self) -> Result<i64> {
        match self {
            Value::I64(v) => Ok(*v),
            _ => Err(WasmError::Trap("expected i64".into())),
        }
    }

    /// The zero value for a declared local of the given type.
    pub fn default_for(ty: ValType) -> Value {
        match ty {
            ValType::I32 => Value::I32(0),
            ValType::I64 => Value::I64(0),
            ValType::F32 => Value::F32(0.0),
            ValType::F64 => Value::F64(0.0),
        }
    }
}

/// An instantiated module ready to invoke.
#[derive(Debug, Clone)]
pub struct Instance {
    pub(crate) module: Module,
}

impl Instance {
    pub fn new(module: Module) -> Self {
        Instance { module }
    }

    pub fn module(&self) -> &Module {
        &self.module
    }

    /// Invoke an exported function by name with the given arguments.
    pub fn invoke(&self, name: &str, args: &[Value]) -> Result<Vec<Value>> {
        // RED stub — real interpreter implemented in the GREEN commit.
        let _ = (name, args);
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Export, ExternKind, FuncBody, FuncType, Module};

    /// Build a one-function module exported as "f".
    fn module1(params: Vec<ValType>, results: Vec<ValType>, code: Vec<u8>) -> Module {
        Module {
            types: vec![FuncType { params, results }],
            functions: vec![0],
            exports: vec![Export {
                name: "f".into(),
                kind: ExternKind::Func,
                index: 0,
            }],
            code: vec![FuncBody {
                locals: vec![],
                code,
            }],
            memory: None,
        }
    }

    #[test]
    fn i32_add() {
        let m = module1(
            vec![ValType::I32, ValType::I32],
            vec![ValType::I32],
            vec![0x20, 0x00, 0x20, 0x01, 0x6a, 0x0b],
        );
        let out = Instance::new(m)
            .invoke("f", &[Value::I32(2), Value::I32(3)])
            .unwrap();
        assert_eq!(out, vec![Value::I32(5)]);
    }

    #[test]
    fn i32_const_mul() {
        // i32.const 7; i32.const 6; i32.mul
        let m = module1(vec![], vec![ValType::I32], vec![0x41, 0x07, 0x41, 0x06, 0x6c, 0x0b]);
        let out = Instance::new(m).invoke("f", &[]).unwrap();
        assert_eq!(out, vec![Value::I32(42)]);
    }

    #[test]
    fn i32_sub_div() {
        // (a - b)  with a=20 b=4 -> 16, then nothing else; check sub and div_s
        let sub = module1(
            vec![ValType::I32, ValType::I32],
            vec![ValType::I32],
            vec![0x20, 0x00, 0x20, 0x01, 0x6b, 0x0b],
        );
        assert_eq!(
            Instance::new(sub)
                .invoke("f", &[Value::I32(20), Value::I32(4)])
                .unwrap(),
            vec![Value::I32(16)]
        );
        let div = module1(
            vec![ValType::I32, ValType::I32],
            vec![ValType::I32],
            vec![0x20, 0x00, 0x20, 0x01, 0x6d, 0x0b],
        );
        assert_eq!(
            Instance::new(div)
                .invoke("f", &[Value::I32(20), Value::I32(4)])
                .unwrap(),
            vec![Value::I32(5)]
        );
    }

    #[test]
    fn i32_eqz_and_compare() {
        let eqz = module1(
            vec![ValType::I32],
            vec![ValType::I32],
            vec![0x20, 0x00, 0x45, 0x0b],
        );
        assert_eq!(
            Instance::new(eqz.clone()).invoke("f", &[Value::I32(0)]).unwrap(),
            vec![Value::I32(1)]
        );
        assert_eq!(
            Instance::new(eqz).invoke("f", &[Value::I32(9)]).unwrap(),
            vec![Value::I32(0)]
        );
    }

    #[test]
    fn signed_const_is_decoded() {
        // i32.const -5 (0x7b in signed LEB128)
        let m = module1(vec![], vec![ValType::I32], vec![0x41, 0x7b, 0x0b]);
        assert_eq!(
            Instance::new(m).invoke("f", &[]).unwrap(),
            vec![Value::I32(-5)]
        );
    }

    #[test]
    fn divide_by_zero_traps() {
        let div = module1(
            vec![ValType::I32, ValType::I32],
            vec![ValType::I32],
            vec![0x20, 0x00, 0x20, 0x01, 0x6d, 0x0b],
        );
        let err = Instance::new(div)
            .invoke("f", &[Value::I32(1), Value::I32(0)])
            .unwrap_err();
        assert!(matches!(err, WasmError::Trap(_)));
    }

    #[test]
    fn cross_function_call() {
        // func0: add(a,b); func1(): call func0 with const 2,3
        let m = Module {
            types: vec![
                FuncType {
                    params: vec![ValType::I32, ValType::I32],
                    results: vec![ValType::I32],
                },
                FuncType {
                    params: vec![],
                    results: vec![ValType::I32],
                },
            ],
            functions: vec![0, 1],
            exports: vec![Export {
                name: "main".into(),
                kind: ExternKind::Func,
                index: 1,
            }],
            code: vec![
                FuncBody {
                    locals: vec![],
                    code: vec![0x20, 0x00, 0x20, 0x01, 0x6a, 0x0b],
                },
                FuncBody {
                    locals: vec![],
                    code: vec![0x41, 0x02, 0x41, 0x03, 0x10, 0x00, 0x0b],
                },
            ],
            memory: None,
        };
        assert_eq!(
            Instance::new(m).invoke("main", &[]).unwrap(),
            vec![Value::I32(5)]
        );
    }

    #[test]
    fn missing_export_errors() {
        let m = module1(vec![], vec![ValType::I32], vec![0x41, 0x01, 0x0b]);
        let err = Instance::new(m).invoke("nope", &[]).unwrap_err();
        assert!(matches!(err, WasmError::ExportNotFound(_)));
    }
}
