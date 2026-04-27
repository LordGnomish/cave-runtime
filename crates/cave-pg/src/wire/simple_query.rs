//! Simple-query mode (`Q` message → result stream).
//!
//! Modeled on `pgwire::api::query::SimpleQueryHandler`. The response shape for
//! a SELECT is:
//!
//! ```text
//! RowDescription → DataRow* → CommandComplete → ReadyForQuery
//! ```
//!
//! For DDL/DML the shape is:
//!
//! ```text
//! CommandComplete → ReadyForQuery
//! ```
//!
//! Multi-statement scripts produce one `CommandComplete` per statement and a
//! single trailing `ReadyForQuery`.

use bytes::BytesMut;

use super::codec::{
    BackendMessage, FieldDescription, FrontendMessage, ProtocolError, ProtocolResult,
    decode_backend, encode_backend, encode_frontend,
};

/// Status byte placed in the trailing `ReadyForQuery`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxStatus {
    Idle,
    InTxn,
    FailedTxn,
}

impl TxStatus {
    pub fn as_byte(self) -> u8 {
        match self {
            Self::Idle => b'I',
            Self::InTxn => b'T',
            Self::FailedTxn => b'E',
        }
    }
    pub fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            b'I' => Self::Idle,
            b'T' => Self::InTxn,
            b'E' => Self::FailedTxn,
            _ => return None,
        })
    }
}

/// A response slice produced by one statement in a simple-query script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementResult {
    pub fields: Option<Vec<FieldDescription>>,
    pub rows: Vec<Vec<Option<Vec<u8>>>>,
    pub completion_tag: String,
}

impl StatementResult {
    pub fn select(fields: Vec<FieldDescription>, rows: Vec<Vec<Option<Vec<u8>>>>) -> Self {
        let tag = format!("SELECT {}", rows.len());
        Self {
            fields: Some(fields),
            rows,
            completion_tag: tag,
        }
    }

    pub fn insert(rows_affected: u64) -> Self {
        Self {
            fields: None,
            rows: vec![],
            completion_tag: format!("INSERT 0 {}", rows_affected),
        }
    }

    pub fn update(rows_affected: u64) -> Self {
        Self {
            fields: None,
            rows: vec![],
            completion_tag: format!("UPDATE {}", rows_affected),
        }
    }

    pub fn delete(rows_affected: u64) -> Self {
        Self {
            fields: None,
            rows: vec![],
            completion_tag: format!("DELETE {}", rows_affected),
        }
    }

    pub fn ddl(verb: &str) -> Self {
        Self {
            fields: None,
            rows: vec![],
            completion_tag: verb.to_string(),
        }
    }
}

/// Render a list of results into the on-wire backend response stream.
pub fn render_query_response(
    results: &[StatementResult],
    tx_status: TxStatus,
    buf: &mut BytesMut,
) {
    if results.is_empty() {
        encode_backend(&BackendMessage::EmptyQueryResponse, buf);
    } else {
        for r in results {
            if let Some(fields) = &r.fields {
                encode_backend(&BackendMessage::RowDescription(fields.clone()), buf);
                for row in &r.rows {
                    encode_backend(&BackendMessage::DataRow(row.clone()), buf);
                }
            }
            encode_backend(&BackendMessage::CommandComplete(r.completion_tag.clone()), buf);
        }
    }
    encode_backend(&BackendMessage::ReadyForQuery(tx_status.as_byte()), buf);
}

/// Drain `buf` and parse a sequence of backend messages produced by one
/// simple-query response. Stops at the first `ReadyForQuery`.
pub fn parse_query_response(buf: &mut BytesMut) -> ProtocolResult<(Vec<StatementResult>, TxStatus)> {
    let mut results = Vec::new();
    let mut current_fields: Option<Vec<FieldDescription>> = None;
    let mut current_rows: Vec<Vec<Option<Vec<u8>>>> = Vec::new();
    loop {
        let msg = decode_backend(buf)?;
        match msg {
            BackendMessage::EmptyQueryResponse => {
                // continue — consume the trailing ReadyForQuery
            }
            BackendMessage::RowDescription(fields) => {
                current_fields = Some(fields);
                current_rows.clear();
            }
            BackendMessage::DataRow(cells) => current_rows.push(cells),
            BackendMessage::CommandComplete(tag) => {
                results.push(StatementResult {
                    fields: current_fields.take(),
                    rows: std::mem::take(&mut current_rows),
                    completion_tag: tag,
                });
            }
            BackendMessage::ReadyForQuery(s) => {
                let status = TxStatus::from_byte(s).ok_or(ProtocolError::InvalidValue {
                    field: "ReadyForQuery.tx_status",
                    reason: format!("0x{:02x}", s),
                })?;
                return Ok((results, status));
            }
            BackendMessage::ErrorResponse(_) | BackendMessage::NoticeResponse(_) => {
                // ignore in this thin parser; callers wanting the error stream
                // should call `decode_backend` directly.
            }
            other => {
                return Err(ProtocolError::InvalidValue {
                    field: "simple_query.unexpected_message",
                    reason: format!("{:?}", other),
                });
            }
        }
    }
}

