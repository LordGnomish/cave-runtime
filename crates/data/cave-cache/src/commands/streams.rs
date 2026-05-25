// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Stream commands: XADD, XREAD, XRANGE, XREVRANGE, XLEN, XTRIM, XDEL, XINFO,
//! XGROUP CREATE/DESTROY/CREATECONSUMER/DELCONSUMER, XREADGROUP, XACK, XPENDING,
//! XCLAIM, XAUTOCLAIM.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::db::Db;
use crate::error::{CacheError, CacheResult};
use crate::resp::Resp;
use crate::types::{
    Consumer, ConsumerGroup, Entry, PendingEntry, Stream, StreamEntry, StreamId, Value,
    bytes_to_i64,
};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── XADD ─────────────────────────────────────────────────────────────────────

pub fn cmd_xadd(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 5 {
        return Err(CacheError::wrong_arity("xadd"));
    }

    let key = args[1].clone();
    let mut maxlen: Option<(bool, usize)> = None; // (approximate, count)
    let mut minid: Option<StreamId> = None;
    let mut i = 2;

    // Parse options
    loop {
        if i >= args.len() {
            return Err(CacheError::Syntax);
        }
        match args[i].to_ascii_uppercase().as_slice() {
            b"MAXLEN" => {
                i += 1;
                let approx = if i < args.len() && args[i] == b"~" {
                    i += 1;
                    true
                } else {
                    false
                };
                let n = bytes_to_i64(&args[i]).ok_or(CacheError::NotInteger)? as usize;
                maxlen = Some((approx, n));
                i += 1;
            }
            b"MINID" => {
                i += 1;
                let _approx = if i < args.len() && args[i] == b"~" {
                    i += 1;
                    true
                } else {
                    false
                };
                minid = Some(
                    StreamId::parse(&args[i])
                        .ok_or_else(|| CacheError::generic("ERR Invalid stream ID"))?,
                );
                i += 1;
            }
            b"NOMKSTREAM" => {
                i += 1;
            }
            _ => break,
        }
    }

    if i >= args.len() {
        return Err(CacheError::Syntax);
    }
    let id_raw = &args[i];
    i += 1;

    if (args.len() - i) % 2 != 0 {
        return Err(CacheError::Syntax);
    }

    let mut fields = Vec::new();
    while i < args.len() {
        fields.push((args[i].clone(), args[i + 1].clone()));
        i += 2;
    }

    let stream = match db.get_typed_mut(&key, "stream")? {
        Some(e) => match &mut e.value {
            Value::Stream(s) => s as *mut Stream,
            _ => unreachable!(),
        },
        None => {
            db.insert(key.clone(), Entry::new(Value::Stream(Stream::new())));
            match db.get_typed_mut(&key, "stream")? {
                Some(e) => match &mut e.value {
                    Value::Stream(s) => s as *mut Stream,
                    _ => unreachable!(),
                },
                None => unreachable!(),
            }
        }
    };
    let stream = unsafe { &mut *stream };

    // Generate ID
    let ms = now_ms();
    let id = if id_raw == b"*" {
        let seq = if stream.last_id.ms == ms {
            stream.last_id.seq + 1
        } else {
            0
        };
        StreamId { ms, seq }
    } else {
        let parsed = StreamId::parse(id_raw).ok_or_else(|| {
            CacheError::generic("ERR Invalid stream ID specified as stream command argument")
        })?;
        // Handle partial ID like "123-*"
        if id_raw.ends_with(b"-*") || id_raw.ends_with(b"*") && !id_raw.starts_with(b"*") {
            // Use existing ms, increment seq
            if parsed.ms < stream.last_id.ms
                || (parsed.ms == stream.last_id.ms && parsed.seq <= stream.last_id.seq)
            {
                return Err(CacheError::StreamIdTooSmall);
            }
        } else if parsed <= stream.last_id {
            return Err(CacheError::StreamIdTooSmall);
        }
        parsed
    };

    stream.last_id = id;
    let entry = StreamEntry { id, fields };
    stream.entries.push(entry);

    // Apply MAXLEN trimming
    if let Some((_, max)) = maxlen {
        if stream.entries.len() > max {
            let trim_count = stream.entries.len() - max;
            stream.entries.drain(..trim_count);
        }
    }

    Ok(Resp::BulkString(Some(id.to_string().into_bytes())))
}

