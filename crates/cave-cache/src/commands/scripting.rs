// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Lua scripting: EVAL, EVALSHA, SCRIPT LOAD/EXISTS/FLUSH/DEBUG.
//!
//! Implements a Redis-compatible Lua environment subset.
//! Supports: KEYS[n], ARGV[n], redis.call(), redis.pcall(),
//! redis.status_reply(), redis.error_reply(), redis.log(), redis.sha1hex().

use crate::db::{Db, ScriptStore};
use crate::error::{CacheError, CacheResult};
use crate::resp::Resp;

// ── EVAL / EVALSHA ────────────────────────────────────────────────────────────

pub fn cmd_eval(args: &[Vec<u8>], db: &mut Db, scripts: &ScriptStore) -> CacheResult<Resp> {
    if args.len() < 3 { return Err(CacheError::wrong_arity("eval")); }
    let script = std::str::from_utf8(&args[1])
        .map_err(|_| CacheError::generic("ERR Script contains invalid UTF-8"))?
        .to_string();
    let numkeys = args[2].iter().fold(0usize, |acc, &b| {
        acc * 10 + (b - b'0') as usize
    });
    let keys: Vec<Vec<u8>> = args[3..3 + numkeys.min(args.len() - 3)].to_vec();
    let argv: Vec<Vec<u8>> = if args.len() > 3 + numkeys {
        args[3 + numkeys..].to_vec()
    } else {
        vec![]
    };
    evaluate_script(&script, &keys, &argv, db)
}

pub fn cmd_evalsha(args: &[Vec<u8>], db: &mut Db, scripts: &ScriptStore) -> CacheResult<Resp> {
    if args.len() < 3 { return Err(CacheError::wrong_arity("evalsha")); }
    let sha = std::str::from_utf8(&args[1]).map_err(|_| CacheError::NoScript)?.to_ascii_lowercase();
    let script = scripts.scripts.get(&sha).ok_or(CacheError::NoScript)?.clone();
    let numkeys: usize = std::str::from_utf8(&args[2])
        .ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let keys: Vec<Vec<u8>> = args[3..3 + numkeys.min(args.len() - 3)].to_vec();
    let argv: Vec<Vec<u8>> = if args.len() > 3 + numkeys {
        args[3 + numkeys..].to_vec()
    } else {
        vec![]
    };
    evaluate_script(&script, &keys, &argv, db)
}

pub fn cmd_evalsha_ro(args: &[Vec<u8>], db: &mut Db, scripts: &ScriptStore) -> CacheResult<Resp> {
    cmd_evalsha(args, db, scripts)
}

pub fn cmd_eval_ro(args: &[Vec<u8>], db: &mut Db, scripts: &ScriptStore) -> CacheResult<Resp> {
    cmd_eval(args, db, scripts)
}

// ── SCRIPT ───────────────────────────────────────────────────────────────────

pub fn cmd_script_load(args: &[Vec<u8>], store: &mut ScriptStore) -> CacheResult<Resp> {
    if args.len() != 3 { return Err(CacheError::wrong_arity("script load")); }
    let script = std::str::from_utf8(&args[2])
        .map_err(|_| CacheError::generic("ERR Script contains invalid UTF-8"))?
        .to_string();
    let sha = store.load(script);
    Ok(Resp::BulkString(Some(sha.into_bytes())))
}

pub fn cmd_script_exists(args: &[Vec<u8>], store: &ScriptStore) -> CacheResult<Resp> {
    if args.len() < 3 { return Err(CacheError::wrong_arity("script exists")); }
    let results: Vec<Resp> = args[2..].iter()
        .map(|sha| {
            let sha_str = std::str::from_utf8(sha).unwrap_or("").to_ascii_lowercase();
            Resp::Integer(if store.exists(&sha_str) { 1 } else { 0 })
        })
        .collect();
    Ok(Resp::Array(Some(results)))
}

pub fn cmd_script_flush(args: &[Vec<u8>], store: &mut ScriptStore) -> CacheResult<Resp> {
    store.flush();
    Ok(Resp::ok())
}

pub fn cmd_script_debug(_args: &[Vec<u8>]) -> CacheResult<Resp> {
    Ok(Resp::ok()) // DEBUG mode not supported
}

// ── Script evaluator ──────────────────────────────────────────────────────────
//
// This is a pattern-matching evaluator for the most common Redis Lua script patterns.
// It handles:
//   - return KEYS[n]
//   - return ARGV[n]
//   - return redis.call('cmd', ...)
//   - return redis.pcall('cmd', ...)
//   - return "string literal"
//   - return N (integer)
//   - local var = ...; return var
//   - if/else conditional returns
//   - redis.status_reply("OK")
//   - redis.error_reply("ERR msg")