/// Build the frontend `Q` request bytes for `query`.
pub fn build_query(query: &str) -> BytesMut {
    let mut buf = BytesMut::new();
    encode_frontend(&FrontendMessage::Query(query.into()), &mut buf);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_field(name: &str, type_oid: u32) -> FieldDescription {
        FieldDescription {
            name: name.into(),
            table_oid: 0,
            column_attr: 0,
            type_oid,
            type_size: -1,
            type_modifier: -1,
            format_code: 0,
        }
    }

    #[test]
    fn tx_status_byte_round_trip() {
        for s in [TxStatus::Idle, TxStatus::InTxn, TxStatus::FailedTxn] {
            assert_eq!(TxStatus::from_byte(s.as_byte()), Some(s));
        }
        assert_eq!(TxStatus::from_byte(b'X'), None);
    }

    #[test]
    fn select_completion_tag_includes_count() {
        let r = StatementResult::select(vec![make_field("a", 23)], vec![vec![Some(b"1".to_vec())]]);
        assert_eq!(r.completion_tag, "SELECT 1");
    }

    #[test]
    fn insert_completion_tag_includes_oid_zero() {
        let r = StatementResult::insert(3);
        assert_eq!(r.completion_tag, "INSERT 0 3");
    }

    #[test]
    fn update_completion_tag() {
        assert_eq!(StatementResult::update(7).completion_tag, "UPDATE 7");
    }

    #[test]
    fn delete_completion_tag() {
        assert_eq!(StatementResult::delete(0).completion_tag, "DELETE 0");
    }

    #[test]
    fn ddl_completion_tag_passthrough() {
        assert_eq!(StatementResult::ddl("CREATE TABLE").completion_tag, "CREATE TABLE");
    }

    #[test]
    fn empty_query_round_trip() {
        let mut buf = BytesMut::new();
        render_query_response(&[], TxStatus::Idle, &mut buf);
        let (results, status) = parse_query_response(&mut buf).unwrap();
        assert!(results.is_empty());
        assert_eq!(status, TxStatus::Idle);
    }

    #[test]
    fn select_one_row_round_trip() {
        let r = StatementResult::select(
            vec![make_field("id", 23)],
            vec![vec![Some(b"42".to_vec())]],
        );
        let mut buf = BytesMut::new();
        render_query_response(&[r.clone()], TxStatus::Idle, &mut buf);
        let (results, status) = parse_query_response(&mut buf).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], r);
        assert_eq!(status, TxStatus::Idle);
    }

    #[test]
    fn select_multi_row_round_trip() {
        let r = StatementResult::select(
            vec![make_field("id", 23), make_field("name", 25)],
            vec![
                vec![Some(b"1".to_vec()), Some(b"alice".to_vec())],
                vec![Some(b"2".to_vec()), None],
                vec![Some(b"3".to_vec()), Some(b"carol".to_vec())],
            ],
        );
        let mut buf = BytesMut::new();
        render_query_response(&[r.clone()], TxStatus::InTxn, &mut buf);
        let (results, status) = parse_query_response(&mut buf).unwrap();
        assert_eq!(results, vec![r]);
        assert_eq!(status, TxStatus::InTxn);
    }

    #[test]
    fn ddl_then_select_round_trip() {
        let r1 = StatementResult::ddl("CREATE TABLE");
        let r2 = StatementResult::insert(1);
        let r3 = StatementResult::select(
            vec![make_field("c", 23)],
            vec![vec![Some(b"1".to_vec())]],
        );
        let mut buf = BytesMut::new();
        render_query_response(&[r1.clone(), r2.clone(), r3.clone()], TxStatus::Idle, &mut buf);
        let (results, _) = parse_query_response(&mut buf).unwrap();
        assert_eq!(results, vec![r1, r2, r3]);
    }

    #[test]
    fn build_query_produces_q_frame() {
        let buf = build_query("SELECT 1");
        assert_eq!(buf[0], b'Q');
    }

    #[test]
    fn failed_tx_status_round_trips() {
        let mut buf = BytesMut::new();
        render_query_response(&[StatementResult::ddl("ROLLBACK")], TxStatus::FailedTxn, &mut buf);
        let (_, status) = parse_query_response(&mut buf).unwrap();
        assert_eq!(status, TxStatus::FailedTxn);
    }

    #[test]
    fn parse_invalid_tx_status_errors() {
        let mut buf = BytesMut::new();
        encode_backend(&BackendMessage::ReadyForQuery(b'X'), &mut buf);
        let err = parse_query_response(&mut buf).unwrap_err();
        match err {
            ProtocolError::InvalidValue { field, .. } => {
                assert_eq!(field, "ReadyForQuery.tx_status")
            }
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn data_row_with_all_nulls_round_trip() {
        let r = StatementResult::select(
            vec![make_field("a", 23), make_field("b", 23)],
            vec![vec![None, None]],
        );
        let mut buf = BytesMut::new();
        render_query_response(&[r.clone()], TxStatus::Idle, &mut buf);
        let (results, _) = parse_query_response(&mut buf).unwrap();
        assert_eq!(results[0], r);
    }
}
