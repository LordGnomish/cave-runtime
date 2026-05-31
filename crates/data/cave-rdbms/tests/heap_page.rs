// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of PostgreSQL's heap page layout and tuple identifiers:
//   * src/include/storage/bufpage.h  — PageHeaderData, ItemIdData line
//     pointers, PageInit / PageAddItem / PageGetItem / PageGetFreeSpace
//   * src/include/storage/itemptr.h  — ItemPointerData (block + offset TID)
//   * src/include/c.h                — MAXALIGN (8-byte alignment)
//
// A heap page is an 8 KiB slotted page: a fixed header, an array of 4-byte
// line pointers growing up from pd_lower, and MAXALIGN'd tuple bodies growing
// down from pd_upper. Items are addressed by a 1-based OffsetNumber; a heap
// tuple's full address is an ItemPointer (block number + offset number).

use cave_rdbms::storage::heap::{
    maxalign, HeapPage, ItemId, ItemPointer, BLCKSZ, LP_NORMAL, LP_UNUSED,
    SIZE_OF_ITEM_ID_DATA, SIZE_OF_PAGE_HEADER_DATA,
};

#[test]
fn block_size_and_header_constants() {
    assert_eq!(BLCKSZ, 8192);
    assert_eq!(SIZE_OF_PAGE_HEADER_DATA, 24);
    assert_eq!(SIZE_OF_ITEM_ID_DATA, 4);
}

#[test]
fn maxalign_rounds_up_to_eight() {
    assert_eq!(maxalign(0), 0);
    assert_eq!(maxalign(1), 8);
    assert_eq!(maxalign(8), 8);
    assert_eq!(maxalign(100), 104);
    assert_eq!(maxalign(104), 104);
}

#[test]
fn fresh_page_layout() {
    let p = HeapPage::new();
    // PageInit: pd_lower = header size, pd_upper = pd_special = BLCKSZ
    assert_eq!(p.lower(), SIZE_OF_PAGE_HEADER_DATA);
    assert_eq!(p.upper(), BLCKSZ);
    assert_eq!(p.item_count(), 0);
    // PageGetFreeSpace = (upper - lower) - sizeof(ItemIdData)
    assert_eq!(p.free_space(), BLCKSZ - SIZE_OF_PAGE_HEADER_DATA - SIZE_OF_ITEM_ID_DATA);
}

#[test]
fn add_item_returns_first_offset_and_maxaligns() {
    let mut p = HeapPage::new();
    let off = p.add_item(&[0xABu8; 100]).expect("room for first item");
    // FirstOffsetNumber == 1
    assert_eq!(off, 1);
    // line pointer consumed 4 bytes from the header end
    assert_eq!(p.lower(), SIZE_OF_PAGE_HEADER_DATA + SIZE_OF_ITEM_ID_DATA);
    // body placed MAXALIGN(100)=104 bytes below pd_upper
    assert_eq!(p.upper(), BLCKSZ - 104);
    assert_eq!(p.item_count(), 1);
}

#[test]
fn round_trips_item_bytes_and_increments_offsets() {
    let mut p = HeapPage::new();
    let a = vec![1u8, 2, 3, 4, 5];
    let b = vec![9u8; 17];
    let o1 = p.add_item(&a).unwrap();
    let o2 = p.add_item(&b).unwrap();
    assert_eq!(o1, 1);
    assert_eq!(o2, 2);
    assert_eq!(p.get_item(o1), Some(a.as_slice()));
    assert_eq!(p.get_item(o2), Some(b.as_slice()));
    // unknown / zero offset is None
    assert_eq!(p.get_item(0), None);
    assert_eq!(p.get_item(3), None);
}

#[test]
fn add_item_fails_when_no_room() {
    let mut p = HeapPage::new();
    // A single body larger than the page can never fit.
    assert_eq!(p.add_item(&vec![0u8; BLCKSZ]), None);
}

#[test]
fn item_pointer_block_and_offset() {
    let tid = ItemPointer::new(5, 3);
    assert_eq!(tid.block_number(), 5);
    assert_eq!(tid.offset_number(), 3);
    assert!(tid.is_valid());
    // offset 0 is the invalid sentinel (InvalidOffsetNumber)
    assert!(!ItemPointer::new(5, 0).is_valid());
    // BlockIdData splits the 32-bit block over hi/lo 16-bit halves
    let big = ItemPointer::new(0x0001_0002, 7);
    assert_eq!(big.block_number(), 65538);
    assert_eq!(big.offset_number(), 7);
}

#[test]
fn item_id_bitfield_round_trip() {
    // ItemIdData packs lp_off:15, lp_flags:2, lp_len:15 into one u32.
    let id = ItemId {
        off: 8088,
        flags: LP_NORMAL,
        len: 104,
    };
    let word = id.to_u32();
    let back = ItemId::from_u32(word);
    assert_eq!(back.off, 8088);
    assert_eq!(back.flags, LP_NORMAL);
    assert_eq!(back.len, 104);
    // a fresh/unused line pointer
    assert_eq!(ItemId::from_u32(0).flags, LP_UNUSED);
}
