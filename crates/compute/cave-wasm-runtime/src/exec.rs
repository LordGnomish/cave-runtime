//! Stack-based WebAssembly interpreter.
//!
//! A straight tree-walking interpreter over the decoded instruction bytes,
//! modelled on the execution semantics documented by wasmtime's
//! `wasmtime-runtime` and the core spec. It implements the numeric/control
//! subset the engine targets; a Cranelift JIT backend is tracked honestly as
//! out-of-scope in the parity manifest.

use crate::error::{Result, WasmError};
use crate::limits::{ResourceLimits, Store};
use crate::sandbox::Capabilities;
use crate::types::{Module, ValType};
use crate::wasi::WasiCtx;
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
        self.invoke_with(name, args, &ResourceLimits::default())
    }

    /// Invoke an exported function under explicit resource limits (fuel +
    /// memory cap).
    pub fn invoke_with(
        &self,
        name: &str,
        args: &[Value],
        limits: &ResourceLimits,
    ) -> Result<Vec<Value>> {
        let idx = self
            .module
            .export_func(name)
            .ok_or_else(|| WasmError::ExportNotFound(name.to_string()))?;
        let mut store = Store::new(self.module.memory, limits);
        let mut wasi = WasiCtx::new();
        self.exec_func(idx, args.to_vec(), 0, &mut store, &mut wasi)
    }

    /// Run a WASI command guest: invoke `entry` (typically `_start`) with the
    /// supplied [`WasiCtx`], returning the context with captured stdout/stderr
    /// and any `proc_exit` code. A clean `proc_exit` is not treated as an error.
    pub fn run_wasi(
        &self,
        entry: &str,
        limits: &ResourceLimits,
        mut wasi: WasiCtx,
    ) -> Result<WasiCtx> {
        let idx = self
            .module
            .export_func(entry)
            .ok_or_else(|| WasmError::ExportNotFound(entry.to_string()))?;
        let mut store = Store::new(self.module.memory, limits);
        match self.exec_func(idx, Vec::new(), 0, &mut store, &mut wasi) {
            Ok(_) => Ok(wasi),
            // A guest `proc_exit` unwinds via a trap; treat it as a clean exit.
            Err(WasmError::Trap(_)) if wasi.exit_code.is_some() => Ok(wasi),
            Err(e) => Err(e),
        }
    }

    /// Run a WASI guest under a capability sandbox: host calls the capability
    /// set does not grant are denied with [`WasmError::CapabilityDenied`], and
    /// the capability set's fuel/memory limits are applied.
    pub fn run_sandboxed(
        &self,
        entry: &str,
        caps: &Capabilities,
        wasi: WasiCtx,
    ) -> Result<WasiCtx> {
        // RED stub — capability gating is wired into dispatch in the GREEN commit.
        self.run_wasi(entry, &caps.to_limits(), wasi)
    }

    /// Execute a function by index, returning its result values. `depth` guards
    /// against runaway recursion.
    pub(crate) fn exec_func(
        &self,
        func_idx: u32,
        args: Vec<Value>,
        depth: u32,
        store: &mut Store,
        wasi: &mut WasiCtx,
    ) -> Result<Vec<Value>> {
        if depth > 1024 {
            return Err(WasmError::Trap("call stack exhausted".into()));
        }
        let body = self
            .module
            .defined_body(func_idx)
            .ok_or(WasmError::IndexOutOfBounds(func_idx))?;

        // locals = params (the args) followed by zero-initialised declared locals.
        let mut locals: Vec<Value> = args;
        for (n, ty) in &body.locals {
            for _ in 0..*n {
                locals.push(Value::default_for(*ty));
            }
        }

        let code = &body.code;
        let mut stack: Vec<Value> = Vec::new();
        let mut pc = 0usize;

        macro_rules! pop {
            () => {
                stack.pop().ok_or(WasmError::StackUnderflow)?
            };
        }
        macro_rules! binop_i32 {
            ($f:expr) => {{
                let b = pop!().as_i32()?;
                let a = pop!().as_i32()?;
                stack.push(Value::I32($f(a, b)));
            }};
        }
        macro_rules! cmp_i32 {
            ($f:expr) => {{
                let b = pop!().as_i32()?;
                let a = pop!().as_i32()?;
                stack.push(Value::I32(if $f(a, b) { 1 } else { 0 }));
            }};
        }

        while pc < code.len() {
            store.charge()?;
            let op = code[pc];
            pc += 1;
            match op {
                0x0b => break,             // end
                0x0f => break,             // return (straight-line)
                0x1a => {
                    pop!();
                } // drop
                0x20 => {
                    let i = read_u32(code, &mut pc)? as usize;
                    let v = *locals.get(i).ok_or(WasmError::IndexOutOfBounds(i as u32))?;
                    stack.push(v);
                } // local.get
                0x21 => {
                    let i = read_u32(code, &mut pc)? as usize;
                    let v = pop!();
                    *locals.get_mut(i).ok_or(WasmError::IndexOutOfBounds(i as u32))? = v;
                } // local.set
                0x22 => {
                    let i = read_u32(code, &mut pc)? as usize;
                    let v = *stack.last().ok_or(WasmError::StackUnderflow)?;
                    *locals.get_mut(i).ok_or(WasmError::IndexOutOfBounds(i as u32))? = v;
                } // local.tee
                0x41 => {
                    let v = read_i32(code, &mut pc)?;
                    stack.push(Value::I32(v));
                } // i32.const
                0x42 => {
                    let v = read_i64(code, &mut pc)?;
                    stack.push(Value::I64(v));
                } // i64.const
                0x45 => {
                    let a = pop!().as_i32()?;
                    stack.push(Value::I32(if a == 0 { 1 } else { 0 }));
                } // i32.eqz
                0x46 => cmp_i32!(|a, b| a == b),
                0x47 => cmp_i32!(|a, b| a != b),
                0x48 => cmp_i32!(|a, b| a < b),
                0x49 => cmp_i32!(|a: i32, b: i32| (a as u32) < (b as u32)),
                0x4a => cmp_i32!(|a, b| a > b),
                0x4b => cmp_i32!(|a: i32, b: i32| (a as u32) > (b as u32)),
                0x4c => cmp_i32!(|a, b| a <= b),
                0x4d => cmp_i32!(|a: i32, b: i32| (a as u32) <= (b as u32)),
                0x4e => cmp_i32!(|a, b| a >= b),
                0x4f => cmp_i32!(|a: i32, b: i32| (a as u32) >= (b as u32)),
                0x6a => binop_i32!(|a: i32, b: i32| a.wrapping_add(b)),
                0x6b => binop_i32!(|a: i32, b: i32| a.wrapping_sub(b)),
                0x6c => binop_i32!(|a: i32, b: i32| a.wrapping_mul(b)),
                0x6d => {
                    let b = pop!().as_i32()?;
                    let a = pop!().as_i32()?;
                    if b == 0 {
                        return Err(WasmError::Trap("integer divide by zero".into()));
                    }
                    if a == i32::MIN && b == -1 {
                        return Err(WasmError::Trap("integer overflow".into()));
                    }
                    stack.push(Value::I32(a / b));
                } // i32.div_s
                0x6e => {
                    let b = pop!().as_i32()? as u32;
                    let a = pop!().as_i32()? as u32;
                    if b == 0 {
                        return Err(WasmError::Trap("integer divide by zero".into()));
                    }
                    stack.push(Value::I32((a / b) as i32));
                } // i32.div_u
                0x6f => {
                    let b = pop!().as_i32()?;
                    let a = pop!().as_i32()?;
                    if b == 0 {
                        return Err(WasmError::Trap("integer divide by zero".into()));
                    }
                    stack.push(Value::I32(a.wrapping_rem(b)));
                } // i32.rem_s
                0x70 => {
                    let b = pop!().as_i32()? as u32;
                    let a = pop!().as_i32()? as u32;
                    if b == 0 {
                        return Err(WasmError::Trap("integer divide by zero".into()));
                    }
                    stack.push(Value::I32((a % b) as i32));
                } // i32.rem_u
                0x71 => binop_i32!(|a: i32, b: i32| a & b),
                0x72 => binop_i32!(|a: i32, b: i32| a | b),
                0x73 => binop_i32!(|a: i32, b: i32| a ^ b),
                0x74 => binop_i32!(|a: i32, b: i32| a.wrapping_shl(b as u32)),
                0x75 => binop_i32!(|a: i32, b: i32| a.wrapping_shr(b as u32)),
                0x76 => binop_i32!(|a: i32, b: i32| ((a as u32).wrapping_shr(b as u32)) as i32),
                0x10 => {
                    // call
                    let callee = read_u32(code, &mut pc)?;
                    let ftype = self
                        .module
                        .func_type(callee)
                        .ok_or(WasmError::IndexOutOfBounds(callee))?;
                    let n = ftype.params.len();
                    if stack.len() < n {
                        return Err(WasmError::StackUnderflow);
                    }
                    let call_args = stack.split_off(stack.len() - n);
                    let imported = self.module.imported_func_count();
                    let results = if (callee as usize) < imported {
                        // Host (imported) function — dispatch to the WASI layer.
                        let name = self
                            .module
                            .imports
                            .iter()
                            .filter(|im| matches!(im.kind, crate::types::ImportKind::Func(_)))
                            .nth(callee as usize)
                            .map(|im| im.name.clone())
                            .ok_or(WasmError::IndexOutOfBounds(callee))?;
                        crate::wasi::dispatch(&name, wasi, store, &call_args)?
                    } else {
                        self.exec_func(callee, call_args, depth + 1, store, wasi)?
                    };
                    stack.extend(results);
                }
                0x28 => {
                    // i32.load
                    let (_align, offset) = read_memarg(code, &mut pc)?;
                    let addr = pop!().as_i32()? as u32;
                    stack.push(Value::I32(store.read_i32(addr, offset)?));
                }
                0x2d => {
                    // i32.load8_u
                    let (_align, offset) = read_memarg(code, &mut pc)?;
                    let addr = pop!().as_i32()? as u32;
                    stack.push(Value::I32(store.read_u8(addr, offset)? as i32));
                }
                0x36 => {
                    // i32.store
                    let (_align, offset) = read_memarg(code, &mut pc)?;
                    let value = pop!().as_i32()?;
                    let addr = pop!().as_i32()? as u32;
                    store.write_i32(addr, offset, value)?;
                }
                0x3a => {
                    // i32.store8
                    let (_align, offset) = read_memarg(code, &mut pc)?;
                    let value = pop!().as_i32()?;
                    let addr = pop!().as_i32()? as u32;
                    store.write_u8(addr, offset, (value & 0xff) as u8)?;
                }
                0x3f => {
                    // memory.size (mem index byte)
                    let _mem = read_u32(code, &mut pc)?;
                    stack.push(Value::I32(store.pages() as i32));
                }
                0x40 => {
                    // memory.grow (mem index byte)
                    let _mem = read_u32(code, &mut pc)?;
                    let delta = pop!().as_i32()? as u32;
                    stack.push(Value::I32(store.grow(delta)));
                }
                other => return Err(WasmError::UnsupportedOpcode(other)),
            }
        }

        Ok(stack)
    }
}