// ── XREAD ────────────────────────────────────────────────────────────────────

pub fn cmd_xread(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 4 {
        return Err(CacheError::wrong_arity("xread"));
    }

    let mut count: Option<usize> = None;
    let mut block: Option<u64> = None;
    let mut i = 1;

    while i < args.len() {
        match args[i].to_ascii_uppercase().as_slice() {
            b"COUNT" => {
                i += 1;
                count = Some(bytes_to_i64(&args[i]).ok_or(CacheError::NotInteger)? as usize);
                i += 1;
            }
            b"BLOCK" => {
                i += 1;
                block = Some(bytes_to_i64(&args[i]).ok_or(CacheError::NotInteger)? as u64);
                i += 1;
            }
            b"STREAMS" => {
                i += 1;
                break;
            }
            _ => {
                i += 1;
            }
        }
    }

    let streams_start = i;
    let num_streams = (args.len() - streams_start) / 2;
    if num_streams == 0 {
        return Err(CacheError::Syntax);
    }

    let keys = &args[streams_start..streams_start + num_streams];
    let ids = &args[streams_start + num_streams..];

    let mut results = Vec::new();
    for (key, id_raw) in keys.iter().zip(ids.iter()) {
        let from_id = if id_raw == b"$" {
            match db.get_typed(key, "stream")? {
                Some(e) => match &e.value {
                    Value::Stream(s) => s.last_id,
                    _ => unreachable!(),
                },
                None => StreamId::zero(),
            }
        } else {
            StreamId::parse(id_raw).ok_or_else(|| CacheError::generic("ERR Invalid stream ID"))?
        };

        let entries = match db.get_typed(key, "stream")? {
            Some(e) => match &e.value {
                Value::Stream(s) => {
                    let take = count.unwrap_or(usize::MAX);
                    s.entries
                        .iter()
                        .filter(|e| e.id > from_id)
                        .take(take)
                        .map(stream_entry_to_resp)
                        .collect::<Vec<_>>()
                }
                _ => unreachable!(),
            },
            None => vec![],
        };

        if !entries.is_empty() {
            results.push(Resp::Array(Some(vec![
                Resp::BulkString(Some(key.clone())),
                Resp::Array(Some(entries)),
            ])));
        }
    }

    if results.is_empty() {
        Ok(Resp::nil_array())
    } else {
        Ok(Resp::Array(Some(results)))
    }
}

fn stream_entry_to_resp(entry: &StreamEntry) -> Resp {
    let mut fields = Vec::new();
    for (k, v) in &entry.fields {
        fields.push(Resp::BulkString(Some(k.clone())));
        fields.push(Resp::BulkString(Some(v.clone())));
    }
    Resp::Array(Some(vec![
        Resp::BulkString(Some(entry.id.to_string().into_bytes())),
        Resp::Array(Some(fields)),
    ]))
}

// ── XRANGE / XREVRANGE ───────────────────────────────────────────────────────

pub fn cmd_xrange(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    xrange_impl(args, db, false)
}

pub fn cmd_xrevrange(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    xrange_impl(args, db, true)
}

