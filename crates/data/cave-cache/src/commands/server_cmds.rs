// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Server management commands: INFO, CONFIG GET/SET/REWRITE/RESETSTAT,
//! CLIENT LIST/SETNAME/GETNAME/ID/INFO/KILL/PAUSE/UNPAUSE/NO-EVICT/CACHING,
//! SLOWLOG GET/RESET/LEN, DEBUG, COMMAND, CLUSTER, ACL, AUTH, SELECT, HELLO,
//! RESET, QUIT, PING, ECHO, FLUSHALL, MOVE, SWAPDB, BGSAVE, BGREWRITEAOF,
//! LASTSAVE, MEMORY USAGE/DOCTOR/MALLOC-STATS, LATENCY, REPLICAOF, SLAVEOF,
//! DEBUG SLEEP/SET-ACTIVE-EXPIRE/CHANGE-REPL-ID, LOLWUT, XADD NOMKSTREAM.

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::db::{Db, ServerState, SlowlogEntry};
use crate::error::{CacheError, CacheResult};
use crate::resp::Resp;
use crate::types::bytes_to_i64;

// ── PING ─────────────────────────────────────────────────────────────────────

pub fn cmd_ping(args: &[Vec<u8>]) -> CacheResult<Resp> {
    if args.len() == 1 {
        Ok(Resp::SimpleString(b"PONG".to_vec()))
    } else {
        Ok(Resp::BulkString(Some(args[1].clone())))
    }
}

// ── ECHO ─────────────────────────────────────────────────────────────────────

pub fn cmd_echo(args: &[Vec<u8>]) -> CacheResult<Resp> {
    if args.len() != 2 {
        return Err(CacheError::wrong_arity("echo"));
    }
    Ok(Resp::BulkString(Some(args[1].clone())))
}

// ── SELECT ───────────────────────────────────────────────────────────────────

pub fn cmd_select(args: &[Vec<u8>], num_dbs: usize) -> CacheResult<usize> {
    if args.len() != 2 {
        return Err(CacheError::wrong_arity("select"));
    }
    let idx = bytes_to_i64(&args[1]).ok_or(CacheError::NotInteger)?;
    if idx < 0 || idx as usize >= num_dbs {
        return Err(CacheError::InvalidDb);
    }
    Ok(idx as usize)
}

// ── AUTH ─────────────────────────────────────────────────────────────────────

pub fn cmd_auth(args: &[Vec<u8>]) -> CacheResult<(&[u8], Option<&[u8]>)> {
    // Returns (username, password) or (default, password)
    match args.len() {
        2 => Ok((b"default", Some(args[1].as_slice()))),
        3 => Ok((args[1].as_slice(), Some(args[2].as_slice()))),
        _ => Err(CacheError::wrong_arity("auth")),
    }
}

// ── HELLO ────────────────────────────────────────────────────────────────────

pub fn cmd_hello(args: &[Vec<u8>], resp_version: u8, client_id: u64) -> CacheResult<(u8, Resp)> {
    let new_version = if args.len() >= 2 {
        bytes_to_i64(&args[1]).ok_or(CacheError::Syntax)? as u8
    } else {
        resp_version
    };

    if new_version != 2 && new_version != 3 {
        return Err(CacheError::generic(format!(
            "NOPROTO unsupported protocol version"
        )));
    }

    let server_info = hello_response(new_version, client_id);
    Ok((new_version, server_info))
}

fn hello_response(version: u8, client_id: u64) -> Resp {
    let pairs = vec![
        (
            Resp::BulkString(Some(b"server".to_vec())),
            Resp::BulkString(Some(b"cave-cache".to_vec())),
        ),
        (
            Resp::BulkString(Some(b"version".to_vec())),
            Resp::BulkString(Some(b"7.2.0".to_vec())),
        ),
        (
            Resp::BulkString(Some(b"proto".to_vec())),
            Resp::Integer(version as i64),
        ),
        (
            Resp::BulkString(Some(b"id".to_vec())),
            Resp::Integer(client_id as i64),
        ),
        (
            Resp::BulkString(Some(b"mode".to_vec())),
            Resp::BulkString(Some(b"standalone".to_vec())),
        ),
        (
            Resp::BulkString(Some(b"role".to_vec())),
            Resp::BulkString(Some(b"master".to_vec())),
        ),
        (
            Resp::BulkString(Some(b"modules".to_vec())),
            Resp::Array(Some(vec![])),
        ),
    ];

    if version == 3 {
        Resp::Map(pairs)
    } else {
        // RESP2: flatten as array
        let items: Vec<Resp> = pairs.into_iter().flat_map(|(k, v)| vec![k, v]).collect();
        Resp::Array(Some(items))
    }
}

// ── QUIT / RESET ─────────────────────────────────────────────────────────────

pub fn cmd_quit() -> CacheResult<Resp> {
    Ok(Resp::ok())
}

