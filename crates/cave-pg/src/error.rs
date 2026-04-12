//! PostgreSQL error types with full SQLSTATE codes and wire-protocol field encoding.

use thiserror::Error;

/// SQLSTATE error codes — 5-character codes as per PostgreSQL documentation.
/// See https://www.postgresql.org/docs/current/errcodes-appendix.html
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlState(pub &'static str);

#[allow(dead_code)]
impl SqlState {
    // Class 00 — Successful Completion
    pub const SUCCESSFUL_COMPLETION: SqlState = SqlState("00000");

    // Class 01 — Warning
    pub const WARNING: SqlState = SqlState("01000");
    pub const NULL_VALUE_ELIMINATED_IN_SET_FUNCTION: SqlState = SqlState("01003");
    pub const STRING_DATA_RIGHT_TRUNCATION_WARNING: SqlState = SqlState("01004");
    pub const PRIVILEGE_NOT_REVOKED: SqlState = SqlState("01006");
    pub const PRIVILEGE_NOT_GRANTED: SqlState = SqlState("01007");
    pub const IMPLICIT_ZERO_BIT_PADDING: SqlState = SqlState("01008");

    // Class 02 — No Data
    pub const NO_DATA: SqlState = SqlState("02000");
    pub const NO_ADDITIONAL_DYNAMIC_RESULT_SETS_RETURNED: SqlState = SqlState("02001");

    // Class 03 — SQL Statement Not Yet Complete
    pub const SQL_STATEMENT_NOT_YET_COMPLETE: SqlState = SqlState("03000");

    // Class 08 — Connection Exception
    pub const CONNECTION_EXCEPTION: SqlState = SqlState("08000");
    pub const CONNECTION_DOES_NOT_EXIST: SqlState = SqlState("08003");
    pub const CONNECTION_FAILURE: SqlState = SqlState("08006");
    pub const SQLCLIENT_UNABLE_TO_ESTABLISH_SQLCONNECTION: SqlState = SqlState("08001");
    pub const SQLSERVER_REJECTED_ESTABLISHMENT_OF_SQLCONNECTION: SqlState = SqlState("08004");
    pub const TRANSACTION_RESOLUTION_UNKNOWN: SqlState = SqlState("08007");
    pub const PROTOCOL_VIOLATION: SqlState = SqlState("08P01");

    // Class 09 — Triggered Action Exception
    pub const TRIGGERED_ACTION_EXCEPTION: SqlState = SqlState("09000");

    // Class 0A — Feature Not Supported
    pub const FEATURE_NOT_SUPPORTED: SqlState = SqlState("0A000");

    // Class 0B — Invalid Transaction Initiation
    pub const INVALID_TRANSACTION_INITIATION: SqlState = SqlState("0B000");

    // Class 0F — Locator Exception
    pub const LOCATOR_EXCEPTION: SqlState = SqlState("0F000");

    // Class 0L — Invalid Grantor
    pub const INVALID_GRANTOR: SqlState = SqlState("0L000");

    // Class 0P — Invalid Role Specification
    pub const INVALID_ROLE_SPECIFICATION: SqlState = SqlState("0P000");

    // Class 0Z — Diagnostics Exception
    pub const DIAGNOSTICS_EXCEPTION: SqlState = SqlState("0Z000");

    // Class 20 — Case Not Found
    pub const CASE_NOT_FOUND: SqlState = SqlState("20000");

    // Class 21 — Cardinality Violation
    pub const CARDINALITY_VIOLATION: SqlState = SqlState("21000");

    // Class 22 — Data Exception
    pub const DATA_EXCEPTION: SqlState = SqlState("22000");
    pub const ARRAY_SUBSCRIPT_ERROR: SqlState = SqlState("2202E");
    pub const CHARACTER_NOT_IN_REPERTOIRE: SqlState = SqlState("22021");
    pub const DATETIME_FIELD_OVERFLOW: SqlState = SqlState("22008");
    pub const DIVISION_BY_ZERO: SqlState = SqlState("22012");
    pub const ERROR_IN_ASSIGNMENT: SqlState = SqlState("22005");
    pub const ESCAPE_CHARACTER_CONFLICT: SqlState = SqlState("2200B");
    pub const INDICATOR_OVERFLOW: SqlState = SqlState("22022");
    pub const INTERVAL_FIELD_OVERFLOW: SqlState = SqlState("22015");
    pub const INVALID_ARGUMENT_FOR_LOGARITHM: SqlState = SqlState("2201E");
    pub const INVALID_ARGUMENT_FOR_NTILE_FUNCTION: SqlState = SqlState("22014");
    pub const INVALID_ARGUMENT_FOR_NTH_VALUE_FUNCTION: SqlState = SqlState("22016");
    pub const INVALID_ARGUMENT_FOR_POWER_FUNCTION: SqlState = SqlState("2201F");
    pub const INVALID_ARGUMENT_FOR_WIDTH_BUCKET_FUNCTION: SqlState = SqlState("2201G");
    pub const INVALID_CHARACTER_VALUE_FOR_CAST: SqlState = SqlState("22018");
    pub const INVALID_DATETIME_FORMAT: SqlState = SqlState("22007");
    pub const INVALID_ESCAPE_CHARACTER: SqlState = SqlState("22019");
    pub const INVALID_ESCAPE_OCTET: SqlState = SqlState("2200D");
    pub const INVALID_ESCAPE_SEQUENCE: SqlState = SqlState("22025");
    pub const NONSTANDARD_USE_OF_ESCAPE_CHARACTER: SqlState = SqlState("22P06");
    pub const INVALID_INDICATOR_PARAMETER_VALUE: SqlState = SqlState("22010");
    pub const INVALID_PARAMETER_VALUE: SqlState = SqlState("22023");
    pub const INVALID_PRECEDING_OR_FOLLOWING_SIZE: SqlState = SqlState("22013");
    pub const INVALID_REGULAR_EXPRESSION: SqlState = SqlState("2201B");
    pub const INVALID_ROW_COUNT_IN_LIMIT_CLAUSE: SqlState = SqlState("2201W");
    pub const INVALID_ROW_COUNT_IN_RESULT_OFFSET_CLAUSE: SqlState = SqlState("2201X");
    pub const INVALID_TABLESAMPLE_ARGUMENT: SqlState = SqlState("2202H");
    pub const INVALID_TABLESAMPLE_REPEAT: SqlState = SqlState("2202G");
    pub const INVALID_TIME_ZONE_DISPLACEMENT_VALUE: SqlState = SqlState("22009");
    pub const INVALID_USE_OF_ESCAPE_CHARACTER: SqlState = SqlState("2200C");
    pub const MOST_SPECIFIC_TYPE_MISMATCH: SqlState = SqlState("2200G");
    pub const NULL_VALUE_NOT_ALLOWED: SqlState = SqlState("22004");
    pub const NULL_VALUE_NO_INDICATOR_PARAMETER: SqlState = SqlState("22002");
    pub const NUMERIC_VALUE_OUT_OF_RANGE: SqlState = SqlState("22003");
    pub const SEQUENCE_GENERATOR_LIMIT_EXCEEDED: SqlState = SqlState("2200H");
    pub const STRING_DATA_LENGTH_MISMATCH: SqlState = SqlState("22026");
    pub const STRING_DATA_RIGHT_TRUNCATION: SqlState = SqlState("22001");
    pub const SUBSTRING_ERROR: SqlState = SqlState("22011");
    pub const TRIM_ERROR: SqlState = SqlState("22027");
    pub const UNTERMINATED_C_STRING: SqlState = SqlState("22024");
    pub const ZERO_LENGTH_CHARACTER_STRING: SqlState = SqlState("2200F");
    pub const FLOATING_POINT_EXCEPTION: SqlState = SqlState("22P01");
    pub const INVALID_TEXT_REPRESENTATION: SqlState = SqlState("22P02");
    pub const INVALID_BINARY_REPRESENTATION: SqlState = SqlState("22P03");
    pub const BAD_COPY_FILE_FORMAT: SqlState = SqlState("22P04");
    pub const UNTRANSLATABLE_CHARACTER: SqlState = SqlState("22P05");
    pub const NOT_AN_XML_DOCUMENT: SqlState = SqlState("2200L");
    pub const INVALID_XML_DOCUMENT: SqlState = SqlState("2200M");
    pub const INVALID_XML_CONTENT: SqlState = SqlState("2200N");
    pub const INVALID_XML_COMMENT: SqlState = SqlState("2200S");
    pub const INVALID_XML_PROCESSING_INSTRUCTION: SqlState = SqlState("2200T");
    pub const DUPLICATE_JSON_OBJECT_KEY_VALUE: SqlState = SqlState("22030");
    pub const INVALID_ARGUMENT_FOR_SQL_JSON_DATETIME_FUNCTION: SqlState = SqlState("22031");
    pub const INVALID_JSON_TEXT: SqlState = SqlState("22032");
    pub const INVALID_SQL_JSON_SUBSCRIPT: SqlState = SqlState("22033");
    pub const MORE_THAN_ONE_SQL_JSON_ITEM: SqlState = SqlState("22034");
    pub const NO_SQL_JSON_ITEM: SqlState = SqlState("22035");
    pub const NON_NUMERIC_SQL_JSON_ITEM: SqlState = SqlState("22036");
    pub const NON_UNIQUE_KEYS_IN_A_JSON_OBJECT: SqlState = SqlState("22037");
    pub const SINGLETON_SQL_JSON_ITEM_REQUIRED: SqlState = SqlState("22038");
    pub const SQL_JSON_ARRAY_NOT_FOUND: SqlState = SqlState("22039");
    pub const SQL_JSON_MEMBER_NOT_FOUND: SqlState = SqlState("2203A");
    pub const SQL_JSON_NUMBER_NOT_FOUND: SqlState = SqlState("2203B");
    pub const SQL_JSON_OBJECT_NOT_FOUND: SqlState = SqlState("2203C");
    pub const TOO_MANY_JSON_ARRAY_ELEMENTS: SqlState = SqlState("2203D");
    pub const TOO_MANY_JSON_OBJECT_MEMBERS: SqlState = SqlState("2203E");
    pub const SQL_JSON_SCALAR_REQUIRED: SqlState = SqlState("2203F");
    pub const SQL_JSON_ITEM_CANNOT_BE_CAST_TO_TARGET_TYPE: SqlState = SqlState("2203G");

    // Class 23 — Integrity Constraint Violation
    pub const INTEGRITY_CONSTRAINT_VIOLATION: SqlState = SqlState("23000");
    pub const RESTRICT_VIOLATION: SqlState = SqlState("23001");
    pub const NOT_NULL_VIOLATION: SqlState = SqlState("23502");
    pub const FOREIGN_KEY_VIOLATION: SqlState = SqlState("23503");
    pub const UNIQUE_VIOLATION: SqlState = SqlState("23505");
    pub const CHECK_VIOLATION: SqlState = SqlState("23514");
    pub const EXCLUSION_VIOLATION: SqlState = SqlState("23P01");

    // Class 24 — Invalid Cursor State
    pub const INVALID_CURSOR_STATE: SqlState = SqlState("24000");

    // Class 25 — Invalid Transaction State
    pub const INVALID_TRANSACTION_STATE: SqlState = SqlState("25000");
    pub const ACTIVE_SQL_TRANSACTION: SqlState = SqlState("25001");
    pub const BRANCH_TRANSACTION_ALREADY_ACTIVE: SqlState = SqlState("25002");
    pub const HELD_CURSOR_REQUIRES_SAME_ISOLATION_LEVEL: SqlState = SqlState("25008");
    pub const INAPPROPRIATE_ACCESS_MODE_FOR_BRANCH_TRANSACTION: SqlState = SqlState("25003");
    pub const INAPPROPRIATE_ISOLATION_LEVEL_FOR_BRANCH_TRANSACTION: SqlState =
        SqlState("25004");
    pub const NO_ACTIVE_SQL_TRANSACTION_FOR_BRANCH_TRANSACTION: SqlState = SqlState("25005");
    pub const READ_ONLY_SQL_TRANSACTION: SqlState = SqlState("25006");
    pub const SCHEMA_AND_DATA_STATEMENT_MIXING_NOT_SUPPORTED: SqlState = SqlState("25007");
    pub const NO_ACTIVE_SQL_TRANSACTION: SqlState = SqlState("25P01");
    pub const IN_FAILED_SQL_TRANSACTION: SqlState = SqlState("25P02");
    pub const IDLE_IN_TRANSACTION_SESSION_TIMEOUT: SqlState = SqlState("25P03");

    // Class 26 — Invalid SQL Statement Name
    pub const INVALID_SQL_STATEMENT_NAME: SqlState = SqlState("26000");

    // Class 27 — Triggered Data Change Violation
    pub const TRIGGERED_DATA_CHANGE_VIOLATION: SqlState = SqlState("27000");

    // Class 28 — Invalid Authorization Specification
    pub const INVALID_AUTHORIZATION_SPECIFICATION: SqlState = SqlState("28000");
    pub const INVALID_PASSWORD: SqlState = SqlState("28P01");

    // Class 2B — Dependent Privilege Descriptors Still Exist
    pub const DEPENDENT_PRIVILEGE_DESCRIPTORS_STILL_EXIST: SqlState = SqlState("2B000");
    pub const DEPENDENT_OBJECTS_STILL_EXIST: SqlState = SqlState("2BP01");

    // Class 2D — Invalid Transaction Termination
    pub const INVALID_TRANSACTION_TERMINATION: SqlState = SqlState("2D000");

    // Class 2F — SQL Routine Exception
    pub const SQL_ROUTINE_EXCEPTION: SqlState = SqlState("2F000");
    pub const FUNCTION_EXECUTED_NO_RETURN_STATEMENT: SqlState = SqlState("2F005");
    pub const MODIFYING_SQL_DATA_NOT_PERMITTED_SQL: SqlState = SqlState("2F002");
    pub const PROHIBITED_SQL_STATEMENT_ATTEMPTED_SQL: SqlState = SqlState("2F003");
    pub const READING_SQL_DATA_NOT_PERMITTED_SQL: SqlState = SqlState("2F004");

    // Class 34 — Invalid Cursor Name
    pub const INVALID_CURSOR_NAME: SqlState = SqlState("34000");

    // Class 38 — External Routine Exception
    pub const EXTERNAL_ROUTINE_EXCEPTION: SqlState = SqlState("38000");

    // Class 39 — External Routine Invocation Exception
    pub const EXTERNAL_ROUTINE_INVOCATION_EXCEPTION: SqlState = SqlState("39000");

    // Class 3B — Savepoint Exception
    pub const SAVEPOINT_EXCEPTION: SqlState = SqlState("3B000");
    pub const INVALID_SAVEPOINT_SPECIFICATION: SqlState = SqlState("3B001");

    // Class 3D — Invalid Catalog Name
    pub const INVALID_CATALOG_NAME: SqlState = SqlState("3D000");

    // Class 3F — Invalid Schema Name
    pub const INVALID_SCHEMA_NAME: SqlState = SqlState("3F000");

    // Class 40 — Transaction Rollback
    pub const TRANSACTION_ROLLBACK: SqlState = SqlState("40000");
    pub const TRANSACTION_INTEGRITY_CONSTRAINT_VIOLATION: SqlState = SqlState("40002");
    pub const SERIALIZATION_FAILURE: SqlState = SqlState("40001");
    pub const STATEMENT_COMPLETION_UNKNOWN: SqlState = SqlState("40003");
    pub const DEADLOCK_DETECTED: SqlState = SqlState("40P01");

    // Class 42 — Syntax Error or Access Rule Violation
    pub const SYNTAX_ERROR_OR_ACCESS_RULE_VIOLATION: SqlState = SqlState("42000");
    pub const SYNTAX_ERROR: SqlState = SqlState("42601");
    pub const INSUFFICIENT_PRIVILEGE: SqlState = SqlState("42501");
    pub const CANNOT_COERCE: SqlState = SqlState("42846");
    pub const GROUPING_ERROR: SqlState = SqlState("42803");
    pub const WINDOWING_ERROR: SqlState = SqlState("42P20");
    pub const INVALID_RECURSION: SqlState = SqlState("42P19");
    pub const INVALID_FOREIGN_KEY: SqlState = SqlState("42830");
    pub const INVALID_NAME: SqlState = SqlState("42602");
    pub const NAME_TOO_LONG: SqlState = SqlState("42622");
    pub const RESERVED_NAME: SqlState = SqlState("42939");
    pub const DATATYPE_MISMATCH: SqlState = SqlState("42804");
    pub const INDETERMINATE_DATATYPE: SqlState = SqlState("42P18");
    pub const COLLATION_MISMATCH: SqlState = SqlState("42P21");
    pub const INDETERMINATE_COLLATION: SqlState = SqlState("42P22");
    pub const WRONG_OBJECT_TYPE: SqlState = SqlState("42809");
    pub const GENERATED_ALWAYS: SqlState = SqlState("428C9");
    pub const UNDEFINED_COLUMN: SqlState = SqlState("42703");
    pub const UNDEFINED_CURSOR: SqlState = SqlState("34000");
    pub const UNDEFINED_DATABASE: SqlState = SqlState("3D000");
    pub const UNDEFINED_FUNCTION: SqlState = SqlState("42883");
    pub const UNDEFINED_PSTATEMENT: SqlState = SqlState("26000");
    pub const UNDEFINED_SCHEMA: SqlState = SqlState("3F000");
    pub const UNDEFINED_TABLE: SqlState = SqlState("42P01");
    pub const UNDEFINED_PARAMETER: SqlState = SqlState("42P02");
    pub const UNDEFINED_OBJECT: SqlState = SqlState("42704");
    pub const DUPLICATE_COLUMN: SqlState = SqlState("42701");
    pub const DUPLICATE_CURSOR: SqlState = SqlState("42P03");
    pub const DUPLICATE_DATABASE: SqlState = SqlState("42P04");
    pub const DUPLICATE_FUNCTION: SqlState = SqlState("42723");
    pub const DUPLICATE_PSTATEMENT: SqlState = SqlState("42P05");
    pub const DUPLICATE_SCHEMA: SqlState = SqlState("42P06");
    pub const DUPLICATE_TABLE: SqlState = SqlState("42P07");
    pub const DUPLICATE_ALIAS: SqlState = SqlState("42712");
    pub const DUPLICATE_OBJECT: SqlState = SqlState("42710");
    pub const AMBIGUOUS_COLUMN: SqlState = SqlState("42702");
    pub const AMBIGUOUS_FUNCTION: SqlState = SqlState("42725");
    pub const AMBIGUOUS_PARAMETER: SqlState = SqlState("42P08");
    pub const AMBIGUOUS_ALIAS: SqlState = SqlState("42P09");
    pub const INVALID_COLUMN_REFERENCE: SqlState = SqlState("42P10");
    pub const INVALID_COLUMN_DEFINITION: SqlState = SqlState("42611");
    pub const INVALID_CURSOR_DEFINITION: SqlState = SqlState("42P11");
    pub const INVALID_DATABASE_DEFINITION: SqlState = SqlState("42P12");
    pub const INVALID_FUNCTION_DEFINITION: SqlState = SqlState("42P13");
    pub const INVALID_PSTATEMENT_DEFINITION: SqlState = SqlState("42P14");
    pub const INVALID_SCHEMA_DEFINITION: SqlState = SqlState("42P15");
    pub const INVALID_TABLE_DEFINITION: SqlState = SqlState("42P16");
    pub const INVALID_OBJECT_DEFINITION: SqlState = SqlState("42P17");

    // Class 44 — WITH CHECK OPTION Violation
    pub const WITH_CHECK_OPTION_VIOLATION: SqlState = SqlState("44000");

    // Class 53 — Insufficient Resources
    pub const INSUFFICIENT_RESOURCES: SqlState = SqlState("53000");
    pub const DISK_FULL: SqlState = SqlState("53100");
    pub const OUT_OF_MEMORY: SqlState = SqlState("53200");
    pub const TOO_MANY_CONNECTIONS: SqlState = SqlState("53300");
    pub const CONFIGURATION_LIMIT_EXCEEDED: SqlState = SqlState("53400");

    // Class 54 — Program Limit Exceeded
    pub const PROGRAM_LIMIT_EXCEEDED: SqlState = SqlState("54000");
    pub const STATEMENT_TOO_COMPLEX: SqlState = SqlState("54001");
    pub const TOO_MANY_COLUMNS: SqlState = SqlState("54011");
    pub const TOO_MANY_ARGUMENTS: SqlState = SqlState("54023");

    // Class 55 — Object Not In Prerequisite State
    pub const OBJECT_NOT_IN_PREREQUISITE_STATE: SqlState = SqlState("55000");
    pub const OBJECT_IN_USE: SqlState = SqlState("55006");
    pub const CANT_CHANGE_RUNTIME_PARAM: SqlState = SqlState("55P02");
    pub const LOCK_NOT_AVAILABLE: SqlState = SqlState("55P03");
    pub const UNSAFE_NEW_ENUM_VALUE_USAGE: SqlState = SqlState("55P04");

    // Class 57 — Operator Intervention
    pub const OPERATOR_INTERVENTION: SqlState = SqlState("57000");
    pub const QUERY_CANCELED: SqlState = SqlState("57014");
    pub const ADMIN_SHUTDOWN: SqlState = SqlState("57P01");
    pub const CRASH_SHUTDOWN: SqlState = SqlState("57P02");
    pub const CANNOT_CONNECT_NOW: SqlState = SqlState("57P03");
    pub const DATABASE_DROPPED: SqlState = SqlState("57P04");
    pub const IDLE_SESSION_TIMEOUT: SqlState = SqlState("57P05");

    // Class 58 — System Error
    pub const SYSTEM_ERROR: SqlState = SqlState("58000");
    pub const IO_ERROR: SqlState = SqlState("58030");
    pub const UNDEFINED_FILE: SqlState = SqlState("58P01");
    pub const DUPLICATE_FILE: SqlState = SqlState("58P02");

    // Class 72 — Snapshot Failure
    pub const SNAPSHOT_TOO_OLD: SqlState = SqlState("72000");

    // Class F0 — Configuration File Error
    pub const CONFIG_FILE_ERROR: SqlState = SqlState("F0000");
    pub const LOCK_FILE_EXISTS: SqlState = SqlState("F0001");

    // Class HV — Foreign Data Wrapper Error (SQL/MED)
    pub const FDW_ERROR: SqlState = SqlState("HV000");

    // Class P0 — PL/pgSQL Error
    pub const PLPGSQL_ERROR: SqlState = SqlState("P0000");
    pub const RAISE_EXCEPTION: SqlState = SqlState("P0001");
    pub const NO_DATA_FOUND: SqlState = SqlState("P0002");
    pub const TOO_MANY_ROWS: SqlState = SqlState("P0003");
    pub const ASSERT_FAILURE: SqlState = SqlState("P0004");

    // Class XX — Internal Error
    pub const INTERNAL_ERROR: SqlState = SqlState("XX000");
    pub const DATA_CORRUPTED: SqlState = SqlState("XX001");
    pub const INDEX_CORRUPTED: SqlState = SqlState("XX002");
}

/// Severity levels as PostgreSQL sends them
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Error,
    Fatal,
    Panic,
    Warning,
    Notice,
    Debug,
    Info,
    Log,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Error => "ERROR",
            Severity::Fatal => "FATAL",
            Severity::Panic => "PANIC",
            Severity::Warning => "WARNING",
            Severity::Notice => "NOTICE",
            Severity::Debug => "DEBUG",
            Severity::Info => "INFO",
            Severity::Log => "LOG",
        }
    }
}

