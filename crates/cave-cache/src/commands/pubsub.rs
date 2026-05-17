// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pub/Sub command helpers (SUBSCRIBE, UNSUBSCRIBE, PUBLISH, PSUBSCRIBE, PUNSUBSCRIBE, SSUBSCRIBE).
//!
//! The actual subscription management happens in the connection handler (server.rs).
//! These functions produce the correct response formats.

use crate::db::PubSubRegistry;
use crate::error::{CacheError, CacheResult};
use crate::resp::Resp;

/// Build the response for SUBSCRIBE confirmation.
pub fn subscribe_response(channel: &[u8], count: usize) -> Resp {
    Resp::Array(Some(vec![
        Resp::BulkString(Some(b"subscribe".to_vec())),
        Resp::BulkString(Some(channel.to_vec())),
        Resp::Integer(count as i64),
    ]))
}

/// Build the response for UNSUBSCRIBE confirmation.
pub fn unsubscribe_response(channel: Option<&[u8]>, count: usize) -> Resp {
    Resp::Array(Some(vec![
        Resp::BulkString(Some(b"unsubscribe".to_vec())),
        channel.map(|c| Resp::BulkString(Some(c.to_vec()))).unwrap_or(Resp::nil()),
        Resp::Integer(count as i64),
    ]))
}

/// Build the response for PSUBSCRIBE confirmation.
pub fn psubscribe_response(pattern: &[u8], count: usize) -> Resp {
    Resp::Array(Some(vec![
        Resp::BulkString(Some(b"psubscribe".to_vec())),
        Resp::BulkString(Some(pattern.to_vec())),
        Resp::Integer(count as i64),
    ]))
}

/// Build the response for PUNSUBSCRIBE confirmation.
pub fn punsubscribe_response(pattern: Option<&[u8]>, count: usize) -> Resp {
    Resp::Array(Some(vec![
        Resp::BulkString(Some(b"punsubscribe".to_vec())),
        pattern.map(|p| Resp::BulkString(Some(p.to_vec()))).unwrap_or(Resp::nil()),
        Resp::Integer(count as i64),
    ]))
}

/// Build the response for SSUBSCRIBE (shard subscribe) confirmation.
pub fn ssubscribe_response(channel: &[u8], count: usize) -> Resp {
    Resp::Array(Some(vec![
        Resp::BulkString(Some(b"ssubscribe".to_vec())),
        Resp::BulkString(Some(channel.to_vec())),
        Resp::Integer(count as i64),
    ]))
}

/// Encode a received pub/sub message.
pub fn message_resp(channel: &[u8], message: &[u8]) -> Resp {
    Resp::Push(vec![
        Resp::BulkString(Some(b"message".to_vec())),
        Resp::BulkString(Some(channel.to_vec())),
        Resp::BulkString(Some(message.to_vec())),
    ])
}

/// Encode a received pattern pub/sub message.
pub fn pmessage_resp(pattern: &[u8], channel: &[u8], message: &[u8]) -> Resp {
    Resp::Push(vec![
        Resp::BulkString(Some(b"pmessage".to_vec())),
        Resp::BulkString(Some(pattern.to_vec())),
        Resp::BulkString(Some(channel.to_vec())),
        Resp::BulkString(Some(message.to_vec())),
    ])
}

/// PUBLISH — returns number of subscribers that received the message.
pub fn cmd_publish(args: &[Vec<u8>], pubsub: &PubSubRegistry) -> CacheResult<Resp> {
    if args.len() != 3 { return Err(CacheError::wrong_arity("publish")); }
    let count = pubsub.publish(&args[1], &args[2]);
    Ok(Resp::Integer(count as i64))
}

/// PUBSUB CHANNELS [pattern]
pub fn cmd_pubsub_channels(args: &[Vec<u8>], pubsub: &PubSubRegistry) -> CacheResult<Resp> {
    let pattern = args.get(3).map(|p| p.as_slice());
    let channels: Vec<Resp> = pubsub.active_channels()
        .into_iter()
        .filter(|ch| pattern.map(|p| crate::db::glob_match(p, ch)).unwrap_or(true))
        .map(|ch| Resp::BulkString(Some(ch)))
        .collect();
    Ok(Resp::Array(Some(channels)))
}

/// PUBSUB NUMSUB [channel ...]
pub fn cmd_pubsub_numsub(args: &[Vec<u8>], pubsub: &PubSubRegistry) -> CacheResult<Resp> {
    let channels: Vec<Vec<u8>> = args[3..].to_vec();
    let counts = pubsub.numsub(&channels);
    let resp: Vec<Resp> = counts.into_iter().flat_map(|(ch, cnt)| {
        vec![Resp::BulkString(Some(ch)), Resp::Integer(cnt as i64)]
    }).collect();
    Ok(Resp::Array(Some(resp)))
}

/// PUBSUB NUMPAT
pub fn cmd_pubsub_numpat(pubsub: &PubSubRegistry) -> CacheResult<Resp> {
    Ok(Resp::Integer(pubsub.patterns.len() as i64))
}

/// PUBSUB SHARDCHANNELS [pattern]
pub fn cmd_pubsub_shardchannels(args: &[Vec<u8>], pubsub: &PubSubRegistry) -> CacheResult<Resp> {
    let pattern = args.get(3).map(|p| p.as_slice());
    let channels: Vec<Resp> = pubsub.shard_channels.keys()
        .filter(|ch| pattern.map(|p| crate::db::glob_match(p, ch)).unwrap_or(true))
        .map(|ch| Resp::BulkString(Some(ch.clone())))
        .collect();
    Ok(Resp::Array(Some(channels)))
}

/// PUBSUB SHARDNUMSUB [channel ...]
pub fn cmd_pubsub_shardnumsub(args: &[Vec<u8>], pubsub: &PubSubRegistry) -> CacheResult<Resp> {
    let channels = &args[3..];
    let resp: Vec<Resp> = channels.iter().flat_map(|ch| {
        let cnt = pubsub.shard_channels.get(ch.as_slice()).map(|s| s.len()).unwrap_or(0);
        vec![Resp::BulkString(Some(ch.clone())), Resp::Integer(cnt as i64)]
    }).collect();
    Ok(Resp::Array(Some(resp)))
}
