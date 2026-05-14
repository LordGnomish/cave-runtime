# cave-streams — Upstream Test Port Batch 4 (2026-05-14)

## Summary
Closes the `connect/runtime/rest/` admin API surface for Kafka Connect.
A new `src/connect_rest.rs` (526 LOC) implements a pure-function
state machine (typed `(status, body)` tuples) that mirrors upstream
Jersey resource semantics without dragging in axum/jetty.

Adds 16 line-by-line ports of `ConnectorsResourceTest` /
`RootResourceTest` + 3 module-internal helper tests.

## Commits (TDD strict — RED → GREEN → REFACTOR)
- `219ee35e` — test(cave-streams): batch4 RED — Connect REST admin API (16 failing tests)
- `f165b36c` — feat(cave-streams): batch4 GREEN — Connect REST admin state machine
- `82b3b840` — chore(cave-streams): batch4 REFACTOR — manifest update + ratio bump
  (post-commit hook auto-amended to include regenerated `docs/parity/parity-index.json`)

## Coverage (apache/kafka@4.2.0)
`connect/runtime/src/test/java/org/apache/kafka/connect/runtime/rest/{ConnectorsResourceTest,RootResourceTest}.java` + corresponding main sources.

| Test | Asserts |
|---|---|
| `upstream_rest_list_connectors_returns_alphabetical_names` | `GET /connectors` → 200 + names in alphabetical order. |
| `upstream_rest_create_connector_returns_201_with_location` | `POST /connectors` 201 + `Location: /connectors/<name>` header. |
| `upstream_rest_create_connector_duplicate_returns_409_conflict` | Same name twice → 409 with `RebalanceNeededException`-shaped envelope. |
| `upstream_rest_create_connector_trims_padding_whitespace_from_name` | Leading/trailing whitespace stripped before storage. |
| `upstream_rest_create_connector_all_whitespace_name_normalises_to_empty` | All-whitespace → empty after trim → 400 InvalidName. |
| `upstream_rest_create_connector_null_name_normalises_to_empty` | Null name → 400 InvalidName. |
| `upstream_rest_create_connector_config_name_mismatch_returns_400` | `name="foo"`, `config["name"]="bar"` → 400 (mismatch). |
| `upstream_rest_restart_connector_not_found_returns_404` | `POST /connectors/missing/restart` → 404. |
| `upstream_rest_restart_connector_and_tasks_returns_202_accepted_with_state_info` | `?includeTasks=true&onlyFailed=true` → 202 + `RestartReport`. |
| `upstream_rest_pause_connector_idempotent_returns_unit_or_404` | Pause twice → second is no-op (202). |
| `upstream_rest_resume_connector_after_pause_resumes_running` | Resume after pause → status flips to RUNNING. |
| `upstream_rest_delete_connector_returns_204_or_404` | Delete present → 204; delete missing → 404. |
| `upstream_rest_validate_config_happy_path_reports_zero_errors` | Valid config → 200 with `error_count: 0`. |
| `upstream_rest_validate_config_reports_missing_required_keys` | Required key absent → `ConfigInfo.validation_errors` populated. |
| `upstream_rest_connector_status_present_or_none` | Status query for known/unknown connector. |
| `upstream_rest_root_server_info_exposes_kafka_cluster_id` | `GET /` returns `version`, `kafka_cluster_id`, `commit`. |

## State machine: `src/connect_rest.rs`
```rust
pub struct ConnectRestAdmin { /* connectors BTreeMap, server_info, ... */ }
pub enum HttpStatus { Ok = 200, Created = 201, Accepted = 202, NoContent = 204,
                      BadRequest = 400, NotFound = 404, Conflict = 409 }

impl ConnectRestAdmin {
    pub fn list_connectors(&self) -> Vec<String>;
    pub fn create_connector(&mut self, name: &str, config: BTreeMap<String,String>) -> Result<CreatedConnector, CreateError>;
    pub fn restart_connector(&mut self, name: &str, include_tasks: bool, only_failed: bool) -> Result<RestartReport, AdminError>;
    pub fn pause_connector(&mut self, name: &str) -> Result<(), AdminError>;
    pub fn resume_connector(&mut self, name: &str) -> Result<(), AdminError>;
    pub fn delete_connector(&mut self, name: &str) -> Result<(), AdminError>;
    pub fn validate_config(&self, plugin_class: &str, config: BTreeMap<String,String>) -> ConfigValidationResult;
    pub fn connector_status(&self, name: &str) -> Option<ConnectorStatus>;
    pub fn root_server_info(&self) -> RootServerInfo;
}

pub enum CreateError { Conflict, InvalidName, NameMismatch, InvalidConfig(Vec<ConfigError>) }
impl CreateError { pub fn status(&self) -> HttpStatus { ... } }
```

Each error carries `.status()` so callers can lift a typed result into
any HTTP framework. The axum mount itself is intentionally out of
scope — pure dispatch tests cover the contract.

## Parity manifest

| Field | Before | After |
|---|---|---|
| `mapped_count` | 21 | **22** |
| `skipped_count` | 16 | 16 |
| `partial_count` | 0 | 0 |
| `unmapped_count` | 7 | **6** |
| `total` | 44 | **45** |
| **`fill_ratio`** | **0.8409** | **0.8444** |
| `honest_ratio` | 0.8409 | **0.8444** |
| `last_audit` | 2026-05-13 | **2026-05-14** |

New `[[mapped]] upstream_pkg = "connect/runtime/rest/"` → `local_files = ["src/connect_rest.rs"]`.

## Honest deferrals
- `RestServerTest.java` at apache/kafka 4.2.0 raw path 404s (file was reorganised between 4.1 and 4.2). The Jersey/Jetty advertised-URL formation + `RestExceptionMapper` JSON envelope mapping are HTTP-server framing concerns, not REST semantics — recorded as 2 `status="missing"` entries with notes. The state-machine surface (status codes, name normalisation, location header, body DTOs) is fully covered.
- Real axum mount + JWT bypass list + tenant-scoped routing — deferred to the cave-portal integration when connectors get an admin UI page.

## Stubs in new code
`src/connect_rest.rs` contains **0** of: `unimplemented!`, `todo!`, `#[ignore]`, `panic!("not implemented`.