/// A full PostgreSQL error with all wire-protocol fields.
#[derive(Debug, Clone)]
pub struct PgError {
    pub severity: Severity,
    pub sqlstate: SqlState,
    pub message: String,
    pub detail: Option<String>,
    pub hint: Option<String>,
    pub position: Option<u32>,
    pub internal_position: Option<u32>,
    pub internal_query: Option<String>,
    pub where_context: Option<String>,
    pub schema_name: Option<String>,
    pub table_name: Option<String>,
    pub column_name: Option<String>,
    pub data_type_name: Option<String>,
    pub constraint_name: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub routine: Option<String>,
}

impl PgError {
    pub fn new(severity: Severity, sqlstate: SqlState, message: impl Into<String>) -> Self {
        Self {
            severity,
            sqlstate,
            message: message.into(),
            detail: None,
            hint: None,
            position: None,
            internal_position: None,
            internal_query: None,
            where_context: None,
            schema_name: None,
            table_name: None,
            column_name: None,
            data_type_name: None,
            constraint_name: None,
            file: None,
            line: None,
            routine: None,
        }
    }

    pub fn error(sqlstate: SqlState, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, sqlstate, message)
    }

    pub fn fatal(sqlstate: SqlState, message: impl Into<String>) -> Self {
        Self::new(Severity::Fatal, sqlstate, message)
    }

    pub fn syntax_error(message: impl Into<String>) -> Self {
        Self::error(SqlState::SYNTAX_ERROR, message)
    }

    pub fn undefined_table(name: &str) -> Self {
        let mut e = Self::error(
            SqlState::UNDEFINED_TABLE,
            format!("relation \"{name}\" does not exist"),
        );
        e.table_name = Some(name.to_string());
        e
    }

    pub fn undefined_column(name: &str) -> Self {
        let mut e = Self::error(
            SqlState::UNDEFINED_COLUMN,
            format!("column \"{name}\" does not exist"),
        );
        e.column_name = Some(name.to_string());
        e
    }

    pub fn duplicate_table(name: &str) -> Self {
        let mut e = Self::error(
            SqlState::DUPLICATE_TABLE,
            format!("relation \"{name}\" already exists"),
        );
        e.table_name = Some(name.to_string());
        e
    }

    pub fn unique_violation(
        table: &str,
        constraint: &str,
        detail: impl Into<String>,
    ) -> Self {
        let mut e = Self::error(
            SqlState::UNIQUE_VIOLATION,
            format!(
                "duplicate key value violates unique constraint \"{constraint}\""
            ),
        );
        e.table_name = Some(table.to_string());
        e.constraint_name = Some(constraint.to_string());
        e.detail = Some(detail.into());
        e
    }

    pub fn not_null_violation(table: &str, column: &str) -> Self {
        let mut e = Self::error(
            SqlState::NOT_NULL_VIOLATION,
            format!("null value in column \"{column}\" of relation \"{table}\" violates not-null constraint"),
        );
        e.table_name = Some(table.to_string());
        e.column_name = Some(column.to_string());
        e.detail = Some(format!(
            "Failing row contains a null value in column {column}."
        ));
        e
    }

    pub fn foreign_key_violation(
        table: &str,
        constraint: &str,
        detail: impl Into<String>,
    ) -> Self {
        let mut e = Self::error(
            SqlState::FOREIGN_KEY_VIOLATION,
            format!(
                "insert or update on table \"{table}\" violates foreign key constraint \"{constraint}\""
            ),
        );
        e.table_name = Some(table.to_string());
        e.constraint_name = Some(constraint.to_string());
        e.detail = Some(detail.into());
        e
    }

    pub fn check_violation(table: &str, constraint: &str) -> Self {
        let mut e = Self::error(
            SqlState::CHECK_VIOLATION,
            format!(
                "new row for relation \"{table}\" violates check constraint \"{constraint}\""
            ),
        );
        e.table_name = Some(table.to_string());
        e.constraint_name = Some(constraint.to_string());
        e
    }

    pub fn division_by_zero() -> Self {
        Self::error(SqlState::DIVISION_BY_ZERO, "division by zero")
    }

    pub fn invalid_text_representation(typ: &str, value: &str) -> Self {
        Self::error(
            SqlState::INVALID_TEXT_REPRESENTATION,
            format!("invalid input syntax for type {typ}: \"{value}\""),
        )
    }

    pub fn feature_not_supported(feature: &str) -> Self {
        Self::error(
            SqlState::FEATURE_NOT_SUPPORTED,
            format!("{feature} is not supported"),
        )
    }

    pub fn too_many_connections(max: usize) -> Self {
        Self::fatal(
            SqlState::TOO_MANY_CONNECTIONS,
            format!("sorry, too many clients already (max {max})"),
        )
    }

    pub fn serialization_failure() -> Self {
        Self::error(
            SqlState::SERIALIZATION_FAILURE,
            "could not serialize access due to concurrent update",
        )
    }

    pub fn invalid_password(user: &str) -> Self {
        Self::fatal(
            SqlState::INVALID_PASSWORD,
            format!("password authentication failed for user \"{user}\""),
        )
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_position(mut self, position: u32) -> Self {
        self.position = Some(position);
        self
    }

    pub fn with_schema(mut self, schema: impl Into<String>) -> Self {
        self.schema_name = Some(schema.into());
        self
    }

    pub fn with_table(mut self, table: impl Into<String>) -> Self {
        self.table_name = Some(table.into());
        self
    }

    pub fn with_column(mut self, column: impl Into<String>) -> Self {
        self.column_name = Some(column.into());
        self
    }

    pub fn with_constraint(mut self, constraint: impl Into<String>) -> Self {
        self.constraint_name = Some(constraint.into());
        self
    }
}

