// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Heap page layout and tuple identifiers.
//!
//! Pure-Rust port of PostgreSQL's slotted heap page
//! (`src/include/storage/bufpage.h`) and tuple-identifier
//! (`src/include/storage/itemptr.h`) machinery.
//!
//! A page is `BLCKSZ` (8 KiB) of storage divided into three regions: a fixed
//! [`SIZE_OF_PAGE_HEADER_DATA`]-byte header, an array of 4-byte line pointers
//! ([`ItemId`]) growing **up** from `pd_lower`, and `MAXALIGN`'d tuple bodies
//! growing **down** from `pd_upper`. New tuples are appended with
//! [`HeapPage::add_item`], which returns a 1-based `OffsetNumber`; a tuple's
//! full address across the relation is an [`ItemPointer`] (block + offset).

/// `BLCKSZ` — the on-disk page size.
pub const BLCKSZ: usize = 8192;
/// `SizeOfPageHeaderData` — the heap page header with no special space.
pub const SIZE_OF_PAGE_HEADER_DATA: usize = 24;
/// `sizeof(ItemIdData)` — one line pointer.
pub const SIZE_OF_ITEM_ID_DATA: usize = 4;

/// `LP_UNUSED` — line pointer is available for re-use.
pub const LP_UNUSED: u8 = 0;
/// `LP_NORMAL` — line pointer points to a live/dead tuple (`lp_len` valid).
pub const LP_NORMAL: u8 = 1;
/// `LP_REDIRECT` — HOT redirect to another line pointer.
pub const LP_REDIRECT: u8 = 2;
/// `LP_DEAD` — dead, awaiting vacuum; storage reclaimable.
pub const LP_DEAD: u8 = 3;

/// `MAXALIGN` — round `n` up to the 8-byte alignment boundary postgres uses
/// for all on-page tuple bodies.
pub const fn maxalign(n: usize) -> usize {
    (n + 7) & !7
}

/// `ItemIdData` — a 4-byte line pointer packing `lp_off:15`, `lp_flags:2`,
/// `lp_len:15` into a single 32-bit word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemId {
    /// byte offset of the tuple body from the start of the page
    pub off: u16,
    /// `LP_*` state
    pub flags: u8,
    /// byte length of the tuple body
    pub len: u16,
}

impl ItemId {
    /// Pack into the C bitfield layout (`lp_off` low 15, `lp_flags` next 2,
    /// `lp_len` high 15).
    pub fn to_u32(self) -> u32 {
        let off = (self.off as u32) & 0x7FFF;
        let flags = (self.flags as u32) & 0x3;
        let len = (self.len as u32) & 0x7FFF;
        off | (flags << 15) | (len << 17)
    }

    /// Unpack from the 32-bit bitfield word.
    pub fn from_u32(word: u32) -> Self {
        ItemId {
            off: (word & 0x7FFF) as u16,
            flags: ((word >> 15) & 0x3) as u8,
            len: ((word >> 17) & 0x7FFF) as u16,
        }
    }
}

/// `ItemPointerData` — a tuple's full address: a 32-bit block number (stored
/// as two 16-bit halves in upstream `BlockIdData`) plus a 1-based
/// `OffsetNumber`. Offset 0 is the `InvalidOffsetNumber` sentinel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemPointer {
    block: u32,
    offset: u16,
}

impl ItemPointer {
    /// `ItemPointerSet(blkno, offnum)`.
    pub fn new(block: u32, offset: u16) -> Self {
        ItemPointer { block, offset }
    }

    /// `ItemPointerGetBlockNumber` — reconstruct the block from its hi/lo
    /// `BlockIdData` halves.
    pub fn block_number(&self) -> u32 {
        let hi = (self.block >> 16) & 0xFFFF;
        let lo = self.block & 0xFFFF;
        (hi << 16) | lo
    }

    /// `ItemPointerGetOffsetNumber`.
    pub fn offset_number(&self) -> u16 {
        self.offset
    }

    /// `ItemPointerIsValid` — the offset is a real `OffsetNumber` (non-zero).
    pub fn is_valid(&self) -> bool {
        self.offset != 0
    }
}