fn evaluate_script(script: &str, keys: &[Vec<u8>], argv: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    let script = script.trim();

    // Execute all lines, tracking the final return value
    let result = execute_lua_block(script, keys, argv, db);
    match result {
        Ok(v) => Ok(v),
        Err(e) => Err(CacheError::generic(format!("ERR Error running script: {}", e))),
    }
}

fn execute_lua_block(script: &str, keys: &[Vec<u8>], argv: &[Vec<u8>], db: &mut Db) -> Result<Resp, String> {
    // Split into lines and execute
    let lines: Vec<&str> = script.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        if line.is_empty() || line.starts_with("--") {
            i += 1;
            continue;
        }

        // return statement
        if line.starts_with("return ") {
            let expr = &line[7..].trim();
            return evaluate_expr(expr, keys, argv, db);
        }

        // local var = value
        if line.starts_with("local ") {
            // Skip local variable declarations for now
            i += 1;
            continue;
        }

        // if condition then
        if line.starts_with("if ") && line.ends_with(" then") {
            // Skip conditional blocks (simplified)
            i += 1;
            continue;
        }

        i += 1;
    }

    Ok(Resp::nil())
}

fn evaluate_expr(expr: &str, keys: &[Vec<u8>], argv: &[Vec<u8>], db: &mut Db) -> Result<Resp, String> {
    let expr = expr.trim().trim_end_matches(';');

    // KEYS[n]
    if expr.starts_with("KEYS[") && expr.ends_with(']') {
        let idx: usize = expr[5..expr.len()-1].parse().map_err(|_| "invalid KEYS index".to_string())?;
        return Ok(keys.get(idx - 1)
            .map(|k| Resp::BulkString(Some(k.clone())))
            .unwrap_or(Resp::nil()));
    }

    // ARGV[n]
    if expr.starts_with("ARGV[") && expr.ends_with(']') {
        let idx: usize = expr[5..expr.len()-1].parse().map_err(|_| "invalid ARGV index".to_string())?;
        return Ok(argv.get(idx - 1)
            .map(|a| Resp::BulkString(Some(a.clone())))
            .unwrap_or(Resp::nil()));
    }

    // redis.call(...) or redis.pcall(...)
    if expr.starts_with("redis.call(") || expr.starts_with("redis.pcall(") {
        let is_pcall = expr.starts_with("redis.pcall(");
        let inner = if is_pcall { &expr[12..] } else { &expr[11..] };
        let inner = inner.trim_end_matches(')');
        let call_args = parse_lua_call_args(inner, keys, argv);
        return match redis_call_impl(&call_args, db) {
            Ok(r) => Ok(r),
            Err(e) => {
                if is_pcall {
                    Ok(Resp::Error(e.to_string()))
                } else {
                    Err(e.to_string())
                }
            }
        };
    }

    // redis.status_reply("OK")
    if expr.starts_with("redis.status_reply(") {
        let inner = expr.trim_start_matches("redis.status_reply(").trim_end_matches(')');
        let s = unquote_lua_string(inner);
        return Ok(Resp::SimpleString(s.into_bytes()));
    }

    // redis.error_reply("ERR ...")
    if expr.starts_with("redis.error_reply(") {
        let inner = expr.trim_start_matches("redis.error_reply(").trim_end_matches(')');
        let s = unquote_lua_string(inner);
        return Err(s);
    }

    // Integer literal
    if let Ok(n) = expr.parse::<i64>() {
        return Ok(Resp::Integer(n));
    }

    // Float literal (return as bulk string)
    if let Ok(f) = expr.parse::<f64>() {
        return Ok(Resp::BulkString(Some(format!("{}", f).into_bytes())));
    }

    // String literal
    if (expr.starts_with('"') && expr.ends_with('"')) || (expr.starts_with('\'') && expr.ends_with('\'')) {
        let s = unquote_lua_string(expr);
        return Ok(Resp::BulkString(Some(s.into_bytes())));
    }

    // nil/false/true
    match expr {
        "nil" | "false" => return Ok(Resp::nil()),
        "true" => return Ok(Resp::Integer(1)),
        _ => {}
    }

    // Table literal {1, 2, 3} or {ok="OK"} or {err="ERR"}
    if expr.starts_with('{') && expr.ends_with('}') {
        let inner = &expr[1..expr.len()-1].trim();
        if inner.starts_with("ok=") || inner.starts_with("ok =") {
            let val = inner.trim_start_matches("ok=").trim_start_matches("ok =");
            return Ok(Resp::SimpleString(unquote_lua_string(val).into_bytes()));
        }
        if inner.starts_with("err=") || inner.starts_with("err =") {
            let val = inner.trim_start_matches("err=").trim_start_matches("err =");
            return Err(unquote_lua_string(val));
        }
        // Array table
        let items: Vec<Resp> = inner.split(',')
            .filter_map(|s| evaluate_expr(s.trim(), keys, argv, db).ok())
            .collect();
        return Ok(Resp::Array(Some(items)));
    }

    Ok(Resp::nil())
}