impl std::fmt::Display for PgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {} (SQLSTATE {})",
            self.severity.as_str(),
            self.message,
            self.sqlstate.0
        )
    }
}

impl std::error::Error for PgError {}

/// The main error type for the cave-pg engine.
#[derive(Error, Debug)]
pub enum Error {
    #[error("PostgreSQL error: {0}")]
    Pg(#[from] PgError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Type error: {0}")]
    Type(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Server shutting down")]
    Shutdown,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Convert to a PgError for wire-protocol transmission.
    pub fn to_pg_error(&self) -> PgError {
        match self {
            Error::Pg(e) => e.clone(),
            Error::Io(e) => PgError::fatal(SqlState::IO_ERROR, e.to_string()),
            Error::Parse(msg) => PgError::error(SqlState::SYNTAX_ERROR, msg.clone()),
            Error::Protocol(msg) => {
                PgError::fatal(SqlState::PROTOCOL_VIOLATION, msg.clone())
            }
            Error::Type(msg) => PgError::error(SqlState::DATATYPE_MISMATCH, msg.clone()),
            Error::Serialization(msg) => {
                PgError::error(SqlState::INTERNAL_ERROR, msg.clone())
            }
            Error::ConnectionClosed => {
                PgError::fatal(SqlState::CONNECTION_FAILURE, "connection closed")
            }
            Error::Shutdown => {
                PgError::fatal(SqlState::ADMIN_SHUTDOWN, "server is shutting down")
            }
        }
    }
}

/// Create an Error::Pg from a quick string.
pub fn pg_error(state: SqlState, msg: impl Into<String>) -> Error {
    Error::Pg(PgError::error(state, msg))
}