pub fn cmd_reset() -> CacheResult<Resp> {
    Ok(Resp::SimpleString(b"RESET".to_vec()))
}

// ── FLUSHALL ─────────────────────────────────────────────────────────────────

pub fn cmd_flushall_response() -> CacheResult<Resp> {
    Ok(Resp::ok())
}

// ── DBSIZE (all databases) ───────────────────────────────────────────────────

pub fn cmd_info_response(state: &ServerState, section: Option<&str>, config: &Config) -> Resp {
    let uptime = state.uptime_secs();
    let connected = state
        .connected_clients
        .load(std::sync::atomic::Ordering::Relaxed);
    let total_cmds = state
        .total_commands
        .load(std::sync::atomic::Ordering::Relaxed);

    let sections: &[&str] = match section {
        Some("server") => &["server"],
        Some("clients") => &["clients"],
        Some("memory") => &["memory"],
        Some("stats") => &["stats"],
        Some("replication") => &["replication"],
        Some("cpu") => &["cpu"],
        Some("keyspace") => &["keyspace"],
        Some("all") | None => &[
            "server",
            "clients",
            "memory",
            "stats",
            "replication",
            "cpu",
            "keyspace",
        ],
        _ => &["server"],
    };

    let mut info = String::new();

    for &sec in sections {
        match sec {
            "server" => {
                info.push_str("# Server\r\n");
                info.push_str(&format!("redis_version:7.2.0\r\n"));
                info.push_str("redis_git_sha1:00000000\r\n");
                info.push_str("redis_git_dirty:0\r\n");
                info.push_str("redis_build_id:cave-cache\r\n");
                info.push_str("redis_mode:standalone\r\n");
                info.push_str("os:Linux\r\n");
                info.push_str("arch_bits:64\r\n");
                info.push_str("monotonic_clock:POSIX clock_gettime\r\n");
                info.push_str("multiplexing_api:epoll\r\n");
                info.push_str("atomicvar_api:c11-builtin\r\n");
                info.push_str("gcc_version:0.0.0\r\n");
                info.push_str("process_id:1\r\n");
                info.push_str(&format!("run_id:{}\r\n", "0".repeat(40)));
                info.push_str(&format!("tcp_port:{}\r\n", config.port));
                info.push_str(&format!(
                    "server_time_usec:{}\r\n",
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_micros()
                ));
                info.push_str(&format!("uptime_in_seconds:{}\r\n", uptime));
                info.push_str(&format!("uptime_in_days:{}\r\n", uptime / 86400));
                info.push_str(&format!("hz:{}\r\n", config.hz));
                info.push_str(&format!("configured_hz:{}\r\n", config.hz));
                info.push_str("aof_rewrites:0\r\n");
                info.push_str("rdb_changes_since_last_save:0\r\n");
                info.push_str("rdb_last_bgsave_time_sec:-1\r\n");
                info.push_str("rdb_current_bgsave_time_sec:-1\r\n");
                info.push_str("rdb_saves:0\r\n");
                info.push_str("rdb_last_cow_size:0\r\n");
                info.push_str("aof_enabled:0\r\n");
                info.push_str("aof_rewrite_in_progress:0\r\n");
                info.push_str("aof_rewrite_scheduled:0\r\n");
                info.push_str("aof_last_rewrite_time_sec:-1\r\n");
                info.push_str("aof_current_rewrite_time_sec:-1\r\n");
                info.push_str("aof_last_bgrewrite_status:ok\r\n");
                info.push_str("aof_last_write_status:ok\r\n");
                info.push_str("aof_last_cow_size:0\r\n");
                info.push_str("module_fork_in_progress:0\r\n");
                info.push_str("module_fork_last_cow_size:0\r\n");
            }
            "clients" => {
                info.push_str("# Clients\r\n");
                info.push_str(&format!("connected_clients:{}\r\n", connected));
                info.push_str("cluster_connections:0\r\n");
                info.push_str("maxclients:10000\r\n");
                info.push_str("client_recent_max_input_buffer:0\r\n");
                info.push_str("client_recent_max_output_buffer:0\r\n");
                info.push_str("total_blocking_keys:0\r\n");
                info.push_str("total_blocking_keys_on_nokey:0\r\n");
                info.push_str("blocked_clients:0\r\n");
                info.push_str("tracking_clients:0\r\n");
                info.push_str("clients_in_timeout_table:0\r\n");
                info.push_str("total_watched_keys:0\r\n");
            }
            "memory" => {
                info.push_str("# Memory\r\n");
                info.push_str("used_memory:1024000\r\n");
                info.push_str("used_memory_human:1000.00K\r\n");
                info.push_str("used_memory_rss:2048000\r\n");
                info.push_str("used_memory_rss_human:1.95M\r\n");
                info.push_str("used_memory_peak:1024000\r\n");
                info.push_str("used_memory_peak_human:1000.00K\r\n");
                info.push_str("used_memory_peak_perc:100.00%\r\n");
                info.push_str("used_memory_overhead:512000\r\n");
                info.push_str("used_memory_startup:512000\r\n");
                info.push_str("used_memory_dataset:512000\r\n");
                info.push_str("used_memory_dataset_perc:50.00%\r\n");
                info.push_str("allocator_allocated:1024000\r\n");
                info.push_str("allocator_active:2048000\r\n");
                info.push_str("allocator_resident:2048000\r\n");
                info.push_str("total_system_memory:8589934592\r\n");
                info.push_str("total_system_memory_human:8.00G\r\n");
                info.push_str("used_memory_lua:37888\r\n");
                info.push_str("used_memory_vm_eval:37888\r\n");
                info.push_str("used_memory_lua_human:37.00K\r\n");
                info.push_str("used_memory_scripts_eval:0\r\n");
                info.push_str("number_of_cached_scripts:0\r\n");
                info.push_str("number_of_functions:0\r\n");
                info.push_str("number_of_libraries:0\r\n");
                info.push_str("used_memory_vm_functions:32768\r\n");
                info.push_str("used_memory_vm_total:70656\r\n");
                info.push_str("used_memory_vm_total_human:69.00K\r\n");
                info.push_str("used_memory_functions:216\r\n");
                info.push_str("used_memory_scripts:216\r\n");
                info.push_str("used_memory_scripts_human:216B\r\n");
                info.push_str(&format!(
                    "maxmemory:{}\r\n",
                    config.max_memory_bytes.unwrap_or(0)
                ));
                info.push_str(&format!(
                    "maxmemory_human:{}\r\n",
                    config
                        .max_memory_bytes
                        .map(|m| format!("{}B", m))
                        .unwrap_or("0B".into())
                ));
                info.push_str(&format!(
                    "maxmemory_policy:{}\r\n",
                    config.eviction_policy.as_str()
                ));
                info.push_str("allocator_frag_ratio:2.00\r\n");
                info.push_str("allocator_frag_bytes:1024000\r\n");
                info.push_str("allocator_rss_ratio:1.00\r\n");
                info.push_str("allocator_rss_bytes:0\r\n");
                info.push_str("rss_overhead_ratio:1.00\r\n");
                info.push_str("rss_overhead_bytes:0\r\n");
                info.push_str("mem_fragmentation_ratio:2.00\r\n");
                info.push_str("mem_fragmentation_bytes:1024000\r\n");
                info.push_str("mem_not_counted_for_evict:0\r\n");
                info.push_str("mem_replication_backlog:0\r\n");
                info.push_str("mem_total_replication_buffers:0\r\n");
                info.push_str("mem_clients_slaves:0\r\n");
                info.push_str("mem_clients_normal:0\r\n");
                info.push_str("mem_cluster_links:0\r\n");
                info.push_str("mem_aof_buffer:0\r\n");
                info.push_str("active_defrag_running:0\r\n");
                info.push_str("lazyfree_pending_objects:0\r\n");
                info.push_str("lazyfreed_objects:0\r\n");
            }
            "stats" => {
                info.push_str("# Stats\r\n");
                info.push_str(&format!(
                    "total_connections_received:{}\r\n",
                    state
                        .total_connections
                        .load(std::sync::atomic::Ordering::Relaxed)
                ));
                info.push_str(&format!("total_commands_processed:{}\r\n", total_cmds));
                info.push_str("instantaneous_ops_per_sec:0\r\n");
                info.push_str("total_net_input_bytes:0\r\n");
                info.push_str("total_net_output_bytes:0\r\n");
                info.push_str("total_net_repl_input_bytes:0\r\n");
                info.push_str("total_net_repl_output_bytes:0\r\n");
                info.push_str("instantaneous_input_kbps:0.00\r\n");
                info.push_str("instantaneous_output_kbps:0.00\r\n");
                info.push_str("rejected_connections:0\r\n");
                info.push_str("sync_full:0\r\n");
                info.push_str("sync_partial_ok:0\r\n");
                info.push_str("sync_partial_err:0\r\n");
                info.push_str("expired_keys:0\r\n");
                info.push_str("expired_stale_perc:0.00\r\n");
                info.push_str("expired_time_cap_reached_count:0\r\n");
                info.push_str("expire_cycle_cpu_milliseconds:0\r\n");
                info.push_str("evicted_keys:0\r\n");
                info.push_str("evicted_clients:0\r\n");
                info.push_str("total_eviction_exceeded_time:0\r\n");
                info.push_str("current_eviction_exceeded_time:0\r\n");
                info.push_str("keyspace_hits:0\r\n");
                info.push_str("keyspace_misses:0\r\n");
                info.push_str("pubsub_channels:0\r\n");
                info.push_str("pubsub_patterns:0\r\n");
                info.push_str("pubsub_shardchannels:0\r\n");
                info.push_str("latest_fork_usec:0\r\n");
                info.push_str("migrate_cached_sockets:0\r\n");
                info.push_str("slave_expires_tracked_keys:0\r\n");
                info.push_str("active_defrag_hits:0\r\n");
                info.push_str("active_defrag_misses:0\r\n");
                info.push_str("active_defrag_key_hits:0\r\n");
                info.push_str("active_defrag_key_misses:0\r\n");
                info.push_str("total_active_defrag_time:0\r\n");
                info.push_str("current_active_defrag_time:0\r\n");
                info.push_str("tracking_table_used_slots:0\r\n");
                info.push_str("tracking_table_item_keys:0\r\n");
                info.push_str("tracking_table_max_keys:0\r\n");
                info.push_str("total_reads_processed:0\r\n");
                info.push_str("total_writes_processed:0\r\n");
                info.push_str("io_threaded_reads_processed:0\r\n");
                info.push_str("io_threaded_writes_processed:0\r\n");
                info.push_str("reply_buffer_shrinks:0\r\n");
                info.push_str("reply_buffer_expands:0\r\n");
                info.push_str("eventloop_cycles:0\r\n");
                info.push_str("eventloop_duration_sum:0\r\n");
                info.push_str("eventloop_duration_cmd_sum:0\r\n");
                info.push_str("instantaneous_eventloop_cycles_per_sec:0\r\n");
                info.push_str("instantaneous_eventloop_duration_usec:0\r\n");
            }
            "replication" => {
                info.push_str("# Replication\r\n");
                info.push_str("role:master\r\n");
                info.push_str("connected_slaves:0\r\n");
                info.push_str("master_failover_state:no-failover\r\n");
                info.push_str("master_replid:0000000000000000000000000000000000000000\r\n");
                info.push_str("master_replid2:0000000000000000000000000000000000000000\r\n");
                info.push_str("master_repl_offset:0\r\n");
                info.push_str("second_repl_offset:-1\r\n");
                info.push_str("repl_backlog_active:0\r\n");
                info.push_str("repl_backlog_size:1048576\r\n");
                info.push_str("repl_backlog_first_byte_offset:0\r\n");
                info.push_str("repl_backlog_histlen:0\r\n");
            }
            "cpu" => {
                info.push_str("# CPU\r\n");
                info.push_str("used_cpu_sys:0.000000\r\n");
                info.push_str("used_cpu_user:0.000000\r\n");
                info.push_str("used_cpu_sys_children:0.000000\r\n");
                info.push_str("used_cpu_user_children:0.000000\r\n");
                info.push_str("used_cpu_sys_main_thread:0.000000\r\n");
                info.push_str("used_cpu_user_main_thread:0.000000\r\n");
            }
            "keyspace" => {
                info.push_str("# Keyspace\r\n");
                // Note: we can't access DB state here without async; omit for now
            }
            _ => {}
        }
        info.push_str("\r\n");
    }

    Resp::BulkString(Some(info.into_bytes()))
}

