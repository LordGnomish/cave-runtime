# cave-profiler — TDD coverage gap report

| field | value |
|---|---|
| crate | cave-profiler (theme: observability) |
| upstream | grafana/pyroscope |
| version | v1.3.0 (Go) |
| upstream test-symbol count | 921 (across 264 `*_test.go` files) |
| cave behavior test-fn count | 5 (engine.rs); +4 generic proptest_smoke scaffold (non-behavioral) |

## Scope framing

cave-profiler is a **minimal profile-aggregation surface**, not a port of the Pyroscope
backend. Its `src/` implements four pure functions over an in-memory `ProfileSession` /
`StackFrame` model (`top_functions`, `session_duration_secs`, `samples_per_second`,
`find_hotspot`), a five-variant `ProfileType` enum, and a `/api/profiler/health` route.

The overwhelming majority of the 921 upstream test symbols exercise subsystems that are
deliberately **out of cave's launch scope**: eBPF agents (`ebpf/**`), ELF/Go symbolization
(`ebpf/symtab/**`, `gosym`), the distributor / ingester / compactor / query-frontend /
metastore storage engine (`pkg/{distributor,ingester,compactor,querier,phlaredb,...}`),
pprof wire-format parsing, and the v1 ("og") flameql/transporttrie stack. These are marked
`scope-cut` collectively below rather than enumerated line-by-line.

The honest mappable overlap is the **profile aggregation / tree-reduction** family
(top-N functions, hotspot/self-vs-total, sample-rate, stacktrace merge) plus the
**profile-type / language model**.

## Gap table

| Behavioral unit | Upstream test | cave impl? | cave test? | gap type | suggested cave test name |
|---|---|---|---|---|---|
| Sample-rate = samples / duration | `pkg/...` (rate/Counter family, `TestCounter`) | yes — `samples_per_second` (engine.rs:18) | no | portable-coverage | `test_samples_per_second_basic` |
| Sample-rate divide-by-zero guard (duration == 0) | analogous to upstream zero-window handling | yes — `samples_per_second` d==0 branch (engine.rs:21) | no | portable-coverage | `test_samples_per_second_zero_duration` |
| Completed-session duration (ended_at = Some) | duration/interval iterator family (`Test_TimeIntervalIterator_*`) | yes — `session_duration_secs` Some branch (engine.rs:12) | only None branch tested | portable-coverage | `test_session_duration_secs_completed` |
| Top-N descending self-weight reduction | `pkg/model/tree_test.go::Test_Tree`, `Test_Tree_minValue` | yes — `top_functions` (engine.rs:5) | partial (sorted, fewer-than-n) — no tie/equal-weight case | portable-coverage | `test_top_functions_equal_weight_stable` |
| Hotspot = max self-weight frame | `pkg/model/tree_test.go::Test_Tree` (max node) | yes — `find_hotspot` (engine.rs:28) | yes (`test_find_hotspot`, `_empty`) | covered | — |
| ProfileType serde (snake_case round-trip, 5 variants) | `pkg/distributor/model/push_test.go::TestProfileSeries_GetLanguage` (type/lang mapping) | yes — `ProfileType` enum (models.rs:18, `#[serde(rename_all="snake_case")]`) | no | portable-coverage | `test_profile_type_serde_roundtrip` |
| ProfileSession JSON serialization stability | `pkg/model/profile_test.go` | yes — `ProfileSession` derives Serialize/Deserialize (models.rs:7) | no | portable-coverage | `test_profile_session_serde_roundtrip` |
| Stacktrace / tree merge (combine two profiles) | `pkg/model/stacktraces_test.go::TestStackTraceMerger`, `tree_test.go::Test_TreeMerge`, `flamegraph_test.go::Test_FlameGraphMerger` | no — cave has no merge fn | — | missing-impl | _(needs `merge_sessions` impl first — not portable)_ |
| Flamegraph diff (A vs B tree delta) | `pkg/model/flamegraph_diff_test.go::Test_Diff_Tree*` (5 cases) | no | — | missing-impl | _(no diff impl)_ |
| MaxNodes / truncation of tree | `pkg/model/tree_test.go::Test_Tree_minValue`, `validation::TestValidateFlamegraphMaxNodes` | no | — | missing-impl | _(no node-cap impl)_ |
| Health endpoint returns module/status JSON | (none upstream; cave-specific) | yes — `health()` (routes.rs:15) | no | portable-coverage | `test_health_route_returns_ok` |
| pprof wire-format parse / fixtures | `pkg/og/convert/pprof/profile_test.go::TestEmptyPPROF`, `TestIngestPPROFFixtures` | no | — | scope-cut | _wire-format ingestion is out of launch scope_ |
| eBPF agent / Python/Go symbolization | `ebpf/**` (~hundreds of symbols) | no | — | scope-cut | _kernel eBPF + native symbolization not in scope_ |
| Distributor/ingester/compactor/querier storage | `pkg/{distributor,ingester,compactor,querier,phlaredb}/**` | no | — | scope-cut | _distributed storage engine not in scope_ |
| Query-frontend select-merge planning | `pkg/...::TestSplitAndMergePlanner_Plan`, `TestSelectMergeStacktraces` | no | — | scope-cut | _query planner/sharding not in scope_ |

## Recommended TDD fills (portable-coverage first)

These exercise behavior cave **already implements** but does not test — cheapest, highest value:

1. **`test_samples_per_second_basic`** — `engine::samples_per_second`. Build a session with
   known `samples` and a `Some(ended_at)` such that duration is e.g. 10s; assert rate equals
   `samples / 10.0`.
2. **`test_samples_per_second_zero_duration`** — `engine::samples_per_second`. Session whose
   `ended_at == started_at` (duration 0); assert it returns `Some(0.0)` and does **not** panic
   (guards engine.rs:21).
3. **`test_session_duration_secs_completed`** — `engine::session_duration_secs`. Session with
   `ended_at = Some(started + 30s)`; assert `Some(30)`. (Current suite only covers the `None`
   running branch.)
4. **`test_profile_type_serde_roundtrip`** — `models::ProfileType`. Serialize each of the five
   variants and assert snake_case strings (`"cpu"`, `"memory"`, `"goroutine"`, `"mutex"`,
   `"block"`) round-trip back via `serde_json`.
5. **`test_profile_session_serde_roundtrip`** — `models::ProfileSession`. Round-trip a populated
   session (with frames) through `serde_json` and assert `PartialEq` equality.
6. **`test_top_functions_equal_weight_stable`** — `engine::top_functions`. Frames with equal
   `self_samples`; assert `n` are returned and the call is total/stable (no panic, correct count).
7. **`test_health_route_returns_ok`** — `routes::health`. Call the handler (or via router) and
   assert the JSON contains `status: "ok"` and `module: "cave-profiler"`.

**Not portable (would need new impl — do NOT stub):** stacktrace/tree **merge**, flamegraph
**diff**, and tree **MaxNodes/truncation**. Upstream tests these heavily but cave has no
corresponding function; writing tests first would require real implementations and is a feature
decision, not a coverage fill.

**Scope-cut (legitimately skipped):** eBPF agents, ELF/Go/Python symbolization, pprof
wire-format ingestion, and the distributor/ingester/compactor/querier/metastore storage and
query-frontend planning subsystems — none are part of cave-profiler's aggregation-surface scope.
