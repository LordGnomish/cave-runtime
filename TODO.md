# TODO

## Phase 5 — Portal backend hookup

`portal/src/pages/LocalLLMDaemon.tsx` currently renders dummy data.
Phase 5 needs:

- `GET /api/local-llm/queue` — serve `docs/BUILD-PLAN-TIER1.yaml` as JSON
  (add route in `crates/cave-local-llm` or `crates/cave-portal`)
- `GET /api/local-llm/metrics` — expose Prometheus text or JSON summary
- Wire `useSWR` / React Query calls in place of the `useState(DUMMY_*)` placeholders

## File-lock primitive

`crates/cave-local-llm/src/queue.rs` uses `fs2 = "0.4"` for `flock(2)`
exclusive locking.  If cave-core ever grows a shared file-lock helper, replace
the inline `fs2` usage with the cave-core primitive and remove the direct dep.
