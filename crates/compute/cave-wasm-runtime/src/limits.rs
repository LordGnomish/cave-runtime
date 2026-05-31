//! Resource limits and the mutable execution store (linear memory + fuel).
//!
//! Fuel metering mirrors wasmtime's `Config::consume_fuel` model (each executed
//! instruction costs one unit; running out traps). Linear memory follows the
//! wasm32 page model (64 KiB pages, `memory.grow` returns the previous size or
//! -1 when it would exceed the maximum).

use crate::error::{Result, WasmError};
use crate::types::Limits;

/// WebAssembly linear-memory page size (64 KiB).
pub const PAGE_SIZE: usize = 65536;

/// Caller-supplied execution limits.
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    /// Maximum instructions to execute, or `None` for unmetered.
    pub fuel: Option<u64>,
    /// Hard cap on linear-memory pages, regardless of the module's declared max.
    pub max_memory_pages: u32,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        // wasm32 architectural maximum is 65536 pages (4 GiB).
        ResourceLimits {
            fuel: None,
            max_memory_pages: 65536,
        }
    }
}

impl ResourceLimits {
    /// Convenience: a metered limit with the given fuel budget.
    pub fn with_fuel(fuel: u64) -> Self {
        ResourceLimits {
            fuel: Some(fuel),
            ..Default::default()
        }
    }
}

/// Mutable per-execution state: the linear memory and the remaining fuel.
#[derive(Debug, Clone)]
pub struct Store {
    memory: Vec<u8>,
    max_pages: u32,
    fuel: Option<u64>,
}

impl Store {
    /// Build a store for a module's declared memory under the given limits.
    pub fn new(module_mem: Option<Limits>, limits: &ResourceLimits) -> Self {
        let min = module_mem.map(|m| m.min).unwrap_or(0);
        let declared_max = module_mem.and_then(|m| m.max).unwrap_or(limits.max_memory_pages);
        let max_pages = declared_max.min(limits.max_memory_pages);
        Store {
            memory: vec![0u8; min as usize * PAGE_SIZE],
            max_pages,
            fuel: limits.fuel,
        }
    }

    /// Current memory size in pages.
    pub fn pages(&self) -> u32 {
        (self.memory.len() / PAGE_SIZE) as u32
    }

    /// Remaining fuel, if metered.
    pub fn fuel(&self) -> Option<u64> {
        self.fuel
    }

    /// Charge one unit of fuel; trap when the budget is exhausted.
    pub fn charge(&mut self) -> Result<()> {
        if let Some(f) = self.fuel {
            if f == 0 {
                return Err(WasmError::FuelExhausted);
            }
            self.fuel = Some(f - 1);
        }
        Ok(())
    }

    /// `memory.grow`: returns the previous page count, or -1 if it would exceed
    /// the maximum.
    pub fn grow(&mut self, delta: u32) -> i32 {
        let cur = self.pages();
        let new = cur as u64 + delta as u64;
        if new > self.max_pages as u64 {
            return -1;
        }
        self.memory.resize(new as usize * PAGE_SIZE, 0);
        cur as i32
    }

    fn range(&self, addr: u32, offset: u32, len: u32) -> Result<std::ops::Range<usize>> {
        let start = (addr as u64) + (offset as u64);
        let end = start + len as u64;
        if end > self.memory.len() as u64 {
            return Err(WasmError::MemoryOutOfBounds { addr, len });
        }
        Ok(start as usize..end as usize)
    }

    pub fn read_i32(&self, addr: u32, offset: u32) -> Result<i32> {
        let r = self.range(addr, offset, 4)?;
        let b = &self.memory[r];
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn write_i32(&mut self, addr: u32, offset: u32, value: i32) -> Result<()> {
        let r = self.range(addr, offset, 4)?;
        self.memory[r].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    pub fn read_u8(&self, addr: u32, offset: u32) -> Result<u8> {
        let r = self.range(addr, offset, 1)?;
        Ok(self.memory[r.start])
    }

    pub fn write_u8(&mut self, addr: u32, offset: u32, value: u8) -> Result<()> {
        let r = self.range(addr, offset, 1)?;
        self.memory[r.start] = value;
        Ok(())
    }

    /// Read a byte range (used by WASI host shims to read guest buffers).
    pub fn read_bytes(&self, addr: u32, len: u32) -> Result<&[u8]> {
        let r = self.range(addr, 0, len)?;
        Ok(&self.memory[r])
    }

    /// Write a byte range (used by WASI host shims to fill guest buffers).
    pub fn write_bytes(&mut self, addr: u32, data: &[u8]) -> Result<()> {
        let r = self.range(addr, 0, data.len() as u32)?;
        self.memory[r].copy_from_slice(data);
        Ok(())
    }
}