/// An 8 KiB slotted heap page.
#[derive(Debug, Clone)]
pub struct HeapPage {
    /// line pointers, in OffsetNumber order (index 0 == offset 1)
    line_pointers: Vec<ItemId>,
    /// tuple bodies keyed by the OffsetNumber that owns them
    bodies: Vec<Vec<u8>>,
    pd_lower: usize,
    pd_upper: usize,
}

impl Default for HeapPage {
    fn default() -> Self {
        Self::new()
    }
}

impl HeapPage {
    /// `PageInit` — an empty page: `pd_lower` at the header end, `pd_upper`
    /// (and `pd_special`) at `BLCKSZ`.
    pub fn new() -> Self {
        HeapPage {
            line_pointers: Vec::new(),
            bodies: Vec::new(),
            pd_lower: SIZE_OF_PAGE_HEADER_DATA,
            pd_upper: BLCKSZ,
        }
    }

    /// `((PageHeader) page)->pd_lower`.
    pub fn lower(&self) -> usize {
        self.pd_lower
    }

    /// `((PageHeader) page)->pd_upper`.
    pub fn upper(&self) -> usize {
        self.pd_upper
    }

    /// Number of allocated line pointers.
    pub fn item_count(&self) -> usize {
        self.line_pointers.len()
    }

    /// `PageGetFreeSpace` — bytes available for one more (line pointer + body):
    /// `(pd_upper - pd_lower) - sizeof(ItemIdData)`, clamped at 0.
    pub fn free_space(&self) -> usize {
        let space = self.pd_upper.saturating_sub(self.pd_lower);
        space.saturating_sub(SIZE_OF_ITEM_ID_DATA)
    }

    /// `PageAddItem` (append flavour) — store a MAXALIGN'd tuple body, append
    /// its line pointer, and return the new 1-based `OffsetNumber`. Returns
    /// `None` when the body plus a line pointer will not fit.
    pub fn add_item(&mut self, item: &[u8]) -> Option<u16> {
        let aligned = maxalign(item.len());
        // Need room for the body (between lower+lp and upper) and the new
        // line pointer.
        if self.pd_upper < aligned
            || self.pd_upper - aligned < self.pd_lower + SIZE_OF_ITEM_ID_DATA
        {
            return None;
        }
        self.pd_upper -= aligned;
        let id = ItemId {
            off: self.pd_upper as u16,
            flags: LP_NORMAL,
            len: item.len() as u16,
        };
        self.line_pointers.push(id);
        self.bodies.push(item.to_vec());
        self.pd_lower += SIZE_OF_ITEM_ID_DATA;
        Some(self.line_pointers.len() as u16)
    }

    /// `PageGetItem` — the tuple body addressed by a 1-based `OffsetNumber`,
    /// or `None` for an out-of-range / unused / invalid offset.
    pub fn get_item(&self, offset: u16) -> Option<&[u8]> {
        if offset == 0 {
            return None;
        }
        let idx = (offset - 1) as usize;
        let id = self.line_pointers.get(idx)?;
        if id.flags != LP_NORMAL {
            return None;
        }
        self.bodies.get(idx).map(|v| v.as_slice())
    }

    /// Mark an item's line pointer dead (`LP_DEAD`) — vacuum will reclaim it.
    pub fn mark_dead(&mut self, offset: u16) -> bool {
        if offset == 0 {
            return false;
        }
        match self.line_pointers.get_mut((offset - 1) as usize) {
            Some(id) => {
                id.flags = LP_DEAD;
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marking_dead_hides_the_item() {
        let mut p = HeapPage::new();
        let off = p.add_item(&[1, 2, 3]).unwrap();
        assert!(p.get_item(off).is_some());
        assert!(p.mark_dead(off));
        assert_eq!(p.get_item(off), None);
    }

    #[test]
    fn free_space_shrinks_as_items_are_added() {
        let mut p = HeapPage::new();
        let f0 = p.free_space();
        p.add_item(&[0u8; 40]).unwrap();
        assert!(p.free_space() < f0);
    }
}
