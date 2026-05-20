// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Key expiry commands: EXPIRE, PEXPIRE, EXPIREAT, PEXPIREAT, TTL, PTTL, PERSIST, EXPIRETIME, PEXPIRETIME.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::db::Db;
use crate::error::{CacheError, CacheResult};
use crate::resp::Resp;
use crate::types::bytes_to_i64;

pub fn cmd_expire(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    expire_impl(args, db, false, false)
}

pub fn cmd_pexpire(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    expire_impl(args, db, true, false)
}

pub fn cmd_expireat(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    expire_impl(args, db, false, true)
}

pub fn cmd_pexpireat(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    expire_impl(args, db, true, true)
}

fn expire_impl(args: &[Vec<u8>], db: &mut Db, ms: bool, abs: bool) -> CacheResult<Resp> {
    let cmd = match (ms, abs) {
        (false, false) => "expire",
        (true, false) => "pexpire",
        (false, true) => "expireat",
        (true, true) => "pexpireat",
    };
    if args.len() < 3 {
        return Err(CacheError::wrong_arity(cmd));
    }

    let key = &args[1];
    let value = bytes_to_i64(&args[2]).ok_or_else(|| CacheError::InvalidExpire(cmd.into()))?;

    // Parse optional condition flags (NX, XX, GT, LT)
    let mut nx = false;
    let mut xx = false;
    let mut gt = false;
    let mut lt = false;
    for flag in &args[3..] {
        match flag.to_ascii_uppercase().as_slice() {
            b"NX" => nx = true,
            b"XX" => xx = true,
            b"GT" => gt = true,
            b"LT" => lt = true,
            _ => return Err(CacheError::Syntax),
        }
    }

    if !db.exists(key) {
        return Ok(Resp::Integer(0));
    }

    let new_expiry = if abs {
        let duration = if ms {
            Duration::from_millis(value as u64)
        } else {
            Duration::from_secs(value as u64)
        };
        let unix_now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        if duration <= unix_now {
            // Already expired — delete the key
            db.remove(key);
            return Ok(Resp::Integer(1));
        }
        Instant::now() + (duration - unix_now)
    } else {
        if value <= 0 {
            return Err(CacheError::InvalidExpire(cmd.into()));
        }
        if ms {
            Instant::now() + Duration::from_millis(value as u64)
        } else {
            Instant::now() + Duration::from_secs(value as u64)
        }
    };

    if let Some(entry) = db.get_mut(key) {
        let current_expiry = entry.expires_at;

        // NX: only set if no expiry
        if nx && current_expiry.is_some() {
            return Ok(Resp::Integer(0));
        }
        // XX: only set if has expiry
        if xx && current_expiry.is_none() {
            return Ok(Resp::Integer(0));
        }
        // GT: only set if new expiry > current
        if gt && current_expiry.map(|t| new_expiry <= t).unwrap_or(false) {
            return Ok(Resp::Integer(0));
        }
        // LT: only set if new expiry < current
        if lt && current_expiry.map(|t| new_expiry >= t).unwrap_or(false) {
            return Ok(Resp::Integer(0));
        }

        entry.expires_at = Some(new_expiry);
        Ok(Resp::Integer(1))
    } else {
        Ok(Resp::Integer(0))
    }
}

pub fn cmd_ttl(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    ttl_impl(args, db, false)
}

pub fn cmd_pttl(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    ttl_impl(args, db, true)
}

fn ttl_impl(args: &[Vec<u8>], db: &mut Db, ms: bool) -> CacheResult<Resp> {
    let cmd = if ms { "pttl" } else { "ttl" };
    if args.len() != 2 {
        return Err(CacheError::wrong_arity(cmd));
    }

    match db.get(&args[1]) {
        None => Ok(Resp::Integer(-2)), // Key does not exist
        Some(e) => match e.pttl() {
            None => Ok(Resp::Integer(-1)), // No expiry
            Some(-2) => {
                db.remove(&args[1]);
                Ok(Resp::Integer(-2))
            }
            Some(pttl) => {
                if ms {
                    Ok(Resp::Integer(pttl))
                } else {
                    Ok(Resp::Integer(pttl / 1000))
                }
            }
        },
    }
}

pub fn cmd_persist(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 2 {
        return Err(CacheError::wrong_arity("persist"));
    }
    match db.get_mut(&args[1]) {
        Some(e) => {
            if e.expires_at.is_some() {
                e.expires_at = None;
                Ok(Resp::Integer(1))
            } else {
                Ok(Resp::Integer(0))
            }
        }
        None => Ok(Resp::Integer(0)),
    }
}

pub fn cmd_expiretime(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    expiretime_impl(args, db, false)
}

pub fn cmd_pexpiretime(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    expiretime_impl(args, db, true)
}

fn expiretime_impl(args: &[Vec<u8>], db: &mut Db, ms: bool) -> CacheResult<Resp> {
    let cmd = if ms { "pexpiretime" } else { "expiretime" };
    if args.len() != 2 {
        return Err(CacheError::wrong_arity(cmd));
    }

    match db.get(&args[1]) {
        None => Ok(Resp::Integer(-2)),
        Some(e) => match e.expires_at {
            None => Ok(Resp::Integer(-1)),
            Some(t) => {
                let remaining = t
                    .checked_duration_since(Instant::now())
                    .unwrap_or(Duration::ZERO);
                let unix_now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default();
                let expire_unix = unix_now + remaining;
                if ms {
                    Ok(Resp::Integer(expire_unix.as_millis() as i64))
                } else {
                    Ok(Resp::Integer(expire_unix.as_secs() as i64))
                }
            }
        },
    }
}

// ── Background expiry task ────────────────────────────────────────────────────

pub async fn expiry_task(state: std::sync::Arc<crate::db::ServerState>) {
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        for (db_idx, db_lock) in state.dbs.iter().enumerate() {
            let expired_keys = {
                let mut db = db_lock.write().await;
                db.expire_cycle()
            };

            for key in &expired_keys {
                state.notify(db_idx, "expired", key);
            }
        }
    }
}