fn xrange_impl(args: &[Vec<u8>], db: &mut Db, rev: bool) -> CacheResult<Resp> {
    let cmd = if rev { "xrevrange" } else { "xrange" };
    if args.len() < 4 {
        return Err(CacheError::wrong_arity(cmd));
    }

    let (start_raw, end_raw) = if rev {
        (&args[3], &args[2])
    } else {
        (&args[2], &args[3])
    };
    let count = if args.len() > 4 && args[4].to_ascii_uppercase() == b"COUNT" {
        Some(bytes_to_i64(&args[5]).ok_or(CacheError::NotInteger)? as usize)
    } else {
        None
    };

    let start = if start_raw == b"-" {
        StreamId::zero()
    } else {
        StreamId::parse(start_raw).ok_or_else(|| CacheError::generic("ERR Invalid stream ID"))?
    };
    let end = if end_raw == b"+" {
        StreamId {
            ms: u64::MAX,
            seq: u64::MAX,
        }
    } else {
        StreamId::parse(end_raw).ok_or_else(|| CacheError::generic("ERR Invalid stream ID"))?
    };

    match db.get_typed(&args[1], "stream")? {
        Some(e) => match &e.value {
            Value::Stream(s) => {
                let take = count.unwrap_or(usize::MAX);
                let mut entries: Vec<&StreamEntry> = s
                    .entries
                    .iter()
                    .filter(|e| e.id >= start && e.id <= end)
                    .collect();
                if rev {
                    entries.reverse();
                }
                let result: Vec<Resp> = entries
                    .into_iter()
                    .take(take)
                    .map(stream_entry_to_resp)
                    .collect();
                Ok(Resp::Array(Some(result)))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Array(Some(vec![]))),
    }
}

// ── XLEN ─────────────────────────────────────────────────────────────────────

pub fn cmd_xlen(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 2 {
        return Err(CacheError::wrong_arity("xlen"));
    }
    match db.get_typed(&args[1], "stream")? {
        Some(e) => match &e.value {
            Value::Stream(s) => Ok(Resp::Integer(s.entries.len() as i64)),
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

// ── XTRIM ────────────────────────────────────────────────────────────────────

pub fn cmd_xtrim(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 4 {
        return Err(CacheError::wrong_arity("xtrim"));
    }
    let key = &args[1];
    let strategy = &args[2];
    let mut i = 3;
    let approx = if i < args.len() && args[i] == b"~" {
        i += 1;
        true
    } else {
        false
    };
    let threshold = bytes_to_i64(&args[i]).ok_or(CacheError::NotInteger)?;

    match db.get_typed_mut(key, "stream")? {
        Some(entry) => match &mut entry.value {
            Value::Stream(s) => {
                let before = s.entries.len();
                match strategy.to_ascii_uppercase().as_slice() {
                    b"MAXLEN" => {
                        let max = threshold as usize;
                        if s.entries.len() > max {
                            let remove = s.entries.len() - max;
                            s.entries.drain(..remove);
                        }
                    }
                    b"MINID" => {
                        let min_id = StreamId::parse(&args[i])
                            .ok_or_else(|| CacheError::generic("ERR Invalid stream ID"))?;
                        s.entries.retain(|e| e.id >= min_id);
                    }
                    _ => return Err(CacheError::Syntax),
                }
                let removed = before - s.entries.len();
                Ok(Resp::Integer(removed as i64))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

// ── XDEL ─────────────────────────────────────────────────────────────────────

pub fn cmd_xdel(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 {
        return Err(CacheError::wrong_arity("xdel"));
    }
    let key = &args[1];
    let ids: Vec<StreamId> = args[2..]
        .iter()
        .map(|id| StreamId::parse(id).ok_or_else(|| CacheError::generic("ERR Invalid stream ID")))
        .collect::<CacheResult<_>>()?;

    match db.get_typed_mut(key, "stream")? {
        Some(entry) => match &mut entry.value {
            Value::Stream(s) => {
                let before = s.entries.len();
                s.entries.retain(|e| !ids.contains(&e.id));
                Ok(Resp::Integer((before - s.entries.len()) as i64))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

// ── XINFO ────────────────────────────────────────────────────────────────────

pub fn cmd_xinfo(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 {
        return Err(CacheError::wrong_arity("xinfo"));
    }
    match args[2].to_ascii_uppercase().as_slice() {
        b"STREAM" => {
            if args.len() < 4 {
                return Err(CacheError::wrong_arity("xinfo stream"));
            }
            match db.get_typed(&args[3], "stream")? {
                Some(e) => match &e.value {
                    Value::Stream(s) => {
                        let first_id = s
                            .entries
                            .first()
                            .map(|e| e.id.to_string())
                            .unwrap_or_default();
                        let last_id = s
                            .entries
                            .last()
                            .map(|e| e.id.to_string())
                            .unwrap_or_default();
                        Ok(Resp::Array(Some(vec![
                            Resp::BulkString(Some(b"length".to_vec())),
                            Resp::Integer(s.entries.len() as i64),
                            Resp::BulkString(Some(b"first-entry".to_vec())),
                            Resp::BulkString(Some(first_id.into_bytes())),
                            Resp::BulkString(Some(b"last-entry".to_vec())),
                            Resp::BulkString(Some(last_id.into_bytes())),
                            Resp::BulkString(Some(b"groups".to_vec())),
                            Resp::Integer(s.groups.len() as i64),
                        ])))
                    }
                    _ => unreachable!(),
                },
                None => Err(CacheError::generic("ERR no such key")),
            }
        }
        b"GROUPS" => {
            if args.len() < 4 {
                return Err(CacheError::wrong_arity("xinfo groups"));
            }
            match db.get_typed(&args[3], "stream")? {
                Some(e) => match &e.value {
                    Value::Stream(s) => {
                        let groups: Vec<Resp> = s
                            .groups
                            .values()
                            .map(|g| {
                                Resp::Array(Some(vec![
                                    Resp::BulkString(Some(b"name".to_vec())),
                                    Resp::BulkString(Some(g.name.clone())),
                                    Resp::BulkString(Some(b"consumers".to_vec())),
                                    Resp::Integer(g.consumers.len() as i64),
                                    Resp::BulkString(Some(b"pending".to_vec())),
                                    Resp::Integer(g.pel.len() as i64),
                                    Resp::BulkString(Some(b"last-delivered-id".to_vec())),
                                    Resp::BulkString(Some(
                                        g.last_delivered_id.to_string().into_bytes(),
                                    )),
                                ]))
                            })
                            .collect();
                        Ok(Resp::Array(Some(groups)))
                    }
                    _ => unreachable!(),
                },
                None => Err(CacheError::generic("ERR no such key")),
            }
        }
        b"CONSUMERS" => {
            if args.len() < 5 {
                return Err(CacheError::wrong_arity("xinfo consumers"));
            }
            match db.get_typed(&args[3], "stream")? {
                Some(e) => match &e.value {
                    Value::Stream(s) => {
                        let group = s
                            .groups
                            .get(args[4].as_slice())
                            .ok_or(CacheError::NoGroup)?;
                        let consumers: Vec<Resp> = group
                            .consumers
                            .values()
                            .map(|c| {
                                Resp::Array(Some(vec![
                                    Resp::BulkString(Some(b"name".to_vec())),
                                    Resp::BulkString(Some(c.name.clone())),
                                    Resp::BulkString(Some(b"pending".to_vec())),
                                    Resp::Integer(c.pel.len() as i64),
                                ]))
                            })
                            .collect();
                        Ok(Resp::Array(Some(consumers)))
                    }
                    _ => unreachable!(),
                },
                None => Err(CacheError::generic("ERR no such key")),
            }
        }
        _ => Err(CacheError::Syntax),
    }
}

// ── XGROUP ───────────────────────────────────────────────────────────────────

pub fn cmd_xgroup(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 {
        return Err(CacheError::wrong_arity("xgroup"));
    }
    match args[2].to_ascii_uppercase().as_slice() {
        b"CREATE" => xgroup_create(&args[2..], db),
        b"DESTROY" => xgroup_destroy(&args[2..], db),
        b"CREATECONSUMER" => xgroup_createconsumer(&args[2..], db),
        b"DELCONSUMER" => xgroup_delconsumer(&args[2..], db),
        b"SETID" => xgroup_setid(&args[2..], db),
        _ => Err(CacheError::generic(format!(
            "ERR Unknown subcommand: {:?}",
            std::str::from_utf8(&args[2])
        ))),
    }
}

fn xgroup_create(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 4 {
        return Err(CacheError::wrong_arity("xgroup create"));
    }
    let key = &args[1];
    let group_name = args[2].clone();
    let id_raw = &args[3];
    let mkstream = args.len() > 4 && args[4].to_ascii_uppercase() == b"MKSTREAM";

    if !db.exists(key) {
        if mkstream {
            db.insert(key.to_vec(), Entry::new(Value::Stream(Stream::new())));
        } else {
            return Err(CacheError::generic(
                "ERR The XGROUP subcommand requires the key to exist",
            ));
        }
    }

    let last_id = if id_raw == b"$" {
        match db.get_typed(key, "stream")? {
            Some(e) => match &e.value {
                Value::Stream(s) => s.last_id,
                _ => unreachable!(),
            },
            None => StreamId::zero(),
        }
    } else {
        StreamId::parse(id_raw).ok_or_else(|| CacheError::generic("ERR Invalid stream ID"))?
    };

    match db.get_typed_mut(key, "stream")? {
        Some(entry) => match &mut entry.value {
            Value::Stream(s) => {
                if s.groups.contains_key(group_name.as_slice()) {
                    return Err(CacheError::generic(
                        "BUSYGROUP Consumer Group name already exists",
                    ));
                }
                s.groups.insert(
                    group_name.clone(),
                    ConsumerGroup {
                        name: group_name,
                        last_delivered_id: last_id,
                        consumers: HashMap::new(),
                        pel: HashMap::new(),
                    },
                );
                Ok(Resp::ok())
            }
            _ => unreachable!(),
        },
        None => unreachable!(),
    }
}

fn xgroup_destroy(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 {
        return Err(CacheError::wrong_arity("xgroup destroy"));
    }
    match db.get_typed_mut(&args[1], "stream")? {
        Some(entry) => match &mut entry.value {
            Value::Stream(s) => {
                let removed = s.groups.remove(args[2].as_slice()).is_some();
                Ok(Resp::Integer(if removed { 1 } else { 0 }))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

fn xgroup_createconsumer(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 4 {
        return Err(CacheError::wrong_arity("xgroup createconsumer"));
    }
    let ms = now_ms();
    match db.get_typed_mut(&args[1], "stream")? {
        Some(entry) => match &mut entry.value {
            Value::Stream(s) => {
                let group = s
                    .groups
                    .get_mut(args[2].as_slice())
                    .ok_or(CacheError::NoGroup)?;
                if group.consumers.contains_key(args[3].as_slice()) {
                    return Ok(Resp::Integer(0));
                }
                group.consumers.insert(
                    args[3].clone(),
                    Consumer {
                        name: args[3].clone(),
                        seen_time: ms,
                        active_time: ms,
                        pel: vec![],
                    },
                );
                Ok(Resp::Integer(1))
            }
            _ => unreachable!(),
        },
        None => Err(CacheError::generic("ERR no such key")),
    }
}

fn xgroup_delconsumer(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 4 {
        return Err(CacheError::wrong_arity("xgroup delconsumer"));
    }
    match db.get_typed_mut(&args[1], "stream")? {
        Some(entry) => match &mut entry.value {
            Value::Stream(s) => {
                let group = s
                    .groups
                    .get_mut(args[2].as_slice())
                    .ok_or(CacheError::NoGroup)?;
                let consumer = group.consumers.remove(args[3].as_slice());
                let pel_count = consumer.as_ref().map(|c| c.pel.len()).unwrap_or(0);
                Ok(Resp::Integer(pel_count as i64))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

fn xgroup_setid(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 4 {
        return Err(CacheError::wrong_arity("xgroup setid"));
    }
    let new_id = if args[3] == b"$" {
        match db.get_typed(&args[1], "stream")? {
            Some(e) => match &e.value {
                Value::Stream(s) => s.last_id,
                _ => unreachable!(),
            },
            None => StreamId::zero(),
        }
    } else {
        StreamId::parse(&args[3]).ok_or_else(|| CacheError::generic("ERR Invalid stream ID"))?
    };

    match db.get_typed_mut(&args[1], "stream")? {
        Some(entry) => match &mut entry.value {
            Value::Stream(s) => {
                let group = s
                    .groups
                    .get_mut(args[2].as_slice())
                    .ok_or(CacheError::NoGroup)?;
                group.last_delivered_id = new_id;
                Ok(Resp::ok())
            }
            _ => unreachable!(),
        },
        None => Err(CacheError::generic("ERR no such key")),
    }
}

// ── XREADGROUP ───────────────────────────────────────────────────────────────

pub fn cmd_xreadgroup(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 7 {
        return Err(CacheError::wrong_arity("xreadgroup"));
    }
    // XREADGROUP GROUP group consumer [COUNT count] [BLOCK milliseconds] [NOACK] STREAMS key [key ...] id [id ...]
    let group = args[2].clone();
    let consumer_name = args[3].clone();

    let mut count = usize::MAX;
    let mut noack = false;
    let mut i = 4;

    while i < args.len() {
        match args[i].to_ascii_uppercase().as_slice() {
            b"COUNT" => {
                i += 1;
                count = bytes_to_i64(&args[i]).ok_or(CacheError::NotInteger)? as usize;
                i += 1;
            }
            b"BLOCK" => {
                i += 2;
            } // skip
            b"NOACK" => {
                noack = true;
                i += 1;
            }
            b"STREAMS" => {
                i += 1;
                break;
            }
            _ => {
                i += 1;
            }
        }
    }

    let streams_start = i;
    let num_streams = (args.len() - streams_start) / 2;
    if num_streams == 0 {
        return Err(CacheError::Syntax);
    }

    let keys = args[streams_start..streams_start + num_streams].to_vec();
    let ids = args[streams_start + num_streams..].to_vec();
    let ms = now_ms();

    let mut results = Vec::new();
    for (key, id_raw) in keys.iter().zip(ids.iter()) {
        let pending_only = id_raw == b">";

        match db.get_typed_mut(key, "stream")? {
            Some(entry) => match &mut entry.value {
                Value::Stream(s) => {
                    let group_data = s
                        .groups
                        .get_mut(group.as_slice())
                        .ok_or(CacheError::NoGroup)?;

                    let consumer =
                        group_data
                            .consumers
                            .entry(consumer_name.clone())
                            .or_insert(Consumer {
                                name: consumer_name.clone(),
                                seen_time: ms,
                                active_time: ms,
                                pel: vec![],
                            });
                    consumer.seen_time = ms;

                    let last_id = group_data.last_delivered_id;
                    let mut delivered = Vec::new();

                    if pending_only {
                        // Deliver new messages
                        for entry in s.entries.iter().filter(|e| e.id > last_id).take(count) {
                            delivered.push(entry.clone());
                        }
                        if let Some(last) = delivered.last() {
                            group_data.last_delivered_id = last.id;
                        }

                        if !noack {
                            let cons = group_data
                                .consumers
                                .get_mut(consumer_name.as_slice())
                                .unwrap();
                            for e in &delivered {
                                cons.pel.push(e.id);
                                group_data.pel.insert(
                                    e.id,
                                    PendingEntry {
                                        id: e.id,
                                        consumer: consumer_name.clone(),
                                        delivery_time: ms,
                                        delivery_count: 1,
                                    },
                                );
                            }
                        }
                    } else {
                        // Return pending entries for this consumer
                        let from = StreamId::parse(id_raw)
                            .ok_or_else(|| CacheError::generic("ERR Invalid stream ID"))?;
                        let cons = group_data.consumers.get(consumer_name.as_slice());
                        if let Some(cons) = cons {
                            let pending_ids: Vec<StreamId> = cons
                                .pel
                                .iter()
                                .filter(|&&id| id >= from)
                                .take(count)
                                .copied()
                                .collect();
                            for id in pending_ids {
                                if let Some(e) = s.entries.iter().find(|e| e.id == id) {
                                    delivered.push(e.clone());
                                }
                            }
                        }
                    }

                    let resp_entries: Vec<Resp> =
                        delivered.iter().map(stream_entry_to_resp).collect();
                    if !resp_entries.is_empty() {
                        results.push(Resp::Array(Some(vec![
                            Resp::BulkString(Some(key.clone())),
                            Resp::Array(Some(resp_entries)),
                        ])));
                    }
                }
                _ => unreachable!(),
            },
            None => {}
        }
    }

    Ok(Resp::Array(Some(results)))
}

// ── XACK ─────────────────────────────────────────────────────────────────────

pub fn cmd_xack(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 4 {
        return Err(CacheError::wrong_arity("xack"));
    }
    let key = &args[1];
    let group = &args[2];
    let ids: Vec<StreamId> = args[3..]
        .iter()
        .map(|id| StreamId::parse(id).ok_or_else(|| CacheError::generic("ERR Invalid stream ID")))
        .collect::<CacheResult<_>>()?;

    let mut acked = 0i64;
    match db.get_typed_mut(key, "stream")? {
        Some(entry) => match &mut entry.value {
            Value::Stream(s) => {
                if let Some(group_data) = s.groups.get_mut(group.as_slice()) {
                    for id in &ids {
                        if group_data.pel.remove(id).is_some() {
                            acked += 1;
                            // Remove from consumer PEL
                            for consumer in group_data.consumers.values_mut() {
                                consumer.pel.retain(|&pid| &pid != id);
                            }
                        }
                    }
                }
                Ok(Resp::Integer(acked))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

// ── XPENDING ─────────────────────────────────────────────────────────────────

pub fn cmd_xpending(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 {
        return Err(CacheError::wrong_arity("xpending"));
    }
    let key = &args[1];
    let group = &args[2];

    match db.get_typed(key, "stream")? {
        Some(e) => match &e.value {
            Value::Stream(s) => {
                let group_data = s.groups.get(group.as_slice()).ok_or(CacheError::NoGroup)?;

                if args.len() == 3 {
                    // Summary mode
                    let count = group_data.pel.len();
                    let min = group_data
                        .pel
                        .keys()
                        .min()
                        .map(|id| id.to_string())
                        .unwrap_or_default();
                    let max = group_data
                        .pel
                        .keys()
                        .max()
                        .map(|id| id.to_string())
                        .unwrap_or_default();

                    let mut consumer_counts = HashMap::new();
                    for pe in group_data.pel.values() {
                        *consumer_counts.entry(pe.consumer.clone()).or_insert(0u64) += 1;
                    }
                    let consumers: Vec<Resp> = consumer_counts
                        .iter()
                        .map(|(name, cnt)| {
                            Resp::Array(Some(vec![
                                Resp::BulkString(Some(name.clone())),
                                Resp::BulkString(Some(cnt.to_string().into_bytes())),
                            ]))
                        })
                        .collect();

                    Ok(Resp::Array(Some(vec![
                        Resp::Integer(count as i64),
                        Resp::BulkString(Some(min.into_bytes())),
                        Resp::BulkString(Some(max.into_bytes())),
                        Resp::Array(Some(consumers)),
                    ])))
                } else {
                    // Range mode: XPENDING key group [[IDLE min-idle-time] start end count [consumer]]
                    let mut i = 3;
                    let mut idle_filter: Option<u64> = None;
                    if i < args.len() && args[i].to_ascii_uppercase() == b"IDLE" {
                        idle_filter =
                            Some(bytes_to_i64(&args[i + 1]).ok_or(CacheError::NotInteger)? as u64);
                        i += 2;
                    }
                    let start =
                        StreamId::parse(&args[i]).ok_or_else(|| CacheError::generic("ERR"))?;
                    let end =
                        StreamId::parse(&args[i + 1]).ok_or_else(|| CacheError::generic("ERR"))?;
                    let count = bytes_to_i64(&args[i + 2]).ok_or(CacheError::NotInteger)? as usize;
                    let consumer_filter = args.get(i + 3);

                    let now = now_ms();
                    let entries: Vec<Resp> = group_data
                        .pel
                        .values()
                        .filter(|pe| pe.id >= start && pe.id <= end)
                        .filter(|pe| {
                            consumer_filter
                                .map(|c| c.as_slice() == pe.consumer.as_slice())
                                .unwrap_or(true)
                        })
                        .filter(|pe| {
                            idle_filter
                                .map(|idle| now - pe.delivery_time >= idle)
                                .unwrap_or(true)
                        })
                        .take(count)
                        .map(|pe| {
                            let idle = now - pe.delivery_time;
                            Resp::Array(Some(vec![
                                Resp::BulkString(Some(pe.id.to_string().into_bytes())),
                                Resp::BulkString(Some(pe.consumer.clone())),
                                Resp::Integer(idle as i64),
                                Resp::Integer(pe.delivery_count as i64),
                            ]))
                        })
                        .collect();
                    Ok(Resp::Array(Some(entries)))
                }
            }
            _ => unreachable!(),
        },
        None => Err(CacheError::generic("ERR no such key")),
    }
}

// ── XCLAIM ───────────────────────────────────────────────────────────────────

pub fn cmd_xclaim(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 6 {
        return Err(CacheError::wrong_arity("xclaim"));
    }
    let key = &args[1];
    let group = &args[2];
    let consumer_name = args[3].clone();
    let min_idle = bytes_to_i64(&args[4]).ok_or(CacheError::NotInteger)? as u64;
    let ids: Vec<StreamId> = args[5..]
        .iter()
        .filter(|a| a.to_ascii_uppercase() != b"JUSTID" && a.to_ascii_uppercase() != b"FORCE")
        .map(|id| StreamId::parse(id).ok_or_else(|| CacheError::generic("ERR Invalid stream ID")))
        .collect::<CacheResult<_>>()?;

    let justid = args.iter().any(|a| a.to_ascii_uppercase() == b"JUSTID");
    let ms = now_ms();

    let mut claimed = Vec::new();
    match db.get_typed_mut(key, "stream")? {
        Some(entry) => match &mut entry.value {
            Value::Stream(s) => {
                let group_data = s
                    .groups
                    .get_mut(group.as_slice())
                    .ok_or(CacheError::NoGroup)?;
                for id in &ids {
                    if let Some(pe) = group_data.pel.get_mut(id) {
                        let idle = ms - pe.delivery_time;
                        if idle >= min_idle {
                            pe.consumer = consumer_name.clone();
                            pe.delivery_time = ms;
                            pe.delivery_count += 1;
                            claimed.push(*id);
                        }
                    }
                }

                let result: Vec<Resp> = if justid {
                    claimed
                        .iter()
                        .map(|id| Resp::BulkString(Some(id.to_string().into_bytes())))
                        .collect()
                } else {
                    claimed
                        .iter()
                        .filter_map(|id| s.entries.iter().find(|e| &e.id == id))
                        .map(stream_entry_to_resp)
                        .collect()
                };
                Ok(Resp::Array(Some(result)))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Array(Some(vec![]))),
    }
}

// ── XAUTOCLAIM ───────────────────────────────────────────────────────────────

pub fn cmd_xautoclaim(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 7 {
        return Err(CacheError::wrong_arity("xautoclaim"));
    }
    let key = &args[1];
    let group = &args[2];
    let consumer_name = args[3].clone();
    let min_idle = bytes_to_i64(&args[4]).ok_or(CacheError::NotInteger)? as u64;
    let start =
        StreamId::parse(&args[5]).ok_or_else(|| CacheError::generic("ERR Invalid stream ID"))?;
    let count = if args.len() > 6 && args[6].to_ascii_uppercase() == b"COUNT" {
        bytes_to_i64(&args[7]).ok_or(CacheError::NotInteger)? as usize
    } else {
        100
    };
    let justid = args.iter().any(|a| a.to_ascii_uppercase() == b"JUSTID");
    let ms = now_ms();

    match db.get_typed_mut(key, "stream")? {
        Some(entry) => match &mut entry.value {
            Value::Stream(s) => {
                let group_data = s
                    .groups
                    .get_mut(group.as_slice())
                    .ok_or(CacheError::NoGroup)?;
                let mut claimed_ids = Vec::new();

                for pe in group_data
                    .pel
                    .values_mut()
                    .filter(|pe| pe.id >= start && ms - pe.delivery_time >= min_idle)
                    .take(count)
                {
                    pe.consumer = consumer_name.clone();
                    pe.delivery_time = ms;
                    pe.delivery_count += 1;
                    claimed_ids.push(pe.id);
                }

                let next_start = claimed_ids
                    .last()
                    .map(|id| StreamId {
                        ms: id.ms,
                        seq: id.seq + 1,
                    })
                    .unwrap_or(StreamId::zero());

                let result: Vec<Resp> = if justid {
                    claimed_ids
                        .iter()
                        .map(|id| Resp::BulkString(Some(id.to_string().into_bytes())))
                        .collect()
                } else {
                    claimed_ids
                        .iter()
                        .filter_map(|id| s.entries.iter().find(|e| &e.id == id))
                        .map(stream_entry_to_resp)
                        .collect()
                };

                Ok(Resp::Array(Some(vec![
                    Resp::BulkString(Some(next_start.to_string().into_bytes())),
                    Resp::Array(Some(result)),
                    Resp::Array(Some(vec![])), // deleted entries (we don't track these)
                ])))
            }
            _ => unreachable!(),
        },
        None => Err(CacheError::generic("ERR no such key")),
    }
}