/// Unsigned LEB128 from a code stream at `*pc`.
fn read_u32(code: &[u8], pc: &mut usize) -> Result<u32> {
    let mut result: u64 = 0;
    let mut shift = 0;
    loop {
        let b = *code.get(*pc).ok_or(WasmError::UnexpectedEof)?;
        *pc += 1;
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

/// Decode a memory immediate (`align`, `offset`) — both unsigned LEB128.
fn read_memarg(code: &[u8], pc: &mut usize) -> Result<(u32, u32)> {
    let align = read_u32(code, pc)?;
    let offset = read_u32(code, pc)?;
    Ok((align, offset))
}

/// Signed LEB128 (32-bit) from a code stream at `*pc`.
fn read_i32(code: &[u8], pc: &mut usize) -> Result<i32> {
    let mut result: i64 = 0;
    let mut shift = 0;
    loop {
        let b = *code.get(*pc).ok_or(WasmError::UnexpectedEof)?;
        *pc += 1;
        result |= ((b & 0x7f) as i64) << shift;
        shift += 7;
        if b & 0x80 == 0 {
            if shift < 32 && (b & 0x40) != 0 {
                result |= -(1i64 << shift);
            }
            break;
        }
        if shift >= 35 {
            return Err(WasmError::InvalidLeb);
        }
    }
    Ok(result as i32)
}

/// Signed LEB128 (64-bit) from a code stream at `*pc`.
fn read_i64(code: &[u8], pc: &mut usize) -> Result<i64> {
    let mut result: i64 = 0;
    let mut shift = 0;
    loop {
        let b = *code.get(*pc).ok_or(WasmError::UnexpectedEof)?;
        *pc += 1;
        result |= ((b & 0x7f) as i64) << shift;
        shift += 7;
        if b & 0x80 == 0 {
            if shift < 64 && (b & 0x40) != 0 {
                result |= -(1i64 << shift);
            }
            break;
        }
        if shift >= 70 {
            return Err(WasmError::InvalidLeb);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Export, ExternKind, FuncBody, FuncType, Module};

    /// Build a one-function module exported as "f".
    fn module1(params: Vec<ValType>, results: Vec<ValType>, code: Vec<u8>) -> Module {
        Module {
            types: vec![FuncType { params, results }],
            imports: vec![],
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
            imports: vec![],
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

    // ---- cycle 3: resource limits (fuel + linear memory) ----

    use crate::limits::ResourceLimits;
    use crate::types::Limits as MemLimits;

    fn module1_mem(
        params: Vec<ValType>,
        results: Vec<ValType>,
        code: Vec<u8>,
        pages: u32,
    ) -> Module {
        let mut m = module1(params, results, code);
        m.memory = Some(MemLimits {
            min: pages,
            max: Some(pages),
        });
        m
    }

    #[test]
    fn fuel_exhaustion_traps() {
        // add(2,3) needs ~5 instrs; give it 1 unit of fuel.
        let m = module1(
            vec![ValType::I32, ValType::I32],
            vec![ValType::I32],
            vec![0x20, 0x00, 0x20, 0x01, 0x6a, 0x0b],
        );
        let err = Instance::new(m)
            .invoke_with(
                "f",
                &[Value::I32(2), Value::I32(3)],
                &ResourceLimits::with_fuel(1),
            )
            .unwrap_err();
        assert_eq!(err, WasmError::FuelExhausted);
    }

    #[test]
    fn fuel_sufficient_runs() {
        let m = module1(
            vec![ValType::I32, ValType::I32],
            vec![ValType::I32],
            vec![0x20, 0x00, 0x20, 0x01, 0x6a, 0x0b],
        );
        let out = Instance::new(m)
            .invoke_with(
                "f",
                &[Value::I32(2), Value::I32(3)],
                &ResourceLimits::with_fuel(100),
            )
            .unwrap();
        assert_eq!(out, vec![Value::I32(5)]);
    }

    #[test]
    fn memory_store_then_load() {
        // i32.const 0; i32.const 42; i32.store; i32.const 0; i32.load
        let code = vec![
            0x41, 0x00, 0x41, 0x2a, 0x36, 0x02, 0x00, // store value 42 @ addr 0
            0x41, 0x00, 0x28, 0x02, 0x00, // load @ addr 0
            0x0b,
        ];
        let m = module1_mem(vec![], vec![ValType::I32], code, 1);
        let out = Instance::new(m).invoke("f", &[]).unwrap();
        assert_eq!(out, vec![Value::I32(42)]);
    }

    #[test]
    fn memory_size_and_grow() {
        // memory.size -> 1 ; then grow by 2 -> returns prev size 1
        let size = module1_mem(vec![], vec![ValType::I32], vec![0x3f, 0x00, 0x0b], 1);
        assert_eq!(
            Instance::new(size).invoke("f", &[]).unwrap(),
            vec![Value::I32(1)]
        );
        let grow = module1_mem(
            vec![],
            vec![ValType::I32],
            vec![0x41, 0x02, 0x40, 0x00, 0x0b],
            1,
        );
        // module max == 1 page, so growing by 2 must fail with -1.
        assert_eq!(
            Instance::new(grow).invoke("f", &[]).unwrap(),
            vec![Value::I32(-1)]
        );
    }

    #[test]
    fn memory_out_of_bounds_traps() {
        // store at a huge address -> trap
        let code = vec![0x41, 0xff, 0xff, 0x7f, 0x41, 0x01, 0x36, 0x02, 0x00, 0x0b];
        let m = module1_mem(vec![], vec![], code, 1);
        let err = Instance::new(m).invoke("f", &[]).unwrap_err();
        assert!(matches!(err, WasmError::MemoryOutOfBounds { .. }));
    }

    // ---- cycle 4: WASI preview1 host dispatch ----

    use crate::types::{Import, ImportKind};
    use crate::wasi::WasiCtx;

    #[test]
    fn wasi_fd_write_captures_stdout() {
        // import fd_write : (i32,i32,i32,i32)->i32   (combined func index 0)
        // defined _start  : ()->()                    (combined func index 1)
        // _start: write "hi" into memory, build an iovec, call fd_write(1, ..).
        let code = vec![
            0x41, 0x10, 0x41, 0xe8, 0x00, 0x3a, 0x00, 0x00, // mem[16] = 'h'
            0x41, 0x11, 0x41, 0xe9, 0x00, 0x3a, 0x00, 0x00, // mem[17] = 'i'
            0x41, 0x00, 0x41, 0x10, 0x36, 0x02, 0x00, // iovec.buf = 16 @ mem[0]
            0x41, 0x04, 0x41, 0x02, 0x36, 0x02, 0x00, // iovec.len = 2  @ mem[4]
            0x41, 0x01, 0x41, 0x00, 0x41, 0x01, 0x41, 0x08, // fd, iovs, n, nwritten
            0x10, 0x00, // call fd_write
            0x1a, // drop errno
            0x0b, // end
        ];
        let m = Module {
            types: vec![
                FuncType {
                    params: vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
                    results: vec![ValType::I32],
                },
                FuncType {
                    params: vec![],
                    results: vec![],
                },
            ],
            imports: vec![Import {
                module: "wasi_snapshot_preview1".into(),
                name: "fd_write".into(),
                kind: ImportKind::Func(0),
            }],
            functions: vec![1],
            exports: vec![Export {
                name: "_start".into(),
                kind: ExternKind::Func,
                index: 1,
            }],
            code: vec![FuncBody {
                locals: vec![],
                code,
            }],
            memory: Some(MemLimits {
                min: 1,
                max: Some(1),
            }),
        };
        let out = Instance::new(m)
            .run_wasi("_start", &ResourceLimits::default(), WasiCtx::new())
            .unwrap();
        assert_eq!(out.stdout_string(), "hi");
        // nwritten written back to mem[8]
    }

    #[test]
    fn wasi_proc_exit_sets_code() {
        // import proc_exit : (i32)->()  (index 0); _start calls proc_exit(7)
        let m = Module {
            types: vec![
                FuncType {
                    params: vec![ValType::I32],
                    results: vec![],
                },
                FuncType {
                    params: vec![],
                    results: vec![],
                },
            ],
            imports: vec![Import {
                module: "wasi_snapshot_preview1".into(),
                name: "proc_exit".into(),
                kind: ImportKind::Func(0),
            }],
            functions: vec![1],
            exports: vec![Export {
                name: "_start".into(),
                kind: ExternKind::Func,
                index: 1,
            }],
            code: vec![FuncBody {
                locals: vec![],
                code: vec![0x41, 0x07, 0x10, 0x00, 0x0b],
            }],
            memory: None,
        };
        let out = Instance::new(m)
            .run_wasi("_start", &ResourceLimits::default(), WasiCtx::new())
            .unwrap();
        assert_eq!(out.exit_code, Some(7));
    }

    // ---- cycle 6: capability sandbox bridge ----

    /// A guest that writes "hi" to stdout via fd_write (import index 0).
    fn fd_write_hi_module() -> Module {
        let code = vec![
            0x41, 0x10, 0x41, 0xe8, 0x00, 0x3a, 0x00, 0x00,
            0x41, 0x11, 0x41, 0xe9, 0x00, 0x3a, 0x00, 0x00,
            0x41, 0x00, 0x41, 0x10, 0x36, 0x02, 0x00,
            0x41, 0x04, 0x41, 0x02, 0x36, 0x02, 0x00,
            0x41, 0x01, 0x41, 0x00, 0x41, 0x01, 0x41, 0x08,
            0x10, 0x00, 0x1a, 0x0b,
        ];
        Module {
            types: vec![
                FuncType {
                    params: vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
                    results: vec![ValType::I32],
                },
                FuncType {
                    params: vec![],
                    results: vec![],
                },
            ],
            imports: vec![Import {
                module: "wasi_snapshot_preview1".into(),
                name: "fd_write".into(),
                kind: ImportKind::Func(0),
            }],
            functions: vec![1],
            exports: vec![Export {
                name: "_start".into(),
                kind: ExternKind::Func,
                index: 1,
            }],
            code: vec![FuncBody {
                locals: vec![],
                code,
            }],
            memory: Some(MemLimits {
                min: 1,
                max: Some(1),
            }),
        }
    }

    #[test]
    fn sandbox_denies_stdout() {
        use crate::sandbox::Capabilities;
        // stdout denied, but enough fuel/memory to reach the fd_write call.
        let caps = Capabilities::none().with_fuel(1000).with_memory_pages(1);
        let err = Instance::new(fd_write_hi_module())
            .run_sandboxed("_start", &caps, WasiCtx::new())
            .unwrap_err();
        assert!(matches!(err, WasmError::CapabilityDenied(_)));
    }

    #[test]
    fn sandbox_allows_granted_stdout() {
        use crate::sandbox::Capabilities;
        let caps = Capabilities::none()
            .allow_stdio()
            .with_fuel(1000)
            .with_memory_pages(1);
        let out = Instance::new(fd_write_hi_module())
            .run_sandboxed("_start", &caps, WasiCtx::new())
            .unwrap();
        assert_eq!(out.stdout_string(), "hi");
    }
}
