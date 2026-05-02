//! `FrameCodec` trait + common error taxonomy.

use bytes::BytesMut;
use thiserror::Error;

/// Errors common to every wire codec in CAVE.
///
/// Concrete codecs (PG, OP_MSG, RESP3) wrap their protocol-specific
/// errors into [`FrameError::Invalid`] with a human-readable message.
/// Buffer-state outcomes are first-class so the I/O loop can treat them
/// without string parsing:
///
/// - [`FrameError::Incomplete`] is **not** returned by `decode`; instead
///   `decode` returns `Ok(None)` when more bytes are needed. `Incomplete`
///   exists for callers that want to convert "need more bytes" into a
///   hard error (e.g. fixed-length framing helpers).
/// - [`FrameError::Limit`] is returned when a length-prefix exceeds the
///   configured maximum frame size, before any allocation happens.
#[derive(Debug, Error)]
pub enum FrameError {
    #[error("incomplete frame: need {needed} more bytes")]
    Incomplete { needed: usize },

    #[error("invalid frame: {0}")]
    Invalid(String),

    #[error("frame size {actual} exceeds limit {max}")]
    Limit { actual: usize, max: usize },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl FrameError {
    pub fn invalid(msg: impl Into<String>) -> Self {
        FrameError::Invalid(msg.into())
    }
}

/// Encode/decode pair for a wire-protocol frame `F`.
///
/// The trait is sync and operates on a `BytesMut` buffer. It is designed
/// to slot into both async and blocking I/O loops:
///
/// ```text
/// loop {
///     // Async loop:
///     let n = socket.read_buf(&mut buf).await?;
///     if n == 0 { break; } // EOF
///     while let Some(frame) = codec.decode(&mut buf)? {
///         handle(frame);
///     }
/// }
/// ```
///
/// `decode` returns `Ok(None)` when the buffer does not yet contain a
/// full frame — the caller reads more bytes and retries. `decode` must
/// only `split_off`/`advance` bytes that belong to a successfully
/// returned frame; on `Ok(None)` the buffer is left untouched.
///
/// `encode` appends to `buf` (does not clear). Encoders can hold codec
/// state (e.g. compression dictionaries, sequence numbers); the trait
/// takes `&mut self` for that reason.
pub trait FrameCodec<F> {
    /// Append the encoded form of `frame` to `buf`.
    fn encode(&mut self, frame: F, buf: &mut BytesMut) -> Result<(), FrameError>;

    /// Try to decode one frame off `buf`.
    ///
    /// - `Ok(Some(frame))` — one frame consumed, buffer advanced.
    /// - `Ok(None)` — buffer too short, no bytes consumed.
    /// - `Err(_)` — protocol error; caller should close the connection.
    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<F>, FrameError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trivial echo codec: each frame is a length-prefixed (u8) byte run.
    /// Used to exercise the trait shape from inside cave-kernel without
    /// pulling in real protocol crates.
    struct U8Echo;

    impl FrameCodec<Vec<u8>> for U8Echo {
        fn encode(&mut self, frame: Vec<u8>, buf: &mut BytesMut) -> Result<(), FrameError> {
            if frame.len() > u8::MAX as usize {
                return Err(FrameError::Limit { actual: frame.len(), max: u8::MAX as usize });
            }
            buf.extend_from_slice(&[frame.len() as u8]);
            buf.extend_from_slice(&frame);
            Ok(())
        }

        fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Vec<u8>>, FrameError> {
            if buf.is_empty() {
                return Ok(None);
            }
            let len = buf[0] as usize;
            if buf.len() < 1 + len {
                return Ok(None);
            }
            let _ = buf.split_to(1);
            let body = buf.split_to(len);
            Ok(Some(body.to_vec()))
        }
    }

    #[test]
    fn roundtrip_single_frame() {
        let mut codec = U8Echo;
        let mut buf = BytesMut::new();
        codec.encode(b"hello".to_vec(), &mut buf).unwrap();
        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded, b"hello");
        assert!(buf.is_empty());
    }

    #[test]
    fn decode_returns_none_when_buffer_empty() {
        let mut codec = U8Echo;
        let mut buf = BytesMut::new();
        assert!(codec.decode(&mut buf).unwrap().is_none());
    }

    #[test]
    fn decode_returns_none_when_payload_partial() {
        let mut codec = U8Echo;
        let mut buf = BytesMut::from(&[5u8, b'h', b'i'][..]);
        assert!(codec.decode(&mut buf).unwrap().is_none());
        // Buffer untouched on Ok(None).
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn decode_two_frames_back_to_back() {
        let mut codec = U8Echo;
        let mut buf = BytesMut::new();
        codec.encode(b"a".to_vec(), &mut buf).unwrap();
        codec.encode(b"bb".to_vec(), &mut buf).unwrap();
        let f1 = codec.decode(&mut buf).unwrap().unwrap();
        let f2 = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(f1, b"a");
        assert_eq!(f2, b"bb");
        assert!(buf.is_empty());
    }

    #[test]
    fn encode_limit_error_carries_size() {
        let mut codec = U8Echo;
        let mut buf = BytesMut::new();
        let big = vec![0u8; 300];
        match codec.encode(big, &mut buf) {
            Err(FrameError::Limit { actual: 300, max: 255 }) => {}
            other => panic!("expected Limit error, got {:?}", other),
        }
    }

    #[test]
    fn invalid_helper_constructs_error() {
        let err = FrameError::invalid("bad opcode");
        assert!(matches!(err, FrameError::Invalid(s) if s == "bad opcode"));
    }

    #[test]
    fn io_error_is_from_std_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "boom");
        let frame_err: FrameError = io_err.into();
        assert!(matches!(frame_err, FrameError::Io(_)));
    }
}
