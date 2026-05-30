// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Probe attach registry — userspace model of grafana/beyla's
//! kprobe / uprobe / tracepoint attachment (pkg/internal/ebpf).
//!
//! Attaching a kprobe requires the target to exist in the kernel symbol
//! table (`/proc/kallsyms`); attaching a uprobe requires resolving a
//! symbol to a file offset inside a target binary's ELF symbol table, or
//! an explicit offset. Each successful attach yields an `fd`-like link
//! that can be detached.
//!
//! This registry reproduces those checks and the link lifecycle over
//! in-memory symbol tables. The real `perf_event_open(2)` + `PERF_*`
//! ioctl attach is the userspace-approximation boundary, tracked honestly
//! as a `partial` subsystem (`perf-event-attach`) in the manifest.

use std::collections::{HashMap, HashSet};

/// Link identifier returned by an attach call.
pub type LinkId = u64;

/// Probe flavour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeKind {
    Kprobe,
    Uprobe,
    Tracepoint,
}

/// A live attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub id: LinkId,
    pub kind: ProbeKind,
    pub program: String,
    /// Kernel symbol, `category:name` tracepoint, or binary path.
    pub target: String,
    /// Resolved file offset for uprobes.
    pub offset: Option<u64>,
    /// `true` for kretprobe / uretprobe.
    pub is_return: bool,
}

/// Attach-time errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    /// Kernel or binary symbol not found in the resolver table.
    UnknownSymbol(String),
    /// Tracepoint category or name was empty.
    BadTracepoint,
    /// Uprobe given neither a resolvable symbol nor an explicit offset.
    MissingLocation,
    /// Detach of an unknown link.
    NotFound,
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProbeError::UnknownSymbol(s) => write!(f, "unknown symbol '{s}'"),
            ProbeError::BadTracepoint => write!(f, "tracepoint category and name must be non-empty"),
            ProbeError::MissingLocation => write!(f, "uprobe needs a resolvable symbol or an offset"),
            ProbeError::NotFound => write!(f, "no such link"),
        }
    }
}

impl std::error::Error for ProbeError {}

/// Registry of probe attachments backed by in-memory symbol tables.
#[derive(Debug, Default, Clone)]
pub struct ProbeRegistry {
    kallsyms: HashSet<String>,
    /// path -> (symbol -> offset)
    usyms: HashMap<String, HashMap<String, u64>>,
    links: Vec<Attachment>,
    next_id: LinkId,
}

impl ProbeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a kernel symbol (as if read from `/proc/kallsyms`).
    pub fn add_kernel_symbol(&mut self, symbol: &str) {
        self.kallsyms.insert(symbol.to_string());
    }

    /// Register a userspace symbol→offset for a binary's ELF symbol table.
    pub fn add_binary_symbol(&mut self, path: &str, symbol: &str, offset: u64) {
        self.usyms
            .entry(path.to_string())
            .or_default()
            .insert(symbol.to_string(), offset);
    }

    fn push(&mut self, att: Attachment) -> LinkId {
        let id = att.id;
        self.links.push(att);
        id
    }

    /// Attach a kprobe (or kretprobe when `is_return`) to a kernel symbol.
    pub fn attach_kprobe(
        &mut self,
        program: &str,
        symbol: &str,
        is_return: bool,
    ) -> Result<LinkId, ProbeError> {
        if !self.kallsyms.contains(symbol) {
            return Err(ProbeError::UnknownSymbol(symbol.to_string()));
        }
        let id = self.next_id;
        self.next_id += 1;
        Ok(self.push(Attachment {
            id,
            kind: ProbeKind::Kprobe,
            program: program.to_string(),
            target: symbol.to_string(),
            offset: None,
            is_return,
        }))
    }

    /// Attach a tracepoint identified by `category` + `name`.
    pub fn attach_tracepoint(
        &mut self,
        program: &str,
        category: &str,
        name: &str,
    ) -> Result<LinkId, ProbeError> {
        if category.is_empty() || name.is_empty() {
            return Err(ProbeError::BadTracepoint);
        }
        let id = self.next_id;
        self.next_id += 1;
        Ok(self.push(Attachment {
            id,
            kind: ProbeKind::Tracepoint,
            program: program.to_string(),
            target: format!("{category}:{name}"),
            offset: None,
            is_return: false,
        }))
    }

    /// Attach a uprobe (or uretprobe when `is_return`). Resolves `symbol`
    /// against the binary's symbol table, or uses an explicit `offset`.
    pub fn attach_uprobe(
        &mut self,
        program: &str,
        path: &str,
        symbol: Option<&str>,
        offset: Option<u64>,
        is_return: bool,
    ) -> Result<LinkId, ProbeError> {
        let resolved = match (symbol, offset) {
            (Some(sym), _) => {
                let off = self
                    .usyms
                    .get(path)
                    .and_then(|t| t.get(sym))
                    .copied()
                    .ok_or_else(|| ProbeError::UnknownSymbol(sym.to_string()))?;
                off
            }
            (None, Some(off)) => off,
            (None, None) => return Err(ProbeError::MissingLocation),
        };
        let id = self.next_id;
        self.next_id += 1;
        Ok(self.push(Attachment {
            id,
            kind: ProbeKind::Uprobe,
            program: program.to_string(),
            target: path.to_string(),
            offset: Some(resolved),
            is_return,
        }))
    }

    /// Detach a link by id.
    pub fn detach(&mut self, id: LinkId) -> Result<(), ProbeError> {
        let before = self.links.len();
        self.links.retain(|a| a.id != id);
        if self.links.len() == before {
            Err(ProbeError::NotFound)
        } else {
            Ok(())
        }
    }

    /// Number of live links.
    pub fn active(&self) -> usize {
        self.links.len()
    }

    /// Look up a link by id.
    pub fn link(&self, id: LinkId) -> Option<&Attachment> {
        self.links.iter().find(|a| a.id == id)
    }

    /// All links owned by a program.
    pub fn links_for_program(&self, program: &str) -> Vec<&Attachment> {
        self.links.iter().filter(|a| a.program == program).collect()
    }
}
