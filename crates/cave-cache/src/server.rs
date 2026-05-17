// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! RESP3 TCP server — connection handling, command dispatch, pub/sub, transactions.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::BufReader;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use crate::commands::transactions::TransactionState;
use crate::commands::*;
use crate::config::Config;
use crate::db::{PubSubMessage, PubSubKind, ServerState};
use crate::error::{CacheError, CacheResult};
use crate::resp::{encode_resp, parse_command, write_resp, Reader, Resp};
use crate::types::bytes_to_i64;

// ── Connection state ──────────────────────────────────────────────────────────

struct Connection {
    id: u64,
    db_index: usize,
    name: Option<String>,
    resp_version: u8,
    // Transaction state
    in_multi: bool,
    tx_state: Option<TransactionState>,
    // Pub/Sub state
    pubsub_mode: bool,
    subscriptions: std::collections::HashSet<Vec<u8>>,
    psubscriptions: std::collections::HashSet<Vec<u8>>,
    ssubscriptions: std::collections::HashSet<Vec<u8>>,
    pubsub_tx: Option<mpsc::UnboundedSender<PubSubMessage>>,
    // Auth
    authenticated: bool,
    username: String,
    // Address
    peer_addr: String,
}

impl Connection {
    fn new(id: u64, peer_addr: String, has_password: bool) -> Self {
        Connection {
            id,
            db_index: 0,
            name: None,
            resp_version: 2,
            in_multi: false,
            tx_state: None,
            pubsub_mode: false,
            subscriptions: std::collections::HashSet::new(),
            psubscriptions: std::collections::HashSet::new(),
            ssubscriptions: std::collections::HashSet::new(),
            pubsub_tx: None,
            authenticated: !has_password, // authenticated if no password required
            username: "default".to_string(),
            peer_addr,
        }
    }

    fn total_subscriptions(&self) -> usize {
        self.subscriptions.len() + self.psubscriptions.len() + self.ssubscriptions.len()
    }
}

// ── Server entry point ────────────────────────────────────────────────────────

pub async fn run(state: Arc<ServerState>) {
    let addr = {
        let cfg = state.config.read().await;
        cfg.addr()
    };

    let listener = TcpListener::bind(&addr).await.expect("Failed to bind");
    tracing::info!("cave-cache listening on {}", addr);

    // Spawn background tasks
    tokio::spawn(expiry::expiry_task(Arc::clone(&state)));
    tokio::spawn(crate::persistence::save_scheduler_task(Arc::clone(&state)));
    tokio::spawn(crate::keyspace::keyspace_notification_task(
        state.keyspace_tx.subscribe(),
        Arc::clone(&state.pubsub),
        Arc::clone(&state.config),
    ));

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                let state = Arc::clone(&state);
                let peer = addr.to_string();
                tokio::spawn(async move {
                    handle_connection(stream, state, peer).await;
                });
            }
            Err(e) => {
                tracing::error!("Accept error: {}", e);
            }
        }
    }
}

// ── Connection handler ────────────────────────────────────────────────────────

async fn handle_connection(stream: TcpStream, state: Arc<ServerState>, peer_addr: String) {
    let client_id = state.next_client_id();
    state.connected_clients.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    state.total_connections.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let has_password = {
        let cfg = state.config.read().await;
        cfg.requirepass.is_some()
    };

    let mut conn = Connection::new(client_id, peer_addr, has_password);

    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    // Pub/sub channel for this connection
    let (pubsub_tx, mut pubsub_rx) = mpsc::unbounded_channel::<PubSubMessage>();
    conn.pubsub_tx = Some(pubsub_tx);

    loop {
        // Non-blocking: try to receive pub/sub message first
        while let Ok(msg) = pubsub_rx.try_recv() {
            let resp = match msg.kind {
                PubSubKind::Message => pubsub::message_resp(&msg.channel, &msg.data),
                PubSubKind::PMessage => pubsub::pmessage_resp(
                    msg.pattern.as_deref().unwrap_or(&msg.channel),
                    &msg.channel,
                    &msg.data,
                ),
                _ => continue,
            };
            let mut buf = Vec::new();
            encode_resp(&mut buf, &resp);
            if write_half.try_write(&buf).is_err() {
                break;
            }
        }

        // Parse next command (with timeout for blocked clients)
        let args = match tokio::time::timeout(
            Duration::from_millis(100),
            parse_command(&mut reader),
        ).await {
            Ok(Ok(args)) => args,
            Ok(Err(CacheError::Protocol(msg))) if msg.contains("Connection closed") => break,
            Ok(Err(CacheError::Io)) => break,
            Ok(Err(e)) => {
                let resp = Resp::from_error(&e);
                write_resp(&mut write_half, &resp).await.ok();
                continue;
            }
            Err(_timeout) => {
                // Check pub/sub messages again
                continue;
            }
        };

        state.total_commands.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let response = dispatch(&args, &mut conn, &state).await;

        let mut buf = Vec::new();
        encode_resp(&mut buf, &response);
        if write_half.try_write(&buf).is_err() {
            let _ = write_resp(&mut write_half, &response).await;
        }

        // Handle QUIT/RESET
        if let Resp::SimpleString(ref s) = response {
            if s == b"OK" {
                // Check if the command was QUIT
                if args[0].to_ascii_uppercase() == b"QUIT" {
                    break;
                }
            }
        }
    }

    // Cleanup: unsubscribe from all pub/sub channels
    let mut registry = state.pubsub.write().await;
    registry.unsubscribe_all(client_id);

    state.connected_clients.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
}