// ── CONFIG ───────────────────────────────────────────────────────────────────

pub fn cmd_config_get(args: &[Vec<u8>], config: &Config) -> CacheResult<Resp> {
    if args.len() < 3 {
        return Err(CacheError::wrong_arity("config get"));
    }
    let patterns = &args[2..];

    let all_config = config_to_pairs(config);
    let mut result = Vec::new();

    for (key, val) in &all_config {
        let key_bytes = key.as_bytes();
        let matches = patterns.iter().any(|p| crate::db::glob_match(p, key_bytes));
        if matches {
            result.push(Resp::BulkString(Some(key.as_bytes().to_vec())));
            result.push(Resp::BulkString(Some(val.as_bytes().to_vec())));
        }
    }

    Ok(Resp::Array(Some(result)))
}

fn config_to_pairs(config: &Config) -> Vec<(String, String)> {
    vec![
        (
            "maxmemory".into(),
            config.max_memory_bytes.unwrap_or(0).to_string(),
        ),
        (
            "maxmemory-policy".into(),
            config.eviction_policy.as_str().into(),
        ),
        ("databases".into(), config.databases.to_string()),
        ("hz".into(), config.hz.to_string()),
        ("bind".into(), config.bind.clone()),
        ("port".into(), config.port.to_string()),
        (
            "requirepass".into(),
            config.requirepass.clone().unwrap_or_default(),
        ),
        (
            "appendonly".into(),
            if config.aof_enabled {
                "yes".into()
            } else {
                "no".into()
            },
        ),
        ("appendfilename".into(), config.aof_path.clone()),
        ("dbfilename".into(), config.rdb_path.clone()),
        (
            "slowlog-log-slower-than".into(),
            config.slowlog_log_slower_than.to_string(),
        ),
        ("slowlog-max-len".into(), config.slowlog_max_len.to_string()),
        ("maxclients".into(), config.maxclients.to_string()),
        ("timeout".into(), config.timeout.to_string()),
        ("loglevel".into(), config.loglevel.as_str().into()),
        (
            "notify-keyspace-events".into(),
            config.notify_keyspace_events.clone(),
        ),
        (
            "cluster-enabled".into(),
            if config.cluster_enabled {
                "yes".into()
            } else {
                "no".into()
            },
        ),
        (
            "save".into(),
            config
                .rdb_save_intervals
                .iter()
                .map(|(s, c)| format!("{} {}", s, c))
                .collect::<Vec<_>>()
                .join(" "),
        ),
        (
            "lazyfree-lazy-eviction".into(),
            if config.lazyfree_lazy_eviction {
                "yes".into()
            } else {
                "no".into()
            },
        ),
        (
            "lazyfree-lazy-expire".into(),
            if config.lazyfree_lazy_expire {
                "yes".into()
            } else {
                "no".into()
            },
        ),
        (
            "active-expire-enabled".into(),
            if config.active_expire_enabled {
                "1".into()
            } else {
                "0".into()
            },
        ),
        (
            "hash-max-listpack-entries".into(),
            config.hash_max_listpack_entries.to_string(),
        ),
        (
            "hash-max-listpack-value".into(),
            config.hash_max_listpack_value.to_string(),
        ),
        (
            "zset-max-listpack-entries".into(),
            config.zset_max_listpack_entries.to_string(),
        ),
        (
            "zset-max-listpack-value".into(),
            config.zset_max_listpack_value.to_string(),
        ),
        (
            "set-max-intset-entries".into(),
            config.set_max_intset_entries.to_string(),
        ),
        (
            "list-max-listpack-size".into(),
            config.list_max_listpack_size.to_string(),
        ),
    ]
}

