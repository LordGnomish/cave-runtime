//! Request pipelining + reply correlation.
//!
//! Mirrors the pipelining behavior of `redis-rs::pipeline::Pipeline`:
//!
//! 1. Caller calls [`Pipeline::push`] N times.
//! 2. [`Pipeline::flush_into`] writes all N RESP requests to a single buffer.
//! 3. The transport sends the buffer as one TCP write.
//! 4. The server replies with N responses in order.
//! 5. [`Pipeline::correlate`] matches the responses 1:1 with the requests.
//!
//! No tokio coupling — the entire pipeline lives on `BytesMut`.

use bytes::BytesMut;

use super::codec::{RespCodecError, RespValue, decode, encode};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PipelineError {
    #[error("decode error: {0}")]
    Decode(#[from] RespCodecError),
    #[error("expected {expected} replies but only got {got}")]
    ResponseCount { expected: usize, got: usize },
    #[error("nothing pending")]
    Empty,
}

#[derive(Debug, Default)]
pub struct Pipeline {
    pending: Vec<RespValue>,
}

impl Pipeline {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, request: RespValue) {
        self.pending.push(request);
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Encode all pending requests into `buf`, leaving the pipeline with the
    /// same number of pending entries (so the caller can call [`correlate`]
    /// after the server replies).
    pub fn flush_into(&self, buf: &mut BytesMut) -> Result<(), PipelineError> {
        if self.pending.is_empty() {
            return Err(PipelineError::Empty);
        }
        for req in &self.pending {
            encode(req, buf);
        }
        Ok(())
    }

    /// Decode `expected = self.len()` replies from `buf` and return them in
    /// order. The internal pending list is cleared on success.
    pub fn correlate(&mut self, buf: &mut BytesMut) -> Result<Vec<RespValue>, PipelineError> {
        let expected = self.pending.len();
        let mut replies = Vec::with_capacity(expected);
        for i in 0..expected {
            match decode(buf) {
                Ok(v) => replies.push(v),
                Err(_) => {
                    return Err(PipelineError::ResponseCount {
                        expected,
                        got: i,
                    });
                }
            }
        }
        self.pending.clear();
        Ok(replies)
    }

    pub fn cancel(&mut self) {
        self.pending.clear();
    }
}

/// Build a Redis command request: array of bulk strings.
pub fn command(parts: &[&[u8]]) -> RespValue {
    RespValue::array(
        parts
            .iter()
            .map(|p| RespValue::bulk(p.to_vec()))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(parts: &[&[u8]]) -> RespValue {
        command(parts)
    }

    #[test]
    fn empty_pipeline_flush_errors() {
        let p = Pipeline::new();
        let mut buf = BytesMut::new();
        let err = p.flush_into(&mut buf).unwrap_err();
        assert_eq!(err, PipelineError::Empty);
    }

    #[test]
    fn single_request_correlates_one_reply() {
        let mut p = Pipeline::new();
        p.push(cmd(&[b"PING"]));
        let mut req_buf = BytesMut::new();
        p.flush_into(&mut req_buf).unwrap();
        // Simulate server reply.
        let mut reply_buf = BytesMut::new();
        encode(&RespValue::pong(), &mut reply_buf);
        let replies = p.correlate(&mut reply_buf).unwrap();
        assert_eq!(replies, vec![RespValue::pong()]);
        assert!(p.is_empty());
    }

    #[test]
    fn three_requests_correlate_three_replies_in_order() {
        let mut p = Pipeline::new();
        p.push(cmd(&[b"SET", b"k", b"v"]));
        p.push(cmd(&[b"GET", b"k"]));
        p.push(cmd(&[b"DEL", b"k"]));
        let mut req_buf = BytesMut::new();
        p.flush_into(&mut req_buf).unwrap();
        let mut reply_buf = BytesMut::new();
        encode(&RespValue::ok(), &mut reply_buf);
        encode(&RespValue::bulk(b"v".to_vec()), &mut reply_buf);
        encode(&RespValue::Integer(1), &mut reply_buf);
        let replies = p.correlate(&mut reply_buf).unwrap();
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0], RespValue::ok());
        assert_eq!(replies[1], RespValue::bulk(b"v".to_vec()));
        assert_eq!(replies[2], RespValue::Integer(1));
    }

    #[test]
    fn missing_reply_errors_with_count() {
        let mut p = Pipeline::new();
        p.push(cmd(&[b"GET", b"a"]));
        p.push(cmd(&[b"GET", b"b"]));
        let mut buf = BytesMut::new();
        encode(&RespValue::ok(), &mut buf);
        let err = p.correlate(&mut buf).unwrap_err();
        assert_eq!(
            err,
            PipelineError::ResponseCount {
                expected: 2,
                got: 1
            }
        );
    }

    #[test]
    fn cancel_clears_pending() {
        let mut p = Pipeline::new();
        p.push(cmd(&[b"GET", b"x"]));
        assert_eq!(p.len(), 1);
        p.cancel();
        assert!(p.is_empty());
    }

    #[test]
    fn flush_layout_back_to_back() {
        let mut p = Pipeline::new();
        p.push(cmd(&[b"PING"]));
        p.push(cmd(&[b"PING"]));
        let mut buf = BytesMut::new();
        p.flush_into(&mut buf).unwrap();
        // Each PING command is "*1\r\n$4\r\nPING\r\n" = 14 bytes.
        assert_eq!(&buf[..], b"*1\r\n$4\r\nPING\r\n*1\r\n$4\r\nPING\r\n");
    }

    #[test]
    fn correlate_clears_pending_on_success() {
        let mut p = Pipeline::new();
        p.push(cmd(&[b"PING"]));
        let mut buf = BytesMut::new();
        encode(&RespValue::pong(), &mut buf);
        p.correlate(&mut buf).unwrap();
        assert_eq!(p.len(), 0);
    }

    #[test]
    fn pipeline_handles_mixed_reply_types() {
        let mut p = Pipeline::new();
        p.push(cmd(&[b"INCR", b"c"]));
        p.push(cmd(&[b"GET", b"missing"]));
        p.push(cmd(&[b"LRANGE", b"l", b"0", b"-1"]));
        let mut reply_buf = BytesMut::new();
        encode(&RespValue::Integer(5), &mut reply_buf);
        encode(&RespValue::nil_bulk(), &mut reply_buf);
        encode(&RespValue::array(vec![RespValue::bulk(b"a".to_vec())]), &mut reply_buf);
        let replies = p.correlate(&mut reply_buf).unwrap();
        assert_eq!(replies[0], RespValue::Integer(5));
        assert_eq!(replies[1], RespValue::nil_bulk());
        assert_eq!(
            replies[2],
            RespValue::array(vec![RespValue::bulk(b"a".to_vec())])
        );
    }

    #[test]
    fn command_helper_builds_array_of_bulk() {
        let c = command(&[b"SET", b"key", b"value"]);
        assert_eq!(
            c,
            RespValue::array(vec![
                RespValue::bulk(b"SET".to_vec()),
                RespValue::bulk(b"key".to_vec()),
                RespValue::bulk(b"value".to_vec()),
            ])
        );
    }
}