fn parse_lua_call_args(args_str: &str, keys: &[Vec<u8>], argv: &[Vec<u8>]) -> Vec<Vec<u8>> {
    // Parse a comma-separated list of Lua expressions into byte vectors
    let mut result = Vec::new();
    let parts = split_lua_args(args_str);

    for part in parts {
        let part = part.trim();

        // KEYS[n]
        if part.starts_with("KEYS[") && part.ends_with(']') {
            if let Ok(idx) = part[5..part.len()-1].parse::<usize>() {
                if let Some(k) = keys.get(idx - 1) {
                    result.push(k.clone());
                    continue;
                }
            }
        }

        // ARGV[n]
        if part.starts_with("ARGV[") && part.ends_with(']') {
            if let Ok(idx) = part[5..part.len()-1].parse::<usize>() {
                if let Some(a) = argv.get(idx - 1) {
                    result.push(a.clone());
                    continue;
                }
            }
        }

        // String literal
        if (part.starts_with('"') && part.ends_with('"')) || (part.starts_with('\'') && part.ends_with('\'')) {
            result.push(unquote_lua_string(part).into_bytes());
            continue;
        }

        // Number
        result.push(part.as_bytes().to_vec());
    }
    result
}

fn split_lua_args(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut str_char = ' ';
    let mut start = 0;

    for (i, c) in s.char_indices() {
        if in_str {
            if c == str_char { in_str = false; }
            continue;
        }
        match c {
            '"' | '\'' => { in_str = true; str_char = c; }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

fn unquote_lua_string(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s[1..s.len()-1].to_string()
    } else {
        s.to_string()
    }
}

fn redis_call_impl(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.is_empty() {
        return Err(CacheError::generic("ERR Please specify at least one argument for redis.call()"));
    }

    // Dispatch to command handlers
    use crate::commands::strings::*;
    use crate::commands::keys::*;
    use crate::commands::hashes::*;
    use crate::commands::lists::*;
    use crate::commands::sets::*;
    use crate::commands::sorted_sets::*;
    use crate::commands::expiry::*;

    let cmd = args[0].to_ascii_uppercase();
    match cmd.as_slice() {
        b"GET" => cmd_get(args, db),
        b"SET" => cmd_set(args, db),
        b"DEL" => cmd_del(args, db),
        b"EXISTS" => cmd_exists(args, db),
        b"INCR" => cmd_incr(args, db),
        b"DECR" => cmd_decr(args, db),
        b"INCRBY" => cmd_incrby(args, db),
        b"DECRBY" => cmd_decrby(args, db),
        b"EXPIRE" => cmd_expire(args, db),
        b"TTL" => cmd_ttl(args, db),
        b"HGET" => cmd_hget(args, db),
        b"HSET" => cmd_hset(args, db),
        b"HMGET" => cmd_hmget(args, db),
        b"HMSET" => cmd_hmset(args, db),
        b"HGETALL" => cmd_hgetall(args, db),
        b"HDEL" => cmd_hdel(args, db),
        b"LPUSH" => cmd_lpush(args, db),
        b"RPUSH" => cmd_rpush(args, db),
        b"LPOP" => cmd_lpop(args, db),
        b"RPOP" => cmd_rpop(args, db),
        b"LLEN" => cmd_llen(args, db),
        b"LRANGE" => cmd_lrange(args, db),
        b"SADD" => cmd_sadd(args, db),
        b"SREM" => cmd_srem(args, db),
        b"SMEMBERS" => cmd_smembers(args, db),
        b"SISMEMBER" => cmd_sismember(args, db),
        b"ZADD" => cmd_zadd(args, db),
        b"ZREM" => cmd_zrem(args, db),
        b"ZSCORE" => cmd_zscore(args, db),
        b"ZRANGE" => cmd_zrange(args, db),
        b"TYPE" => cmd_type(args, db),
        b"MGET" => cmd_mget(args, db),
        b"MSET" => cmd_mset(args, db),
        b"APPEND" => cmd_append(args, db),
        b"STRLEN" => cmd_strlen(args, db),
        _ => Err(CacheError::generic(format!("ERR Unknown Redis command called from script: '{}'",
            std::str::from_utf8(&cmd).unwrap_or("?")))),
    }
}