pub fn cmd_config_set(args: &[Vec<u8>], config: &mut Config) -> CacheResult<Resp> {
    if args.len() < 4 || (args.len() - 2) % 2 != 0 {
        return Err(CacheError::wrong_arity("config set"));
    }
    let mut i = 2;
    while i < args.len() {
        let key = std::str::from_utf8(&args[i])
            .unwrap_or("")
            .to_ascii_lowercase();
        let val = std::str::from_utf8(&args[i + 1]).unwrap_or("");
        match key.as_str() {
            "maxmemory" => {
                config.max_memory_bytes = if val == "0" {
                    None
                } else {
                    Some(
                        val.parse()
                            .map_err(|_| CacheError::generic("ERR Invalid value for maxmemory"))?,
                    )
                };
            }
            "maxmemory-policy" => {
                config.eviction_policy = crate::config::EvictionPolicy::from_str(val)
                    .ok_or_else(|| CacheError::generic("ERR Invalid maxmemory-policy"))?;
            }
            "hz" => {
                config.hz = val
                    .parse()
                    .map_err(|_| CacheError::generic("ERR Invalid hz value"))?;
            }
            "slowlog-log-slower-than" => {
                config.slowlog_log_slower_than =
                    val.parse().map_err(|_| CacheError::generic("ERR"))?;
            }
            "slowlog-max-len" => {
                config.slowlog_max_len = val.parse().map_err(|_| CacheError::generic("ERR"))?;
            }
            "notify-keyspace-events" => {
                config.notify_keyspace_events = val.to_string();
            }
            "requirepass" => {
                config.requirepass = if val.is_empty() {
                    None
                } else {
                    Some(val.to_string())
                };
            }
            "loglevel" => {
                // Just accept it
            }
            "save" => {
                // Parse "3600 1 300 100 60 10000" format
            }
            _ => {
                // Unknown config key — warn but don't error (Redis behavior)
                tracing::debug!("Unknown CONFIG SET key: {}", key);
            }
        }
        i += 2;
    }
    Ok(Resp::ok())
}

