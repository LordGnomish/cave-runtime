// LocalLLMDaemon.tsx — multi-repo queue view with tab navigation
//
// TODO (Phase 5): wire up real backend endpoints:
//   GET /api/local-llm/queue   → queue items JSON (replaces DUMMY_QUEUE)
//   GET /api/local-llm/metrics → Prometheus text or JSON (replaces DUMMY_METRICS)

import React, { useState } from "react";

// ── Types ─────────────────────────────────────────────────────────────────────

type Repo = "cave-runtime" | "pipeline-platform";

interface QueueItem {
  id: string;
  crate_name: string;
  upstream_repo: string;
  upstream_fn: string;
  status: "pending" | "in_progress" | "done" | "stuck";
  attempts: number;
  last_error: string | null;
  priority: number;
  updated_at: string;
  repo: Repo;
}

interface DaemonMetricsSummary {
  daemon_ticks_total: number;
  tier1_commits_total: number;
  tier2_escalations_total: number;
  queue_pending: number;
  queue_in_progress: number;
  queue_done: number;
  queue_stuck: number;
}

// ── Dummy data (replaced by API calls in Phase 5) ────────────────────────────

const DUMMY_QUEUE: QueueItem[] = [
  {
    id: "00000000-0000-0000-0000-000000000001",
    crate_name: "cave-secrets",
    upstream_repo: "trufflesecurity/trufflehog",
    upstream_fn: "FromData",
    status: "done",
    attempts: 1,
    last_error: null,
    priority: 1,
    updated_at: "2026-04-22T08:15:00Z",
    repo: "cave-runtime",
  },
  {
    id: "00000000-0000-0000-0000-000000000002",
    crate_name: "cave-auth",
    upstream_repo: "trufflesecurity/trufflehog",
    upstream_fn: "FromData",
    status: "in_progress",
    attempts: 2,
    last_error: null,
    priority: 2,
    updated_at: "2026-04-22T09:01:00Z",
    repo: "cave-runtime",
  },
  {
    id: "00000000-0000-0000-0000-000000000003",
    crate_name: "cave-events",
    upstream_repo: "etcd-io/etcd",
    upstream_fn: "Watch",
    status: "stuck",
    attempts: 3,
    last_error: "test_fail: cargo test -p cave-events exited Some(1)",
    priority: 3,
    updated_at: "2026-04-22T07:45:00Z",
    repo: "cave-runtime",
  },
  {
    id: "00000000-0000-0000-0000-000000000004",
    crate_name: "pipeline-ingest",
    upstream_repo: "apache/kafka",
    upstream_fn: "Produce",
    status: "pending",
    attempts: 0,
    last_error: null,
    priority: 1,
    updated_at: "2026-04-22T09:30:00Z",
    repo: "pipeline-platform",
  },
  {
    id: "00000000-0000-0000-0000-000000000005",
    crate_name: "pipeline-transform",
    upstream_repo: "apache/flink",
    upstream_fn: "ProcessElement",
    status: "pending",
    attempts: 0,
    last_error: null,
    priority: 2,
    updated_at: "2026-04-22T09:31:00Z",
    repo: "pipeline-platform",
  },
];