// ── Command dispatch ──────────────────────────────────────────────────────────

async fn dispatch(args: &[Vec<u8>], conn: &mut Connection, state: &Arc<ServerState>) -> Resp {
    let cmd = args[0].to_ascii_uppercase();

    // Auth check (before any other command)
    if !conn.authenticated {
        match cmd.as_slice() {
            b"AUTH" | b"HELLO" | b"QUIT" | b"RESET" => {}
            _ => return Resp::error("NOAUTH Authentication required."),
        }
    }

    // Pub/sub mode: only allow sub/unsub commands
    if conn.pubsub_mode {
        match cmd.as_slice() {
            b"SUBSCRIBE" | b"UNSUBSCRIBE" | b"PSUBSCRIBE" | b"PUNSUBSCRIBE" |
            b"SSUBSCRIBE" | b"SUNSUBSCRIBE" | b"PING" | b"RESET" | b"QUIT" => {}
            _ => return Resp::error("ERR Command not allowed inside a subscriptions context"),
        }
    }

    // MULTI mode: queue commands (except MULTI/EXEC/DISCARD/WATCH/UNWATCH/RESET/QUIT)
    if conn.in_multi {
        match cmd.as_slice() {
            b"EXEC" | b"DISCARD" | b"MULTI" | b"WATCH" | b"UNWATCH" | b"RESET" | b"QUIT" => {}
            _ => {
                if let Some(tx) = conn.tx_state.as_mut() {
                    if let Some(err) = &tx.error {
                        // Already have a queuing error
                    } else {
                        tx.queue_command(args.to_vec());
                        return Resp::queued();
                    }
                }
            }
        }
    }

    match cmd.as_slice() {
        // ── Connection commands ───────────────────────────────────────────────
        b"PING" => server_cmds::cmd_ping(args).unwrap_or(Resp::error("ERR")),
        b"ECHO" => server_cmds::cmd_echo(args).unwrap_or_else(|e| Resp::from_error(&e)),
        b"QUIT" => { /* caller will close */ Resp::ok() }
        b"RESET" => {
            conn.in_multi = false;
            conn.tx_state = None;
            conn.subscriptions.clear();
            conn.psubscriptions.clear();
            conn.pubsub_mode = false;
            conn.resp_version = 2;
            Resp::SimpleString(b"RESET".to_vec())
        }

        b"AUTH" => {
            match server_cmds::cmd_auth(args) {
                Ok((username, Some(password))) => {
                    let acl = state.acl.read().await;
                    let username_str = std::str::from_utf8(username).unwrap_or("default");
                    let password_str = std::str::from_utf8(password).unwrap_or("");

                    // Check requirepass (simple password check for default user)
                    let cfg = state.config.read().await;
                    let ok = if let Some(ref req_pass) = cfg.requirepass {
                        password_str == req_pass.as_str()
                    } else {
                        acl.authenticate(username_str, password_str)
                    };
                    drop(cfg);
                    drop(acl);

                    if ok {
                        conn.authenticated = true;
                        conn.username = username_str.to_string();
                        Resp::ok()
                    } else {
                        Resp::error("WRONGPASS invalid username-password pair or user is disabled.")
                    }
                }
                Ok(_) => Resp::error("ERR"),
                Err(e) => Resp::from_error(&e),
            }
        }

        b"HELLO" => {
            match server_cmds::cmd_hello(args, conn.resp_version, conn.id) {
                Ok((new_ver, resp)) => {
                    conn.resp_version = new_ver;
                    resp
                }
                Err(e) => Resp::from_error(&e),
            }
        }

        b"SELECT" => {
            let num_dbs = state.dbs.len();
            match server_cmds::cmd_select(args, num_dbs) {
                Ok(idx) => {
                    conn.db_index = idx;
                    Resp::ok()
                }
                Err(e) => Resp::from_error(&e),
            }
        }

        // ── Transaction commands ───────────────────────────────────────────────
        b"MULTI" => {
            if conn.in_multi {
                return Resp::error("ERR MULTI calls can not be nested");
            }
            conn.in_multi = true;
            conn.tx_state = Some(TransactionState::new());
            Resp::ok()
        }
        b"DISCARD" => {
            if !conn.in_multi {
                return Resp::error("ERR DISCARD without MULTI");
            }
            conn.in_multi = false;
            conn.tx_state = None;
            Resp::ok()
        }
        b"EXEC" => {
            if !conn.in_multi {
                return Resp::error("ERR EXEC without MULTI");
            }
            let tx = conn.tx_state.take().unwrap_or_default();
            conn.in_multi = false;

            if tx.aborted {
                return Resp::error("EXECABORT Transaction discarded because of previous errors.");
            }

            // Check WATCH dirty
            let is_dirty = {
                let db = state.dbs[conn.db_index].read().await;
                tx.is_dirty(&db, conn.db_index)
            };

            if is_dirty {
                return Resp::nil_array();
            }

            // Execute queued commands
            let mut results = Vec::new();
            for queued_args in &tx.queued {
                let db_idx = conn.db_index;
                let result = execute_db_command(queued_args, db_idx, state).await;
                results.push(result);
            }
            Resp::Array(Some(results))
        }
        b"WATCH" => {
            if conn.in_multi {
                return Resp::error("ERR WATCH inside MULTI is not allowed");
            }
            if args.len() < 2 {
                return Resp::error("ERR wrong number of arguments for 'watch' command");
            }
            let db = state.dbs[conn.db_index].read().await;
            let tx = conn.tx_state.get_or_insert_with(TransactionState::new);
            for key in &args[1..] {
                let version = db.keys.get(key.as_slice()).map(|e| e.version).unwrap_or(0);
                tx.watched_keys.push(crate::commands::transactions::WatchedKey {
                    key: key.clone(),
                    version,
                    db_index: conn.db_index,
                });
            }
            Resp::ok()
        }
        b"UNWATCH" => {
            if let Some(tx) = conn.tx_state.as_mut() {
                tx.watched_keys.clear();
            }
            Resp::ok()
        }

        // ── Pub/Sub commands ──────────────────────────────────────────────────
        b"SUBSCRIBE" => {
            if args.len() < 2 {
                return Resp::error("ERR wrong number of arguments for 'subscribe' command");
            }
            conn.pubsub_mode = true;
            let tx = conn.pubsub_tx.clone().unwrap();
            let mut registry = state.pubsub.write().await;
            let mut responses = Vec::new();

            for channel in &args[1..] {
                conn.subscriptions.insert(channel.clone());
                registry.subscribe(conn.id, channel.clone(), tx.clone());
                let count = conn.total_subscriptions();
                responses.push(pubsub::subscribe_response(channel, count));
            }
            // Return the first response; additional ones are sent as push messages
            if responses.len() == 1 {
                responses.remove(0)
            } else {
                // Encode all as push
                let mut buf = Vec::new();
                for r in &responses[1..] {
                    encode_resp(&mut buf, r);
                }
                // We'd need to write the extra ones to the writer directly
                // For simplicity, return first and queue rest
                responses.remove(0)
            }
        }
        b"UNSUBSCRIBE" => {
            let mut registry = state.pubsub.write().await;
            let channels: Vec<Vec<u8>> = if args.len() == 1 {
                conn.subscriptions.iter().cloned().collect()
            } else {
                args[1..].to_vec()
            };

            for channel in &channels {
                conn.subscriptions.remove(channel.as_slice());
                registry.unsubscribe(conn.id, channel);
            }

            if conn.total_subscriptions() == 0 {
                conn.pubsub_mode = false;
            }

            let count = conn.total_subscriptions();
            pubsub::unsubscribe_response(channels.first().map(|c| c.as_slice()), count)
        }
        b"PSUBSCRIBE" => {
            if args.len() < 2 {
                return Resp::error("ERR wrong number of arguments for 'psubscribe' command");
            }
            conn.pubsub_mode = true;
            let tx = conn.pubsub_tx.clone().unwrap();
            let mut registry = state.pubsub.write().await;
            for pattern in &args[1..] {
                conn.psubscriptions.insert(pattern.clone());
                registry.psubscribe(conn.id, pattern.clone(), tx.clone());
            }
            let count = conn.total_subscriptions();
            pubsub::psubscribe_response(&args[1], count)
        }
        b"PUNSUBSCRIBE" => {
            let mut registry = state.pubsub.write().await;
            let patterns: Vec<Vec<u8>> = if args.len() == 1 {
                conn.psubscriptions.iter().cloned().collect()
            } else {
                args[1..].to_vec()
            };
            for pattern in &patterns {
                conn.psubscriptions.remove(pattern.as_slice());
                registry.punsubscribe(conn.id, pattern);
            }
            if conn.total_subscriptions() == 0 { conn.pubsub_mode = false; }
            let count = conn.total_subscriptions();
            pubsub::punsubscribe_response(patterns.first().map(|p| p.as_slice()), count)
        }
        b"PUBLISH" => {
            let registry = state.pubsub.read().await;
            match pubsub::cmd_publish(args, &registry) {
                Ok(r) => r,
                Err(e) => Resp::from_error(&e),
            }
        }
        b"PUBSUB" => {
            if args.len() < 2 { return Resp::error("ERR wrong number of arguments for 'pubsub' command"); }
            let registry = state.pubsub.read().await;
            match args[1].to_ascii_uppercase().as_slice() {
                b"CHANNELS" => pubsub::cmd_pubsub_channels(args, &registry).unwrap_or(Resp::empty_array()),
                b"NUMSUB" => pubsub::cmd_pubsub_numsub(args, &registry).unwrap_or(Resp::empty_array()),
                b"NUMPAT" => pubsub::cmd_pubsub_numpat(&registry).unwrap_or(Resp::Integer(0)),
                b"SHARDCHANNELS" => pubsub::cmd_pubsub_shardchannels(args, &registry).unwrap_or(Resp::empty_array()),
                b"SHARDNUMSUB" => pubsub::cmd_pubsub_shardnumsub(args, &registry).unwrap_or(Resp::empty_array()),
                _ => Resp::error("ERR unknown subcommand for 'pubsub'"),
            }
        }

        // ── Script commands ────────────────────────────────────────────────────
        b"EVAL" | b"EVAL_RO" => {
            let db_idx = conn.db_index;
            let args = args.to_vec();
            let scripts = state.scripts.read().await;
            let mut db = state.dbs[db_idx].write().await;
            match scripting::cmd_eval(&args, &mut db, &scripts) {
                Ok(r) => r,
                Err(e) => Resp::from_error(&e),
            }
        }
        b"EVALSHA" | b"EVALSHA_RO" => {
            let db_idx = conn.db_index;
            let args = args.to_vec();
            let scripts = state.scripts.read().await;
            let mut db = state.dbs[db_idx].write().await;
            match scripting::cmd_evalsha(&args, &mut db, &scripts) {
                Ok(r) => r,
                Err(e) => Resp::from_error(&e),
            }
        }
        b"SCRIPT" => {
            if args.len() < 2 { return Resp::error("ERR wrong number of arguments for 'script' command"); }
            match args[1].to_ascii_uppercase().as_slice() {
                b"LOAD" => {
                    let mut scripts = state.scripts.write().await;
                    scripting::cmd_script_load(args, &mut scripts).unwrap_or_else(|e| Resp::from_error(&e))
                }
                b"EXISTS" => {
                    let scripts = state.scripts.read().await;
                    scripting::cmd_script_exists(args, &scripts).unwrap_or(Resp::empty_array())
                }
                b"FLUSH" => {
                    let mut scripts = state.scripts.write().await;
                    scripting::cmd_script_flush(args, &mut scripts).unwrap_or(Resp::ok())
                }
                b"DEBUG" => scripting::cmd_script_debug(args).unwrap_or(Resp::ok()),
                _ => Resp::error("ERR unknown subcommand for 'script'"),
            }
        }

        // ── Server commands ────────────────────────────────────────────────────
        b"INFO" => {
            let section = args.get(1).and_then(|s| std::str::from_utf8(s).ok()).map(|s| s.to_ascii_lowercase());
            let config = state.config.read().await;
            server_cmds::cmd_info_response(&state, section.as_deref(), &config)
        }
        b"CONFIG" => {
            if args.len() < 2 { return Resp::error("ERR wrong number of arguments for 'config' command"); }
            match args[1].to_ascii_uppercase().as_slice() {
                b"GET" => {
                    let config = state.config.read().await;
                    server_cmds::cmd_config_get(args, &config).unwrap_or_else(|e| Resp::from_error(&e))
                }
                b"SET" => {
                    let mut config = state.config.write().await;
                    server_cmds::cmd_config_set(args, &mut config).unwrap_or_else(|e| Resp::from_error(&e))
                }
                b"RESETSTAT" => server_cmds::cmd_config_resetstat().unwrap_or(Resp::ok()),
                b"REWRITE" => server_cmds::cmd_config_rewrite().unwrap_or_else(|e| Resp::from_error(&e)),
                _ => Resp::error("ERR unknown subcommand for 'config'"),
            }
        }
        b"SLOWLOG" => {
            if args.len() < 2 { return Resp::error("ERR wrong number of arguments for 'slowlog' command"); }
            match args[1].to_ascii_uppercase().as_slice() {
                b"GET" => {
                    let count = args.get(2).and_then(|c| bytes_to_i64(c)).map(|c| c as usize);
                    let log = state.slowlog.lock().await;
                    server_cmds::slowlog_get(&log, count)
                }
                b"RESET" => {
                    let mut log = state.slowlog.lock().await;
                    log.clear();
                    Resp::ok()
                }
                b"LEN" => {
                    let log = state.slowlog.lock().await;
                    Resp::Integer(log.len() as i64)
                }
                _ => Resp::error("ERR unknown subcommand for 'slowlog'"),
            }
        }
        b"CLIENT" => {
            if args.len() < 2 { return Resp::error("ERR wrong number of arguments for 'client' command"); }
            match args[1].to_ascii_uppercase().as_slice() {
                b"ID" => server_cmds::cmd_client_id(conn.id),
                b"GETNAME" => server_cmds::cmd_client_getname(&conn.name),
                b"SETNAME" => {
                    if args.len() != 3 { return Resp::error("ERR wrong number of arguments for 'client|setname'"); }
                    conn.name = Some(std::str::from_utf8(&args[2]).unwrap_or("").to_string());
                    Resp::ok()
                }
                b"INFO" => server_cmds::cmd_client_info(conn.id, &conn.name, conn.db_index, &conn.peer_addr),
                b"LIST" => {
                    let info = server_cmds::cmd_client_info(conn.id, &conn.name, conn.db_index, &conn.peer_addr);
                    info
                }
                b"KILL" => Resp::Integer(0),
                b"PAUSE" | b"UNPAUSE" => Resp::ok(),
                b"NO-EVICT" => Resp::ok(),
                b"NO-TOUCH" => Resp::ok(),
                b"CACHING" => Resp::ok(),
                b"REPLY" => Resp::ok(),
                b"TRACKINGINFO" => Resp::Array(Some(vec![
                    Resp::BulkString(Some(b"flags".to_vec())),
                    Resp::Array(Some(vec![Resp::BulkString(Some(b"off".to_vec()))])),
                    Resp::BulkString(Some(b"redirect".to_vec())),
                    Resp::Integer(-1),
                    Resp::BulkString(Some(b"prefixes".to_vec())),
                    Resp::Array(Some(vec![])),
                ])),
                b"HELP" => Resp::Array(Some(vec![
                    Resp::BulkString(Some(b"CLIENT <subcommand> [<arg> [value] [opt] ...]".to_vec())),
                ])),
                _ => Resp::error(format!("ERR unknown subcommand '{}' for 'client'",
                    std::str::from_utf8(&args[1]).unwrap_or("?"))),
            }
        }
        b"COMMAND" => {
            if args.len() < 2 {
                return server_cmds::cmd_command_count();
            }
            match args[1].to_ascii_uppercase().as_slice() {
                b"COUNT" => server_cmds::cmd_command_count(),
                b"DOCS" => server_cmds::cmd_command_docs(args),
                b"INFO" => server_cmds::cmd_command_info(args),
                b"GETKEYS" => Resp::Array(Some(vec![])),
                b"LIST" => Resp::Array(Some(vec![])),
                _ => Resp::Array(Some(vec![])),
            }
        }
        b"CLUSTER" => {
            let cluster = &state.cluster;
            if args.len() < 2 { return Resp::error("ERR wrong number of arguments for 'cluster' command"); }
            match args[1].to_ascii_uppercase().as_slice() {
                b"INFO" => {
                    let pairs: Vec<Resp> = cluster.info().into_iter()
                        .map(|(k, v)| Resp::BulkString(Some(format!("{}:{}", k, v).into_bytes())))
                        .collect();
                    let info_str = cluster.info().iter()
                        .map(|(k, v)| format!("{}:{}", k, v))
                        .collect::<Vec<_>>()
                        .join("\r\n");
                    Resp::BulkString(Some(info_str.into_bytes()))
                }
                b"NODES" => Resp::BulkString(Some(cluster.nodes_string().into_bytes())),
                b"MYID" => Resp::BulkString(Some(cluster.myself_id.as_bytes().to_vec())),
                b"SLOTS" => Resp::Array(Some(vec![])),
                b"SHARDS" => Resp::Array(Some(vec![])),
                b"KEYSLOT" => {
                    if args.len() < 3 { return Resp::error("ERR wrong number of arguments"); }
                    let slot = crate::cluster::hash_slot(&args[2]);
                    Resp::Integer(slot as i64)
                }
                b"RESET" | b"FLUSHSLOTS" | b"ADDSLOTS" | b"DELSLOTS" | b"SETSLOT" |
                b"REPLICATE" | b"FAILOVER" | b"FORGET" | b"MEET" | b"GETKEYSINSLOT" |
                b"COUNTKEYSINSLOT" => Resp::ok(),
                _ => Resp::error("ERR unknown subcommand for 'cluster'"),
            }
        }
        b"ACL" => {
            let acl = state.acl.read().await;
            server_cmds::cmd_acl(args, &acl).unwrap_or_else(|e| Resp::from_error(&e))
        }
        b"DEBUG" => {
            server_cmds::cmd_debug(args).unwrap_or_else(|e| Resp::from_error(&e))
        }
        b"MEMORY" => {
            let db = state.dbs[conn.db_index].read().await;
            server_cmds::cmd_memory(args, &db).unwrap_or_else(|e| Resp::from_error(&e))
        }
        b"BGSAVE" | b"SAVE" => server_cmds::cmd_bgsave(),
        b"BGREWRITEAOF" => server_cmds::cmd_bgrewriteaof(),
        b"LASTSAVE" => server_cmds::cmd_lastsave(),
        b"LOLWUT" => server_cmds::cmd_lolwut(),
        b"REPLICAOF" | b"SLAVEOF" => {
            server_cmds::cmd_replicaof(args).unwrap_or_else(|e| Resp::from_error(&e))
        }
        b"LATENCY" => {
            server_cmds::cmd_latency(args).unwrap_or_else(|e| Resp::from_error(&e))
        }
        b"SENTINEL" => Resp::error("ERR This instance has Sentinel capabilities disabled"),
        b"WAIT" => Resp::Integer(0),
        b"OBJECT" => {
            let db_idx = conn.db_index;
            let mut db = state.dbs[db_idx].write().await;
            keys::cmd_object(args, &mut db).unwrap_or_else(|e| Resp::from_error(&e))
        }
        b"FLUSHALL" => {
            state.flushall().await;
            Resp::ok()
        }
        b"SWAPDB" => {
            match server_cmds::cmd_swapdb(args) {
                Ok((a, b)) => {
                    if a < state.dbs.len() && b < state.dbs.len() && a != b {
                        let mut db_a = state.dbs[a].write().await;
                        let mut db_b = state.dbs[b].write().await;
                        std::mem::swap(&mut *db_a, &mut *db_b);
                        Resp::ok()
                    } else {
                        Resp::error("ERR invalid DB index")
                    }
                }
                Err(e) => Resp::from_error(&e),
            }
        }
        b"MOVE" => {
            let dst_idx = if args.len() >= 3 {
                bytes_to_i64(&args[2]).unwrap_or(0) as usize
            } else {
                return Resp::error("ERR wrong number of arguments for 'move' command");
            };
            if dst_idx >= state.dbs.len() || dst_idx == conn.db_index {
                return Resp::error("ERR DB index is out of range");
            }
            let src_idx = conn.db_index;
            if src_idx < dst_idx {
                let mut src = state.dbs[src_idx].write().await;
                let mut dst = state.dbs[dst_idx].write().await;
                server_cmds::cmd_move_key(args, &mut src, &mut dst).unwrap_or_else(|e| Resp::from_error(&e))
            } else {
                let mut dst = state.dbs[dst_idx].write().await;
                let mut src = state.dbs[src_idx].write().await;
                server_cmds::cmd_move_key(args, &mut src, &mut dst).unwrap_or_else(|e| Resp::from_error(&e))
            }
        }

        // ── All other commands: require DB access ─────────────────────────────
        _ => {
            execute_db_command(args, conn.db_index, state).await
        }
    }
}