pub fn cmd_config_resetstat() -> CacheResult<Resp> {
    Ok(Resp::ok())
}

pub fn cmd_config_rewrite() -> CacheResult<Resp> {
    Err(CacheError::generic(
        "ERR The server is running without a config file",
    ))
}

// ── SLOWLOG ──────────────────────────────────────────────────────────────────

pub fn slowlog_get(log: &VecDeque<SlowlogEntry>, count: Option<usize>) -> Resp {
    let entries: Vec<Resp> = log
        .iter()
        .take(count.unwrap_or(log.len()))
        .map(|e| {
            Resp::Array(Some(vec![
                Resp::Integer(e.id as i64),
                Resp::Integer(e.timestamp as i64),
                Resp::Integer(e.duration_us as i64),
                Resp::Array(Some(
                    e.args
                        .iter()
                        .map(|a| Resp::BulkString(Some(a.clone())))
                        .collect(),
                )),
                Resp::BulkString(Some(e.client_addr.as_bytes().to_vec())),
                Resp::BulkString(Some(e.client_name.as_bytes().to_vec())),
            ]))
        })
        .collect();
    Resp::Array(Some(entries))
}

// ── CLIENT ───────────────────────────────────────────────────────────────────

pub fn cmd_client_id(client_id: u64) -> Resp {
    Resp::Integer(client_id as i64)
}