const DUMMY_METRICS: DaemonMetricsSummary = {
  daemon_ticks_total: 42,
  tier1_commits_total: 8,
  tier2_escalations_total: 3,
  queue_pending: 2,
  queue_in_progress: 1,
  queue_done: 1,
  queue_stuck: 1,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

const STATUS_COLOR: Record<QueueItem["status"], string> = {
  pending: "#6b7280",
  in_progress: "#2563eb",
  done: "#16a34a",
  stuck: "#dc2626",
};

function StatusBadge({ status }: { status: QueueItem["status"] }) {
  return (
    <span
      style={{
        padding: "2px 8px",
        borderRadius: 12,
        fontSize: 12,
        fontWeight: 600,
        background: STATUS_COLOR[status] + "22",
        color: STATUS_COLOR[status],
        border: `1px solid ${STATUS_COLOR[status]}44`,
      }}
    >
      {status}
    </span>
  );
}

function MetricCard({
  label,
  value,
  color = "#2563eb",
}: {
  label: string;
  value: number;
  color?: string;
}) {
  return (
    <div
      style={{
        padding: "16px 20px",
        border: "1px solid #e5e7eb",
        borderRadius: 8,
        minWidth: 120,
        textAlign: "center",
        background: "#fff",
      }}
    >
      <div style={{ fontSize: 28, fontWeight: 700, color }}>{value}</div>
      <div style={{ fontSize: 12, color: "#6b7280", marginTop: 4 }}>{label}</div>
    </div>
  );
}

type RepoTab = "all" | Repo;

const REPO_TABS: { id: RepoTab; label: string }[] = [
  { id: "all", label: "All" },
  { id: "cave-runtime", label: "Cave Runtime" },
  { id: "pipeline-platform", label: "Pipeline Platform" },
];

// ── Page ──────────────────────────────────────────────────────────────────────

export function LocalLLMDaemonPage() {
  // TODO (Phase 5): replace useState initializers with useSWR / React Query
  const [queue] = useState<QueueItem[]>(DUMMY_QUEUE);
  const [metrics] = useState<DaemonMetricsSummary>(DUMMY_METRICS);
  const [repoTab, setRepoTab] = useState<RepoTab>("all");
  const [statusFilter, setStatusFilter] = useState<string>("all");

  const repoFiltered =
    repoTab === "all" ? queue : queue.filter((i) => i.repo === repoTab);

  const filtered =
    statusFilter === "all"
      ? repoFiltered
      : repoFiltered.filter((i) => i.status === statusFilter);

  return (
    <div style={{ fontFamily: "system-ui, sans-serif", padding: 24, maxWidth: 1100 }}>
      <h1 style={{ fontSize: 22, fontWeight: 700, marginBottom: 4 }}>
        cave-local-llm Daemon
      </h1>
      <p style={{ color: "#6b7280", fontSize: 14, marginBottom: 24 }}>
        24/7 Qwen amele scheduler — tier-1 draft generation from parity manifests (runtime 70% / pipeline 30%).
        {/* TODO (Phase 5): show live daemon status from /api/local-llm/status */}
      </p>

      {/* ── Metrics cards ──────────────────────────────────────────────── */}
      <section style={{ marginBottom: 32 }}>
        <h2 style={{ fontSize: 15, fontWeight: 600, marginBottom: 12 }}>Metrics</h2>
        <div style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
          <MetricCard label="Ticks" value={metrics.daemon_ticks_total} color="#7c3aed" />
          <MetricCard label="Tier-1 Commits" value={metrics.tier1_commits_total} color="#16a34a" />
          <MetricCard label="Escalations" value={metrics.tier2_escalations_total} color="#dc2626" />
          <MetricCard label="Pending" value={metrics.queue_pending} color="#6b7280" />
          <MetricCard label="In Progress" value={metrics.queue_in_progress} color="#2563eb" />
          <MetricCard label="Done" value={metrics.queue_done} color="#16a34a" />
          <MetricCard label="Stuck" value={metrics.queue_stuck} color="#dc2626" />
        </div>
      </section>

      {/* ── Repo tabs ───────────────────────────────────────────────────── */}
      <div
        style={{
          display: "flex",
          gap: 0,
          borderBottom: "1px solid #e5e7eb",
          marginBottom: 16,
        }}
      >
        {REPO_TABS.map((tab) => {
          const count =
            tab.id === "all"
              ? queue.length
              : queue.filter((i) => i.repo === tab.id).length;
          const active = repoTab === tab.id;
          return (
            <button
              key={tab.id}
              onClick={() => setRepoTab(tab.id)}
              style={{
                padding: "8px 20px",
                fontSize: 14,
                fontWeight: active ? 600 : 400,
                background: "none",
                border: "none",
                borderBottom: active ? "2px solid #2563eb" : "2px solid transparent",
                color: active ? "#2563eb" : "#6b7280",
                cursor: "pointer",
                marginBottom: -1,
              }}
            >
              {tab.label}
              <span
                style={{
                  marginLeft: 6,
                  fontSize: 11,
                  fontWeight: 600,
                  padding: "1px 6px",
                  borderRadius: 10,
                  background: active ? "#dbeafe" : "#f3f4f6",
                  color: active ? "#2563eb" : "#6b7280",
                }}
              >
                {count}
              </span>
            </button>
          );
        })}
      </div>

      {/* ── Queue table ─────────────────────────────────────────────────── */}
      <section>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 12,
            marginBottom: 12,
          }}
        >
          <h2 style={{ fontSize: 15, fontWeight: 600, margin: 0 }}>
            Queue ({filtered.length})
          </h2>
          <select
            value={statusFilter}
            onChange={(e) => setStatusFilter(e.target.value)}
            style={{
              fontSize: 13,
              padding: "3px 8px",
              borderRadius: 6,
              border: "1px solid #d1d5db",
            }}
          >
            <option value="all">All statuses</option>
            <option value="pending">Pending</option>
            <option value="in_progress">In Progress</option>
            <option value="done">Done</option>
            <option value="stuck">Stuck</option>
          </select>
        </div>

        <table
          style={{
            width: "100%",
            borderCollapse: "collapse",
            fontSize: 13,
            background: "#fff",
            border: "1px solid #e5e7eb",
            borderRadius: 8,
            overflow: "hidden",
          }}
        >
          <thead>
            <tr style={{ background: "#f9fafb" }}>
              {[
                "Crate",
                "Upstream",
                "Function",
                "Status",
                "Attempts",
                "Last Error",
                "Updated",
              ].map((h) => (
                <th
                  key={h}
                  style={{
                    padding: "10px 14px",
                    textAlign: "left",
                    fontWeight: 600,
                    color: "#374151",
                    borderBottom: "1px solid #e5e7eb",
                  }}
                >
                  {h}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {filtered.map((item) => (
              <tr key={item.id} style={{ borderBottom: "1px solid #f3f4f6" }}>
                <td style={{ padding: "10px 14px", fontWeight: 600 }}>
                  {item.crate_name}
                </td>
                <td style={{ padding: "10px 14px", color: "#6b7280" }}>
                  {item.upstream_repo}
                </td>
                <td style={{ padding: "10px 14px", fontFamily: "monospace" }}>
                  {item.upstream_fn}
                </td>
                <td style={{ padding: "10px 14px" }}>
                  <StatusBadge status={item.status} />
                </td>
                <td style={{ padding: "10px 14px", textAlign: "center" }}>
                  {item.attempts}
                </td>
                <td
                  style={{
                    padding: "10px 14px",
                    color: "#dc2626",
                    fontSize: 12,
                    maxWidth: 220,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                >
                  {item.last_error ?? "—"}
                </td>
                <td
                  style={{ padding: "10px 14px", color: "#6b7280", fontSize: 12 }}
                >
                  {new Date(item.updated_at).toLocaleString()}
                </td>
              </tr>
            ))}
            {filtered.length === 0 && (
              <tr>
                <td
                  colSpan={7}
                  style={{
                    padding: "20px 14px",
                    textAlign: "center",
                    color: "#9ca3af",
                  }}
                >
                  No items matching filter.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </section>
    </div>
  );
}

export default LocalLLMDaemonPage;