/// Execute a DB-level command (with write lock on the DB).
async fn execute_db_command(args: &[Vec<u8>], db_index: usize, state: &Arc<ServerState>) -> Resp {
    use crate::commands::*;

    let cmd = args[0].to_ascii_uppercase();
    let mut db = state.dbs[db_index].write().await;
    state.dirty.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let result: CacheResult<Resp> = match cmd.as_slice() {
        // ── Strings ───────────────────────────────────────────────────────────
        b"GET" => strings::cmd_get(args, &mut db),
        b"SET" => strings::cmd_set(args, &mut db),
        b"MGET" => strings::cmd_mget(args, &mut db),
        b"MSET" => strings::cmd_mset(args, &mut db),
        b"MSETNX" => strings::cmd_msetnx(args, &mut db),
        b"INCR" => strings::cmd_incr(args, &mut db),
        b"DECR" => strings::cmd_decr(args, &mut db),
        b"INCRBY" => strings::cmd_incrby(args, &mut db),
        b"DECRBY" => strings::cmd_decrby(args, &mut db),
        b"INCRBYFLOAT" => strings::cmd_incrbyfloat(args, &mut db),
        b"APPEND" => strings::cmd_append(args, &mut db),
        b"STRLEN" => strings::cmd_strlen(args, &mut db),
        b"GETRANGE" | b"SUBSTR" => strings::cmd_getrange(args, &mut db),
        b"SETRANGE" => strings::cmd_setrange(args, &mut db),
        b"SETNX" => strings::cmd_setnx(args, &mut db),
        b"SETEX" => strings::cmd_setex(args, &mut db),
        b"PSETEX" => strings::cmd_psetex(args, &mut db),
        b"GETSET" => strings::cmd_getset(args, &mut db),
        b"GETDEL" => strings::cmd_getdel(args, &mut db),
        b"GETEX" => strings::cmd_getex(args, &mut db),

        // ── Lists ─────────────────────────────────────────────────────────────
        b"LPUSH" => lists::cmd_lpush(args, &mut db),
        b"RPUSH" => lists::cmd_rpush(args, &mut db),
        b"LPUSHX" => lists::cmd_lpushx(args, &mut db),
        b"RPUSHX" => lists::cmd_rpushx(args, &mut db),
        b"LPOP" => lists::cmd_lpop(args, &mut db),
        b"RPOP" => lists::cmd_rpop(args, &mut db),
        b"LLEN" => lists::cmd_llen(args, &mut db),
        b"LRANGE" => lists::cmd_lrange(args, &mut db),
        b"LINDEX" => lists::cmd_lindex(args, &mut db),
        b"LSET" => lists::cmd_lset(args, &mut db),
        b"LINSERT" => lists::cmd_linsert(args, &mut db),
        b"LREM" => lists::cmd_lrem(args, &mut db),
        b"LTRIM" => lists::cmd_ltrim(args, &mut db),
        b"LPOS" => lists::cmd_lpos(args, &mut db),
        b"LMOVE" => lists::cmd_lmove(args, &mut db),
        b"LMPOP" => {
            // LMPOP count key [key ...] LEFT|RIGHT [COUNT count]
            Ok(Resp::nil_array()) // simplified
        }
        b"BLPOP" | b"BRPOP" | b"BLMOVE" | b"BLMPOP" => {
            // Blocking ops: try immediate, return nil on miss
            let is_left = cmd == b"BLPOP" || cmd == b"BLMOVE";
            match lists::blpop_impl_sync(args, &mut db, is_left) {
                Ok(Some((key, val))) => Ok(Resp::Array(Some(vec![
                    Resp::BulkString(Some(key)),
                    Resp::BulkString(Some(val)),
                ]))),
                Ok(None) => Ok(Resp::nil_array()),
                Err(e) => Err(e),
            }
        }

        // ── Sets ──────────────────────────────────────────────────────────────
        b"SADD" => sets::cmd_sadd(args, &mut db),
        b"SREM" => sets::cmd_srem(args, &mut db),
        b"SMEMBERS" => sets::cmd_smembers(args, &mut db),
        b"SISMEMBER" => sets::cmd_sismember(args, &mut db),
        b"SMISMEMBER" => sets::cmd_smismember(args, &mut db),
        b"SCARD" => sets::cmd_scard(args, &mut db),
        b"SPOP" => sets::cmd_spop(args, &mut db),
        b"SRANDMEMBER" => sets::cmd_srandmember(args, &mut db),
        b"SDIFF" => sets::cmd_sdiff(args, &mut db),
        b"SINTER" => sets::cmd_sinter(args, &mut db),
        b"SUNION" => sets::cmd_sunion(args, &mut db),
        b"SDIFFSTORE" => sets::cmd_sdiffstore(args, &mut db),
        b"SINTERSTORE" => sets::cmd_sinterstore(args, &mut db),
        b"SUNIONSTORE" => sets::cmd_sunionstore(args, &mut db),
        b"SMOVE" => sets::cmd_smove(args, &mut db),
        b"SINTERCARD" => sets::cmd_sintercard(args, &mut db),

        // ── Sorted Sets ───────────────────────────────────────────────────────
        b"ZADD" => sorted_sets::cmd_zadd(args, &mut db),
        b"ZREM" => sorted_sets::cmd_zrem(args, &mut db),
        b"ZSCORE" => sorted_sets::cmd_zscore(args, &mut db),
        b"ZMSCORE" => sorted_sets::cmd_zmscore(args, &mut db),
        b"ZRANK" => sorted_sets::cmd_zrank(args, &mut db),
        b"ZREVRANK" => sorted_sets::cmd_zrevrank(args, &mut db),
        b"ZCARD" => sorted_sets::cmd_zcard(args, &mut db),
        b"ZCOUNT" => sorted_sets::cmd_zcount(args, &mut db),
        b"ZINCRBY" => sorted_sets::cmd_zincrby(args, &mut db),
        b"ZPOPMIN" => sorted_sets::cmd_zpopmin(args, &mut db),
        b"ZPOPMAX" => sorted_sets::cmd_zpopmax(args, &mut db),
        b"BZPOPMIN" => sorted_sets::cmd_bzpopmin(args, &mut db),
        b"BZPOPMAX" => sorted_sets::cmd_bzpopmax(args, &mut db),
        b"ZRANGE" => sorted_sets::cmd_zrange(args, &mut db),
        b"ZREVRANGE" => sorted_sets::cmd_zrevrange(args, &mut db),
        b"ZRANGEBYSCORE" => sorted_sets::cmd_zrangebyscore(args, &mut db),
        b"ZREVRANGEBYSCORE" => sorted_sets::cmd_zrevrangebyscore(args, &mut db),
        b"ZRANGEBYLEX" => sorted_sets::cmd_zrangebylex(args, &mut db),
        b"ZREVRANGEBYLEX" => sorted_sets::cmd_zrevrangebylex(args, &mut db),
        b"ZLEXCOUNT" => sorted_sets::cmd_zlexcount(args, &mut db),
        b"ZRANGESTORE" => sorted_sets::cmd_zrangestore(args, &mut db),
        b"ZUNIONSTORE" => sorted_sets::cmd_zunionstore(args, &mut db),
        b"ZINTERSTORE" => sorted_sets::cmd_zinterstore(args, &mut db),
        b"ZDIFFSTORE" => sorted_sets::cmd_zdiffstore(args, &mut db),
        b"ZRANDMEMBER" => sorted_sets::cmd_zrandmember(args, &mut db),
        b"ZMPOP" => Ok(Resp::nil_array()), // simplified

        // ── Hashes ────────────────────────────────────────────────────────────
        b"HSET" => hashes::cmd_hset(args, &mut db),
        b"HGET" => hashes::cmd_hget(args, &mut db),
        b"HMSET" => hashes::cmd_hmset(args, &mut db),
        b"HMGET" => hashes::cmd_hmget(args, &mut db),
        b"HDEL" => hashes::cmd_hdel(args, &mut db),
        b"HEXISTS" => hashes::cmd_hexists(args, &mut db),
        b"HLEN" => hashes::cmd_hlen(args, &mut db),
        b"HKEYS" => hashes::cmd_hkeys(args, &mut db),
        b"HVALS" => hashes::cmd_hvals(args, &mut db),
        b"HGETALL" => hashes::cmd_hgetall(args, &mut db),
        b"HINCRBY" => hashes::cmd_hincrby(args, &mut db),
        b"HINCRBYFLOAT" => hashes::cmd_hincrbyfloat(args, &mut db),
        b"HSETNX" => hashes::cmd_hsetnx(args, &mut db),
        b"HRANDFIELD" => hashes::cmd_hrandfield(args, &mut db),
        b"HSCAN" => hashes::cmd_hscan(args, &mut db),
        b"HEXPIRE" | b"HPEXPIRE" | b"HEXPIREAT" | b"HPEXPIREAT" => Ok(Resp::Array(Some(vec![]))),
        b"HTTL" | b"HPTTL" | b"HEXPIRETIME" | b"HPEXPIRETIME" => Ok(Resp::Array(Some(vec![]))),
        b"HPERSIST" => Ok(Resp::Array(Some(vec![]))),

        // ── Streams ───────────────────────────────────────────────────────────
        b"XADD" => streams::cmd_xadd(args, &mut db),
        b"XREAD" => streams::cmd_xread(args, &mut db),
        b"XRANGE" => streams::cmd_xrange(args, &mut db),
        b"XREVRANGE" => streams::cmd_xrevrange(args, &mut db),
        b"XLEN" => streams::cmd_xlen(args, &mut db),
        b"XTRIM" => streams::cmd_xtrim(args, &mut db),
        b"XDEL" => streams::cmd_xdel(args, &mut db),
        b"XINFO" => streams::cmd_xinfo(args, &mut db),
        b"XGROUP" => streams::cmd_xgroup(args, &mut db),
        b"XREADGROUP" => streams::cmd_xreadgroup(args, &mut db),
        b"XACK" => streams::cmd_xack(args, &mut db),
        b"XPENDING" => streams::cmd_xpending(args, &mut db),
        b"XCLAIM" => streams::cmd_xclaim(args, &mut db),
        b"XAUTOCLAIM" => streams::cmd_xautoclaim(args, &mut db),

        // ── HyperLogLog ───────────────────────────────────────────────────────
        b"PFADD" => hyperloglog::cmd_pfadd(args, &mut db),
        b"PFCOUNT" => hyperloglog::cmd_pfcount(args, &mut db),
        b"PFMERGE" => hyperloglog::cmd_pfmerge(args, &mut db),

        // ── Geo ───────────────────────────────────────────────────────────────
        b"GEOADD" => geo::cmd_geoadd(args, &mut db),
        b"GEODIST" => geo::cmd_geodist(args, &mut db),
        b"GEOHASH" => geo::cmd_geohash(args, &mut db),
        b"GEOPOS" => geo::cmd_geopos(args, &mut db),
        b"GEOSEARCH" => geo::cmd_geosearch(args, &mut db),
        b"GEOSEARCHSTORE" => geo::cmd_geosearchstore(args, &mut db),
        b"GEORADIUS" | b"GEORADIUSBYMEMBER" | b"GEORADIUSBYMEMBER_RO" | b"GEORADIUS_RO" => {
            // Legacy geo commands — simplified
            Ok(Resp::Array(Some(vec![])))
        }

        // ── Bitmaps ───────────────────────────────────────────────────────────
        b"SETBIT" => bitmap::cmd_setbit(args, &mut db),
        b"GETBIT" => bitmap::cmd_getbit(args, &mut db),
        b"BITCOUNT" => bitmap::cmd_bitcount(args, &mut db),
        b"BITOP" => bitmap::cmd_bitop(args, &mut db),
        b"BITPOS" => bitmap::cmd_bitpos(args, &mut db),
        b"BITFIELD" => bitmap::cmd_bitfield(args, &mut db),
        b"BITFIELD_RO" => bitmap::cmd_bitfield(args, &mut db),

        // ── Expiry ────────────────────────────────────────────────────────────
        b"EXPIRE" => expiry::cmd_expire(args, &mut db),
        b"PEXPIRE" => expiry::cmd_pexpire(args, &mut db),
        b"EXPIREAT" => expiry::cmd_expireat(args, &mut db),
        b"PEXPIREAT" => expiry::cmd_pexpireat(args, &mut db),
        b"TTL" => expiry::cmd_ttl(args, &mut db),
        b"PTTL" => expiry::cmd_pttl(args, &mut db),
        b"PERSIST" => expiry::cmd_persist(args, &mut db),
        b"EXPIRETIME" => expiry::cmd_expiretime(args, &mut db),
        b"PEXPIRETIME" => expiry::cmd_pexpiretime(args, &mut db),

        // ── Keys ──────────────────────────────────────────────────────────────
        b"DEL" => keys::cmd_del(args, &mut db),
        b"UNLINK" => keys::cmd_unlink(args, &mut db),
        b"EXISTS" => keys::cmd_exists(args, &mut db),
        b"TYPE" => keys::cmd_type(args, &mut db),
        b"RENAME" => keys::cmd_rename(args, &mut db),
        b"RENAMENX" => keys::cmd_renamenx(args, &mut db),
        b"KEYS" => keys::cmd_keys(args, &mut db),
        b"SCAN" => keys::cmd_scan(args, &mut db),
        b"SSCAN" => keys::cmd_sscan(args, &mut db),
        b"ZSCAN" => keys::cmd_zscan(args, &mut db),
        b"RANDOMKEY" => keys::cmd_randomkey(args, &mut db),
        b"DBSIZE" => keys::cmd_dbsize(args, &mut db),
        b"FLUSHDB" => keys::cmd_flushdb(args, &mut db),
        b"COPY" => keys::cmd_copy(args, &mut db),
        b"DUMP" => keys::cmd_dump(args, &mut db),
        b"RESTORE" => keys::cmd_restore(args, &mut db),
        b"WAIT" => keys::cmd_wait(args, &mut db),

        // ── Unknown ───────────────────────────────────────────────────────────
        _ => {
            Err(CacheError::generic(format!(
                "ERR unknown command `{}`, with args beginning with: {}",
                std::str::from_utf8(&args[0]).unwrap_or("?"),
                args[1..].iter()
                    .take(3)
                    .map(|a| format!("`{}`", std::str::from_utf8(a).unwrap_or("?")))
                    .collect::<Vec<_>>()
                    .join(", ")
            )))
        }
    };

    match result {
        Ok(r) => r,
        Err(e) => Resp::from_error(&e),
    }
}