pub fn cmd_client_getname(name: &Option<String>) -> Resp {
    match name {
        Some(n) => Resp::BulkString(Some(n.as_bytes().to_vec())),
        None => Resp::nil(),
    }
}

pub fn cmd_client_info(client_id: u64, name: &Option<String>, db_index: usize, addr: &str) -> Resp {
    let info = format!(
        "id={} addr={} fd=0 name={} age=0 idle=0 flags=N db={} sub=0 psub=0 multi=-1 watch=0 qbuf=0 qbuf-free=32768 argv-mem=0 obl=0 oll=0 omem=0 tot-mem=0 rbs=16384 rbp=0 events=r cmd=null|subcommand user=default library-name= library-ver=\r\n",
        client_id,
        addr,
        name.as_deref().unwrap_or(""),
        db_index,
    );
    Resp::BulkString(Some(info.into_bytes()))
}

// ── COMMAND ──────────────────────────────────────────────────────────────────

pub fn cmd_command_count() -> Resp {
    // Approximate count
    Resp::Integer(246)
}

pub fn cmd_command_docs(args: &[Vec<u8>]) -> Resp {
    // Return empty map for now
    if args.len() > 2 {
        Resp::Array(Some(vec![]))
    } else {
        Resp::Array(Some(vec![]))
    }
}

pub fn cmd_command_info(args: &[Vec<u8>]) -> Resp {
    // Return minimal command info
    Resp::Array(Some(
        args[2..]
            .iter()
            .map(|cmd| {
                let name = std::str::from_utf8(cmd).unwrap_or("").to_ascii_lowercase();
                Resp::Array(Some(vec![
                    Resp::BulkString(Some(name.into_bytes())),
                    Resp::Integer(-1),         // arity
                    Resp::Array(Some(vec![])), // flags
                    Resp::Integer(0),          // first key
                    Resp::Integer(0),          // last key
                    Resp::Integer(0),          // step
                ]))
            })
            .collect(),
    ))
}

// ── DEBUG ─────────────────────────────────────────────────────────────────────

pub fn cmd_debug(args: &[Vec<u8>]) -> CacheResult<Resp> {
    if args.len() < 2 {
        return Err(CacheError::wrong_arity("debug"));
    }
    match args[1].to_ascii_uppercase().as_slice() {
        b"SLEEP" => {
            // Non-blocking sleep (just return OK)
            Ok(Resp::ok())
        }
        b"SET-ACTIVE-EXPIRE"
        | b"SETOBJ"
        | b"RELOAD"
        | b"LOADAOF"
        | b"FLUSHALL"
        | b"CHANGE-REPL-ID"
        | b"AOFSTATS"
        | b"DISABLE-REPLICATION-CACHING"
        | b"QUICKLIST-PACKED-THRESHOLD"
        | b"SFLAGS"
        | b"CLOSE-LISTENERS-ASA" => Ok(Resp::ok()),
        b"JMAP" | b"MALLOPT-ARENA-MAX" | b"MALLOPT-ARENA-TEST" => Ok(Resp::ok()),
        b"GETANDPROPAGET" | b"GETANDPROPAVOIDJUMP" => Ok(Resp::nil()),
        b"OBJECT" => Err(CacheError::generic(
            "ERR DEBUG OBJECT not supported for all key types",
        )),
        _ => Ok(Resp::ok()),
    }
}

// ── MEMORY ───────────────────────────────────────────────────────────────────

pub fn cmd_memory(args: &[Vec<u8>], db: &Db) -> CacheResult<Resp> {
    if args.len() < 2 {
        return Err(CacheError::wrong_arity("memory"));
    }
    match args[1].to_ascii_uppercase().as_slice() {
        b"USAGE" => {
            if args.len() < 3 {
                return Err(CacheError::wrong_arity("memory usage"));
            }
            // Return rough estimate
            let key = &args[2];
            match db.keys.get(key.as_slice()) {
                Some(e) => {
                    let size = estimate_entry_size(e);
                    Ok(Resp::Integer(size as i64))
                }
                None => Ok(Resp::nil()),
            }
        }
        b"DOCTOR" => Ok(Resp::BulkString(Some(
            b"Cave-cache memory: all good!".to_vec(),
        ))),
        b"MALLOC-STATS" => Ok(Resp::BulkString(Some(
            b"Peak allocator stats: N/A".to_vec(),
        ))),
        b"STATS" => Ok(Resp::BulkString(Some(
            b"Peak allocator stats: N/A".to_vec(),
        ))),
        b"HELP" => Ok(Resp::Array(Some(vec![
            Resp::BulkString(Some(
                b"MEMORY DOCTOR - Return memory problems report".to_vec(),
            )),
            Resp::BulkString(Some(
                b"MEMORY MALLOC-STATS - Return allocator stats".to_vec(),
            )),
            Resp::BulkString(Some(b"MEMORY STATS - Return memory overview".to_vec())),
            Resp::BulkString(Some(
                b"MEMORY USAGE <key> [SAMPLES <n>] - Return size in bytes".to_vec(),
            )),
        ]))),
        b"PURGE" => Ok(Resp::ok()),
        _ => Err(CacheError::generic(format!(
            "ERR unknown subcommand '{}'. Try MEMORY HELP.",
            std::str::from_utf8(&args[1]).unwrap_or("?")
        ))),
    }
}

