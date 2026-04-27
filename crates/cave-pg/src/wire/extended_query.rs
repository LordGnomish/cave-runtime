//! Extended-query protocol — Parse / Bind / Describe / Execute / Sync.
//!
//! Extended query splits a request into reusable named **prepared statements**
//! and short-lived **portals**. The canonical message sequence is:
//!
//! ```text
//! Parse(stmt) → Describe('S', stmt) → Bind(stmt → portal) →
//!   Describe('P', portal) → Execute(portal, max_rows) → Sync
//! ```
//!
//! Backend reply for the above is:
//!
//! ```text
//! ParseComplete → ParameterDescription → RowDescription | NoData →
//!   BindComplete → RowDescription | NoData → DataRow* →
//!   CommandComplete | PortalSuspended | EmptyQueryResponse → ReadyForQuery
//! ```
//!
//! Modeled on `pgwire::api::query::ExtendedQueryHandler` with a tiny
//! [`PreparedStatementCache`] that the upstream callers can plug in.

use std::collections::HashMap;

use bytes::BytesMut;

use super::codec::{
    BackendMessage, FieldDescription, FrontendMessage, ProtocolError, ProtocolResult,
    decode_backend, encode_backend, encode_frontend,
};
use super::simple_query::TxStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedStatement {
    pub name: String,
    pub query: String,
    pub param_oids: Vec<u32>,
    pub fields: Option<Vec<FieldDescription>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Portal {
    pub name: String,
    pub statement_name: String,
    pub param_formats: Vec<i16>,
    pub params: Vec<Option<Vec<u8>>>,
    pub result_formats: Vec<i16>,
}

#[derive(Debug, Default)]
pub struct PreparedStatementCache {
    statements: HashMap<String, PreparedStatement>,
    portals: HashMap<String, Portal>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExtendedQueryError {
    #[error("unknown prepared statement: {0:?}")]
    UnknownStatement(String),
    #[error("unknown portal: {0:?}")]
    UnknownPortal(String),
    #[error("invalid describe kind: 0x{0:02x}")]
    InvalidDescribeKind(u8),
}

impl PreparedStatementCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_statement(&mut self, stmt: PreparedStatement) {
        self.statements.insert(stmt.name.clone(), stmt);
    }

    pub fn insert_portal(&mut self, portal: Portal) -> Result<(), ExtendedQueryError> {
        if !self.statements.contains_key(&portal.statement_name) {
            return Err(ExtendedQueryError::UnknownStatement(
                portal.statement_name.clone(),
            ));
        }
        self.portals.insert(portal.name.clone(), portal);
        Ok(())
    }

    pub fn get_statement(&self, name: &str) -> Option<&PreparedStatement> {
        self.statements.get(name)
    }

    pub fn get_portal(&self, name: &str) -> Option<&Portal> {
        self.portals.get(name)
    }

    pub fn close_statement(&mut self, name: &str) -> bool {
        let removed = self.statements.remove(name).is_some();
        if removed {
            self.portals.retain(|_, p| p.statement_name != name);
        }
        removed
    }

    pub fn close_portal(&mut self, name: &str) -> bool {
        self.portals.remove(name).is_some()
    }

    pub fn statement_count(&self) -> usize {
        self.statements.len()
    }

    pub fn portal_count(&self) -> usize {
        self.portals.len()
    }
}

/// Apply one frontend extended-query message to the cache, returning the
/// backend reply messages that should be appended to the output buffer.
pub fn apply_message(
    cache: &mut PreparedStatementCache,
    msg: &FrontendMessage,
) -> Result<Vec<BackendMessage>, ExtendedQueryError> {
    match msg {
        FrontendMessage::Parse {
            statement_name,
            query,
            param_oids,
        } => {
            cache.insert_statement(PreparedStatement {
                name: statement_name.clone(),
                query: query.clone(),
                param_oids: param_oids.clone(),
                fields: None,
            });
            Ok(vec![BackendMessage::ParseComplete])
        }
        FrontendMessage::Bind {
            portal_name,
            statement_name,
            param_formats,
            params,
            result_formats,
        } => {
            cache.insert_portal(Portal {
                name: portal_name.clone(),
                statement_name: statement_name.clone(),
                param_formats: param_formats.clone(),
                params: params.clone(),
                result_formats: result_formats.clone(),
            })?;
            Ok(vec![BackendMessage::BindComplete])
        }
        FrontendMessage::Describe { kind, name } => match *kind {
            b'S' => {
                let stmt = cache
                    .get_statement(name)
                    .ok_or_else(|| ExtendedQueryError::UnknownStatement(name.clone()))?;
                let mut out = Vec::new();
                out.push(BackendMessage::ParameterDescription(stmt.param_oids.clone()));
                match &stmt.fields {
                    Some(fields) => out.push(BackendMessage::RowDescription(fields.clone())),
                    None => out.push(BackendMessage::NoData),
                }
                Ok(out)
            }
            b'P' => {
                let portal = cache
                    .get_portal(name)
                    .ok_or_else(|| ExtendedQueryError::UnknownPortal(name.clone()))?;
                let stmt = cache
                    .get_statement(&portal.statement_name)
                    .ok_or_else(|| ExtendedQueryError::UnknownStatement(portal.statement_name.clone()))?;
                Ok(vec![match &stmt.fields {
                    Some(fields) => BackendMessage::RowDescription(fields.clone()),
                    None => BackendMessage::NoData,
                }])
            }
            other => Err(ExtendedQueryError::InvalidDescribeKind(other)),
        },
        FrontendMessage::Close { kind, name } => {
            let _ok = match *kind {
                b'S' => cache.close_statement(name),
                b'P' => cache.close_portal(name),
                other => return Err(ExtendedQueryError::InvalidDescribeKind(other)),
            };
            Ok(vec![BackendMessage::CloseComplete])
        }
        FrontendMessage::Sync => Ok(vec![BackendMessage::ReadyForQuery(TxStatus::Idle.as_byte())]),
        FrontendMessage::Flush => Ok(vec![]),
        _ => Ok(vec![]),
    }
}

/// Build the canonical "Parse → Bind → Describe → Execute → Sync" sequence
/// emitted by libpq's `PQexecParams` for a one-shot extended query.
pub fn build_one_shot(
    statement_name: &str,
    query: &str,
    param_oids: &[u32],
    params: &[Option<Vec<u8>>],
    result_formats: &[i16],
) -> BytesMut {
    let mut buf = BytesMut::new();
    encode_frontend(
        &FrontendMessage::Parse {
            statement_name: statement_name.into(),
            query: query.into(),
            param_oids: param_oids.to_vec(),
        },
        &mut buf,
    );
    encode_frontend(
        &FrontendMessage::Bind {
            portal_name: String::new(),
            statement_name: statement_name.into(),
            param_formats: vec![0; params.len()],
            params: params.to_vec(),
            result_formats: result_formats.to_vec(),
        },
        &mut buf,
    );
    encode_frontend(
        &FrontendMessage::Describe {
            kind: b'P',
            name: String::new(),
        },
        &mut buf,
    );
    encode_frontend(
        &FrontendMessage::Execute {
            portal_name: String::new(),
            max_rows: 0,
        },
        &mut buf,
    );
    encode_frontend(&FrontendMessage::Sync, &mut buf);
    buf
}

/// Drain `buf` and parse a single extended-query response. Returns when a
/// `ReadyForQuery` is seen.
pub fn parse_response(buf: &mut BytesMut) -> ProtocolResult<ExtendedResponse> {
    let mut out = ExtendedResponse::default();
    loop {
        let msg = decode_backend(buf)?;
        match msg {
            BackendMessage::ParseComplete => out.parse_complete = true,
            BackendMessage::BindComplete => out.bind_complete = true,
            BackendMessage::CloseComplete => out.close_complete = true,
            BackendMessage::ParameterDescription(oids) => out.param_oids = Some(oids),
            BackendMessage::RowDescription(fields) => out.fields = Some(fields),
            BackendMessage::NoData => out.no_data = true,
            BackendMessage::DataRow(cells) => out.rows.push(cells),
            BackendMessage::CommandComplete(tag) => out.completion_tag = Some(tag),
            BackendMessage::PortalSuspended => out.portal_suspended = true,
            BackendMessage::EmptyQueryResponse => out.empty_query = true,
            BackendMessage::ReadyForQuery(s) => {
                out.ready = TxStatus::from_byte(s);
                return Ok(out);
            }
            BackendMessage::ErrorResponse(fields) => out.error = Some(fields),
            BackendMessage::NoticeResponse(_) => {}
            other => {
                return Err(ProtocolError::InvalidValue {
                    field: "extended_query.unexpected_message",
                    reason: format!("{:?}", other),
                });
            }
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ExtendedResponse {
    pub parse_complete: bool,
    pub bind_complete: bool,
    pub close_complete: bool,
    pub param_oids: Option<Vec<u32>>,
    pub fields: Option<Vec<FieldDescription>>,
    pub no_data: bool,
    pub rows: Vec<Vec<Option<Vec<u8>>>>,
    pub completion_tag: Option<String>,
    pub portal_suspended: bool,
    pub empty_query: bool,
    pub ready: Option<TxStatus>,
    pub error: Option<Vec<(u8, String)>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_inserts_statement() {
        let mut cache = PreparedStatementCache::new();
        let reply = apply_message(
            &mut cache,
            &FrontendMessage::Parse {
                statement_name: "s1".into(),
                query: "SELECT $1".into(),
                param_oids: vec![23],
            },
        )
        .unwrap();
        assert_eq!(reply, vec![BackendMessage::ParseComplete]);
        assert!(cache.get_statement("s1").is_some());
        assert_eq!(cache.statement_count(), 1);
    }

    #[test]
    fn bind_requires_existing_statement() {
        let mut cache = PreparedStatementCache::new();
        let err = apply_message(
            &mut cache,
            &FrontendMessage::Bind {
                portal_name: "p".into(),
                statement_name: "missing".into(),
                param_formats: vec![],
                params: vec![],
                result_formats: vec![],
            },
        )
        .unwrap_err();
        assert_eq!(err, ExtendedQueryError::UnknownStatement("missing".into()));
    }

    #[test]
    fn bind_after_parse_succeeds() {
        let mut cache = PreparedStatementCache::new();
        apply_message(
            &mut cache,
            &FrontendMessage::Parse {
                statement_name: "s1".into(),
                query: "SELECT 1".into(),
                param_oids: vec![],
            },
        )
        .unwrap();
        let reply = apply_message(
            &mut cache,
            &FrontendMessage::Bind {
                portal_name: "p1".into(),
                statement_name: "s1".into(),
                param_formats: vec![],
                params: vec![],
                result_formats: vec![],
            },
        )
        .unwrap();
        assert_eq!(reply, vec![BackendMessage::BindComplete]);
        assert_eq!(cache.portal_count(), 1);
    }

    #[test]
    fn describe_statement_includes_param_description() {
        let mut cache = PreparedStatementCache::new();
        cache.insert_statement(PreparedStatement {
            name: "s".into(),
            query: "SELECT $1::int4".into(),
            param_oids: vec![23],
            fields: None,
        });
        let reply = apply_message(
            &mut cache,
            &FrontendMessage::Describe {
                kind: b'S',
                name: "s".into(),
            },
        )
        .unwrap();
        assert_eq!(reply.len(), 2);
        assert_eq!(reply[0], BackendMessage::ParameterDescription(vec![23]));
        assert_eq!(reply[1], BackendMessage::NoData);
    }

    #[test]
    fn describe_statement_with_fields_returns_row_description() {
        let mut cache = PreparedStatementCache::new();
        let fields = vec![FieldDescription {
            name: "n".into(),
            table_oid: 0,
            column_attr: 0,
            type_oid: 23,
            type_size: 4,
            type_modifier: -1,
            format_code: 0,
        }];
        cache.insert_statement(PreparedStatement {
            name: "s".into(),
            query: "SELECT 1".into(),
            param_oids: vec![],
            fields: Some(fields.clone()),
        });
        let reply = apply_message(
            &mut cache,
            &FrontendMessage::Describe {
                kind: b'S',
                name: "s".into(),
            },
        )
        .unwrap();
        assert_eq!(
            reply,
            vec![
                BackendMessage::ParameterDescription(vec![]),
                BackendMessage::RowDescription(fields)
            ]
        );
    }

    #[test]
    fn describe_unknown_statement_errors() {
        let mut cache = PreparedStatementCache::new();
        let err = apply_message(
            &mut cache,
            &FrontendMessage::Describe {
                kind: b'S',
                name: "nope".into(),
            },
        )
        .unwrap_err();
        assert_eq!(err, ExtendedQueryError::UnknownStatement("nope".into()));
    }

    #[test]
    fn describe_invalid_kind_errors() {
        let mut cache = PreparedStatementCache::new();
        let err = apply_message(
            &mut cache,
            &FrontendMessage::Describe {
                kind: b'X',
                name: "s".into(),
            },
        )
        .unwrap_err();
        assert_eq!(err, ExtendedQueryError::InvalidDescribeKind(b'X'));
    }

    #[test]
    fn close_statement_removes_dependent_portals() {
        let mut cache = PreparedStatementCache::new();
        apply_message(
            &mut cache,
            &FrontendMessage::Parse {
                statement_name: "s1".into(),
                query: "SELECT 1".into(),
                param_oids: vec![],
            },
        )
        .unwrap();
        apply_message(
            &mut cache,
            &FrontendMessage::Bind {
                portal_name: "p1".into(),
                statement_name: "s1".into(),
                param_formats: vec![],
                params: vec![],
                result_formats: vec![],
            },
        )
        .unwrap();
        let reply = apply_message(
            &mut cache,
            &FrontendMessage::Close {
                kind: b'S',
                name: "s1".into(),
            },
        )
        .unwrap();
        assert_eq!(reply, vec![BackendMessage::CloseComplete]);
        assert_eq!(cache.statement_count(), 0);
        assert_eq!(cache.portal_count(), 0);
    }

    #[test]
    fn close_portal_only_removes_portal() {
        let mut cache = PreparedStatementCache::new();
        apply_message(
            &mut cache,
            &FrontendMessage::Parse {
                statement_name: "s".into(),
                query: "SELECT 1".into(),
                param_oids: vec![],
            },
        )
        .unwrap();
        apply_message(
            &mut cache,
            &FrontendMessage::Bind {
                portal_name: "p".into(),
                statement_name: "s".into(),
                param_formats: vec![],
                params: vec![],
                result_formats: vec![],
            },
        )
        .unwrap();
        apply_message(
            &mut cache,
            &FrontendMessage::Close {
                kind: b'P',
                name: "p".into(),
            },
        )
        .unwrap();
        assert_eq!(cache.statement_count(), 1);
        assert_eq!(cache.portal_count(), 0);
    }

    #[test]
    fn sync_emits_ready_for_query_idle() {
        let mut cache = PreparedStatementCache::new();
        let reply = apply_message(&mut cache, &FrontendMessage::Sync).unwrap();
        assert_eq!(reply, vec![BackendMessage::ReadyForQuery(b'I')]);
    }

    #[test]
    fn flush_returns_empty() {
        let mut cache = PreparedStatementCache::new();
        let reply = apply_message(&mut cache, &FrontendMessage::Flush).unwrap();
        assert!(reply.is_empty());
    }

    #[test]
    fn one_shot_emits_five_messages() {
        let buf = build_one_shot(
            "",
            "SELECT $1::int4",
            &[23],
            &[Some(b"42".to_vec())],
            &[0],
        );
        let mut tail = buf.clone();
        let m1 = super::super::codec::decode_frontend(&mut tail, false).unwrap();
        let m2 = super::super::codec::decode_frontend(&mut tail, false).unwrap();
        let m3 = super::super::codec::decode_frontend(&mut tail, false).unwrap();
        let m4 = super::super::codec::decode_frontend(&mut tail, false).unwrap();
        let m5 = super::super::codec::decode_frontend(&mut tail, false).unwrap();
        assert!(matches!(m1, FrontendMessage::Parse { .. }));
        assert!(matches!(m2, FrontendMessage::Bind { .. }));
        assert!(matches!(m3, FrontendMessage::Describe { kind: b'P', .. }));
        assert!(matches!(m4, FrontendMessage::Execute { .. }));
        assert!(matches!(m5, FrontendMessage::Sync));
        assert!(tail.is_empty());
    }

    #[test]
    fn parse_response_select_with_one_row() {
        let mut buf = BytesMut::new();
        encode_backend(&BackendMessage::ParseComplete, &mut buf);
        encode_backend(&BackendMessage::BindComplete, &mut buf);
        encode_backend(
            &BackendMessage::RowDescription(vec![FieldDescription {
                name: "a".into(),
                table_oid: 0,
                column_attr: 0,
                type_oid: 23,
                type_size: 4,
                type_modifier: -1,
                format_code: 0,
            }]),
            &mut buf,
        );
        encode_backend(
            &BackendMessage::DataRow(vec![Some(b"1".to_vec())]),
            &mut buf,
        );
        encode_backend(&BackendMessage::CommandComplete("SELECT 1".into()), &mut buf);
        encode_backend(&BackendMessage::ReadyForQuery(b'I'), &mut buf);
        let resp = parse_response(&mut buf).unwrap();
        assert!(resp.parse_complete);
        assert!(resp.bind_complete);
        assert!(resp.fields.is_some());
        assert_eq!(resp.rows, vec![vec![Some(b"1".to_vec())]]);
        assert_eq!(resp.completion_tag.as_deref(), Some("SELECT 1"));
        assert_eq!(resp.ready, Some(TxStatus::Idle));
    }

    #[test]
    fn parse_response_with_error() {
        let mut buf = BytesMut::new();
        encode_backend(&BackendMessage::ParseComplete, &mut buf);
        encode_backend(
            &BackendMessage::ErrorResponse(vec![(b'M', "boom".into())]),
            &mut buf,
        );
        encode_backend(&BackendMessage::ReadyForQuery(b'E'), &mut buf);
        let resp = parse_response(&mut buf).unwrap();
        assert!(resp.parse_complete);
        assert_eq!(resp.error.as_ref().unwrap()[0].1, "boom");
        assert_eq!(resp.ready, Some(TxStatus::FailedTxn));
    }

    #[test]
    fn parse_response_portal_suspended() {
        let mut buf = BytesMut::new();
        encode_backend(&BackendMessage::PortalSuspended, &mut buf);
        encode_backend(&BackendMessage::ReadyForQuery(b'T'), &mut buf);
        let resp = parse_response(&mut buf).unwrap();
        assert!(resp.portal_suspended);
        assert_eq!(resp.ready, Some(TxStatus::InTxn));
    }
}