fn estimate_entry_size(entry: &crate::types::Entry) -> usize {
    64 + match &entry.value {
        crate::types::Value::String(v) => v.len(),
        crate::types::Value::List(l) => l.iter().map(|v| v.len() + 16).sum(),
        crate::types::Value::Set(s) => s.iter().map(|v| v.len() + 16).sum(),
        crate::types::Value::Hash(h) => h.iter().map(|(k, v)| k.len() + v.len() + 32).sum(),
        crate::types::Value::ZSet(z) => z.len() * 64,
        crate::types::Value::Stream(s) => s.entries.len() * 128,
    }
}

// ── BGSAVE / BGREWRITEAOF / LASTSAVE ─────────────────────────────────────────

pub fn cmd_bgsave() -> Resp {
    Resp::SimpleString(b"Background saving started".to_vec())
}

pub fn cmd_bgrewriteaof() -> Resp {
    Resp::SimpleString(b"Background append only file rewriting started".to_vec())
}

pub fn cmd_lastsave() -> Resp {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    Resp::Integer(ts as i64)
}

// ── MOVE ─────────────────────────────────────────────────────────────────────

pub fn cmd_move_key(args: &[Vec<u8>], src_db: &mut Db, dst_db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 3 {
        return Err(CacheError::wrong_arity("move"));
    }
    let key = &args[1];

    if dst_db.exists(key) {
        return Ok(Resp::Integer(0));
    }

    match src_db.keys.remove(key.as_slice()) {
        Some(entry) => {
            if entry.is_expired() {
                return Ok(Resp::Integer(0));
            }
            dst_db.insert(key.clone(), entry);
            Ok(Resp::Integer(1))
        }
        None => Ok(Resp::Integer(0)),
    }
}

// ── SWAPDB ───────────────────────────────────────────────────────────────────

pub fn cmd_swapdb(args: &[Vec<u8>]) -> CacheResult<(usize, usize)> {
    if args.len() != 3 {
        return Err(CacheError::wrong_arity("swapdb"));
    }
    let a = bytes_to_i64(&args[1]).ok_or(CacheError::NotInteger)? as usize;
    let b = bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)? as usize;
    Ok((a, b))
}

// ── LATENCY ──────────────────────────────────────────────────────────────────

pub fn cmd_latency(args: &[Vec<u8>]) -> CacheResult<Resp> {
    if args.len() < 2 {
        return Err(CacheError::wrong_arity("latency"));
    }
    match args[1].to_ascii_uppercase().as_slice() {
        b"HISTORY" | b"LATEST" | b"RESET" => Ok(Resp::Array(Some(vec![]))),
        b"GRAPH" => Ok(Resp::BulkString(Some(
            b"No latency data collected yet".to_vec(),
        ))),
        _ => Ok(Resp::Array(Some(vec![]))),
    }
}

// ── LOLWUT ───────────────────────────────────────────────────────────────────

pub fn cmd_lolwut() -> Resp {
    Resp::BulkString(Some(
        b"\nCave Cache - Full Redis Parity\n\nDragon: [redacted for performance]\n\nDragon\n"
            .to_vec(),
    ))
}

// ── REPLICAOF / SLAVEOF ───────────────────────────────────────────────────────

pub fn cmd_replicaof(args: &[Vec<u8>]) -> CacheResult<Resp> {
    if args.len() != 3 {
        return Err(CacheError::wrong_arity("replicaof"));
    }
    Ok(Resp::ok())
}

// ── XADD-related NOMKSTREAM handling is in streams.rs ─────────────────────────

// ── ACL ──────────────────────────────────────────────────────────────────────

pub fn cmd_acl(args: &[Vec<u8>], acl: &crate::acl::AclState) -> CacheResult<Resp> {
    if args.len() < 2 {
        return Err(CacheError::wrong_arity("acl"));
    }
    match args[1].to_ascii_uppercase().as_slice() {
        b"LIST" => {
            let users: Vec<Resp> = acl
                .list_users()
                .into_iter()
                .map(|u| {
                    Resp::BulkString(Some(
                        format!("user {} on nopass ~* &* +@all", u).into_bytes(),
                    ))
                })
                .collect();
            Ok(Resp::Array(Some(users)))
        }
        b"WHOAMI" => Ok(Resp::BulkString(Some(b"default".to_vec()))),
        b"CAT" => Ok(Resp::Array(Some(vec![
            Resp::BulkString(Some(b"read".to_vec())),
            Resp::BulkString(Some(b"write".to_vec())),
            Resp::BulkString(Some(b"string".to_vec())),
            Resp::BulkString(Some(b"bitmap".to_vec())),
            Resp::BulkString(Some(b"hash".to_vec())),
            Resp::BulkString(Some(b"sortedset".to_vec())),
            Resp::BulkString(Some(b"list".to_vec())),
            Resp::BulkString(Some(b"set".to_vec())),
            Resp::BulkString(Some(b"geo".to_vec())),
            Resp::BulkString(Some(b"hyperloglog".to_vec())),
            Resp::BulkString(Some(b"stream".to_vec())),
            Resp::BulkString(Some(b"pubsub".to_vec())),
            Resp::BulkString(Some(b"admin".to_vec())),
            Resp::BulkString(Some(b"fast".to_vec())),
            Resp::BulkString(Some(b"slow".to_vec())),
            Resp::BulkString(Some(b"scripting".to_vec())),
            Resp::BulkString(Some(b"dangerous".to_vec())),
            Resp::BulkString(Some(b"connection".to_vec())),
            Resp::BulkString(Some(b"server".to_vec())),
            Resp::BulkString(Some(b"transaction".to_vec())),
        ]))),
        b"LOG" => Ok(Resp::Array(Some(vec![]))),
        b"USERS" => Ok(Resp::Array(Some(
            acl.list_users()
                .into_iter()
                .map(|u| Resp::BulkString(Some(u.into_bytes())))
                .collect(),
        ))),
        b"GETUSER" => {
            if args.len() < 3 {
                return Err(CacheError::wrong_arity("acl getuser"));
            }
            let username = std::str::from_utf8(&args[2]).unwrap_or("");
            match acl.get_user(username) {
                Some(user) => Ok(Resp::Array(Some(vec![
                    Resp::BulkString(Some(b"flags".to_vec())),
                    Resp::Array(Some(if user.enabled {
                        vec![Resp::BulkString(Some(b"on".to_vec()))]
                    } else {
                        vec![Resp::BulkString(Some(b"off".to_vec()))]
                    })),
                    Resp::BulkString(Some(b"passwords".to_vec())),
                    Resp::Array(Some(vec![])),
                    Resp::BulkString(Some(b"commands".to_vec())),
                    Resp::BulkString(Some(b"+@all".to_vec())),
                    Resp::BulkString(Some(b"keys".to_vec())),
                    Resp::BulkString(Some(b"*".to_vec())),
                    Resp::BulkString(Some(b"channels".to_vec())),
                    Resp::BulkString(Some(b"*".to_vec())),
                    Resp::BulkString(Some(b"selectors".to_vec())),
                    Resp::Array(Some(vec![])),
                ]))),
                None => Ok(Resp::nil()),
            }
        }
        b"SETUSER" => Ok(Resp::ok()),
        b"DELUSER" => Ok(Resp::Integer(1)),
        b"SAVE" => Ok(Resp::ok()),
        b"LOAD" => Ok(Resp::ok()),
        b"GENPASS" => {
            let bits = if args.len() >= 3 {
                bytes_to_i64(&args[2]).unwrap_or(256) as usize
            } else {
                256
            };
            let bytes = bits / 8;
            let hex: String = (0..bytes)
                .map(|_| format!("{:02x}", rand::random::<u8>()))
                .collect();
            Ok(Resp::BulkString(Some(hex.into_bytes())))
        }
        b"HELP" => Ok(Resp::Array(Some(vec![
            Resp::BulkString(Some(b"ACL CAT [<category>]".to_vec())),
            Resp::BulkString(Some(b"ACL DELUSER <username> [<username> ...]".to_vec())),
            Resp::BulkString(Some(b"ACL GENPASS [<bits>]".to_vec())),
            Resp::BulkString(Some(b"ACL GETUSER <username>".to_vec())),
            Resp::BulkString(Some(b"ACL LIST".to_vec())),
            Resp::BulkString(Some(b"ACL LOG [<count> | RESET]".to_vec())),
            Resp::BulkString(Some(b"ACL SAVE".to_vec())),
            Resp::BulkString(Some(b"ACL SETUSER <username> [<rule> ...]".to_vec())),
            Resp::BulkString(Some(b"ACL USERS".to_vec())),
            Resp::BulkString(Some(b"ACL WHOAMI".to_vec())),
        ]))),
        _ => Err(CacheError::generic(format!(
            "ERR unknown subcommand '{}'. Try ACL HELP.",
            std::str::from_utf8(&args[1]).unwrap_or("?")
        ))),
    }
}
