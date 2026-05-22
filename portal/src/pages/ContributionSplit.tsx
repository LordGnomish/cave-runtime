// ContributionSplit.tsx — RACI contribution attribution widget
//
// Shows commits / net-LOC / files-touched broken down by:
//   Qwen3 (amele, [qwen-amele] prefix or Co-Authored-By: Qwen trailer)
//   Sonnet (Co-Authored-By: Claude, author != Burak)
//   Burak  (human author, no AI trailer)
//   Other  (external contributor, rare)
//
// TODO (Phase 5): replace dummy data with useSWR calling GET /api/attribution

import React, { useState, useEffect, useCallback } from "react";

// ── Types ─────────────────────────────────────────────────────────────────────

type Metric = "commits" | "loc_net" | "files";
type Period = "1d" | "7d";
type Repo = "all" | "cave-runtime" | "pipeline-platform";

interface AuthorBreakdown {
  qwen3: number;
  sonnet: number;
  burak: number;
  other: number;
}

interface AttributionResponse {
  period_days: number;
  repo: string;
  by_commits: AuthorBreakdown;
  by_loc_net: AuthorBreakdown;
  by_files: AuthorBreakdown;
  timestamps: { earliest?: string; latest?: string };
}

// ── Dummy data ────────────────────────────────────────────────────────────────

const DUMMY: AttributionResponse = {
  period_days: 7,
  repo: "all",
  by_commits: { qwen3: 42, sonnet: 18, burak: 5, other: 0 },
  by_loc_net: { qwen3: 6240, sonnet: 1120, burak: 340, other: 0 },
  by_files: { qwen3: 87, sonnet: 34, burak: 12, other: 0 },
  timestamps: {
    earliest: "2026-04-16T00:00:00Z",
    latest: "2026-04-22T09:31:00Z",
  },
};

// ── Helpers ───────────────────────────────────────────────────────────────────

const AUTHOR_COLORS: Record<string, string> = {
  qwen3: "#7c3aed",
  sonnet: "#2563eb",
  burak: "#16a34a",
  other: "#9ca3af",
};

const AUTHOR_LABELS: Record<string, string> = {
  qwen3: "Qwen3",
  sonnet: "Sonnet",
  burak: "Burak",
  other: "Other",
};

function breakdownForMetric(data: AttributionResponse, metric: Metric): AuthorBreakdown {
  if (metric === "commits") return data.by_commits;
  if (metric === "loc_net") return data.by_loc_net;
  return data.by_files;
}

function formatValue(value: number, metric: Metric): string {
  if (metric === "loc_net") return value >= 1000 ? `${(value / 1000).toFixed(1)}k` : String(value);
  return String(value);
}

// ── Donut chart (inline SVG, no Recharts dep) ─────────────────────────────────

interface DonutSlice {
  author: string;
  value: number;
  pct: number;
  color: string;
  startAngle: number;
  endAngle: number;
}

function polarToXY(cx: number, cy: number, r: number, angleDeg: number) {
  const rad = ((angleDeg - 90) * Math.PI) / 180;
  return { x: cx + r * Math.cos(rad), y: cy + r * Math.sin(rad) };
}

function arcPath(
  cx: number,
  cy: number,
  r: number,
  startAngle: number,
  endAngle: number,
  innerR: number
): string {
  const largeArc = endAngle - startAngle > 180 ? 1 : 0;
  const s = polarToXY(cx, cy, r, startAngle);
  const e = polarToXY(cx, cy, r, endAngle);
  const si = polarToXY(cx, cy, innerR, startAngle);
  const ei = polarToXY(cx, cy, innerR, endAngle);
  return [
    `M ${s.x} ${s.y}`,
    `A ${r} ${r} 0 ${largeArc} 1 ${e.x} ${e.y}`,
    `L ${ei.x} ${ei.y}`,
    `A ${innerR} ${innerR} 0 ${largeArc} 0 ${si.x} ${si.y}`,
    "Z",
  ].join(" ");
}

function DonutChart({
  breakdown,
  metric,
  size = 180,
}: {
  breakdown: AuthorBreakdown;
  metric: Metric;
  size?: number;
}) {
  const cx = size / 2;
  const cy = size / 2;
  const r = size * 0.42;
  const innerR = size * 0.26;

  const entries = (["qwen3", "sonnet", "burak", "other"] as const).map((k) => ({
    author: k,
    value: breakdown[k],
    color: AUTHOR_COLORS[k],
  }));
  const total = entries.reduce((s, e) => s + e.value, 0);

  if (total === 0) {
    return (
      <svg width={size} height={size}>
        <circle cx={cx} cy={cy} r={r} fill="none" stroke="#e5e7eb" strokeWidth={r - innerR} />
        <text x={cx} y={cy} textAnchor="middle" dominantBaseline="middle" fontSize={12} fill="#9ca3af">
          no data
        </text>
      </svg>
    );
  }

  let angle = 0;
  const slices: DonutSlice[] = entries
    .filter((e) => e.value > 0)
    .map((e) => {
      const pct = (e.value / total) * 100;
      const sweep = (e.value / total) * 360;
      const slice: DonutSlice = {
        author: e.author,
        value: e.value,
        pct,
        color: e.color,
        startAngle: angle,
        endAngle: angle + sweep,
      };
      angle += sweep;
      return slice;
    });

  const [hovered, setHovered] = useState<string | null>(null);

  return (
    <svg width={size} height={size} style={{ overflow: "visible" }}>
      {slices.map((s) => (
        <path
          key={s.author}
          d={arcPath(cx, cy, r, s.startAngle, s.endAngle, innerR)}
          fill={s.color}
          opacity={hovered === null || hovered === s.author ? 1 : 0.4}
          onMouseEnter={() => setHovered(s.author)}
          onMouseLeave={() => setHovered(null)}
          style={{ cursor: "default", transition: "opacity 0.15s" }}
        />
      ))}
      {/* Centre label */}
      <text x={cx} y={cy - 8} textAnchor="middle" fontSize={11} fill="#6b7280">
        {hovered ? AUTHOR_LABELS[hovered] : "total"}
      </text>
      <text x={cx} y={cy + 10} textAnchor="middle" fontSize={16} fontWeight="700" fill="#111827">
        {hovered
          ? formatValue(
              (breakdown as Record<string, number>)[hovered] ?? 0,
              metric
            )
          : formatValue(total, metric)}
      </text>
    </svg>
  );
}

// ── Legend row ────────────────────────────────────────────────────────────────

function LegendRow({
  author,
  value,
  total,
  metric,
}: {
  author: string;
  value: number;
  total: number;
  metric: Metric;
}) {
  const pct = total > 0 ? ((value / total) * 100).toFixed(1) : "0.0";
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 8,
        padding: "4px 0",
        fontSize: 13,
      }}
    >
      <span
        style={{
          display: "inline-block",
          width: 12,
          height: 12,
          borderRadius: 3,
          background: AUTHOR_COLORS[author],
          flexShrink: 0,
        }}
      />
      <span style={{ flex: 1, color: "#374151" }}>{AUTHOR_LABELS[author]}</span>
      <span style={{ fontWeight: 600, color: "#111827" }}>
        {formatValue(value, metric)}
      </span>
      <span style={{ color: "#9ca3af", minWidth: 42, textAlign: "right" }}>
        {pct}%
      </span>
    </div>
  );
}

// ── Widget ────────────────────────────────────────────────────────────────────

export function ContributionSplit() {
  const [data, setData] = useState<AttributionResponse>(DUMMY);
  const [metric, setMetric] = useState<Metric>("commits");
  const [period, setPeriod] = useState<Period>("7d");
  const [repo, setRepo] = useState<Repo>("all");
  const [lastRefresh, setLastRefresh] = useState<Date>(new Date());

  const refresh = useCallback(() => {
    // TODO: fetch(`/api/attribution?days=${period === "1d" ? 1 : 7}&repo=${repo}`)
    //   .then(r => r.json()).then(setData)
    setLastRefresh(new Date());
  }, [period, repo]);

  useEffect(() => {
    refresh();
    const id = setInterval(refresh, 30_000);
    return () => clearInterval(id);
  }, [refresh]);

  const breakdown = breakdownForMetric(data, metric);
  const total = breakdown.qwen3 + breakdown.sonnet + breakdown.burak + breakdown.other;

  return (
    <div
      style={{
        padding: 20,
        border: "1px solid #e5e7eb",
        borderRadius: 12,
        background: "#fff",
        maxWidth: 480,
      }}
    >
      {/* Header */}
      <div style={{ display: "flex", alignItems: "baseline", gap: 8, marginBottom: 16 }}>
        <h3 style={{ fontSize: 15, fontWeight: 700, margin: 0 }}>Contribution split</h3>
        <span style={{ fontSize: 12, color: "#9ca3af" }}>
          refreshed {lastRefresh.toLocaleTimeString()}
        </span>
      </div>

      {/* Controls */}
      <div style={{ display: "flex", gap: 8, flexWrap: "wrap", marginBottom: 16 }}>
        {/* Period toggle */}
        <div
          style={{
            display: "flex",
            border: "1px solid #e5e7eb",
            borderRadius: 6,
            overflow: "hidden",
          }}
        >
          {(["1d", "7d"] as Period[]).map((p) => (
            <button
              key={p}
              onClick={() => setPeriod(p)}
              style={{
                padding: "4px 10px",
                fontSize: 12,
                fontWeight: period === p ? 600 : 400,
                background: period === p ? "#eff6ff" : "#fff",
                color: period === p ? "#2563eb" : "#6b7280",
                border: "none",
                cursor: "pointer",
              }}
            >
              {p}
            </button>
          ))}
        </div>

        {/* Metric toggle */}
        <div
          style={{
            display: "flex",
            border: "1px solid #e5e7eb",
            borderRadius: 6,
            overflow: "hidden",
          }}
        >
          {(["commits", "loc_net", "files"] as Metric[]).map((m) => (
            <button
              key={m}
              onClick={() => setMetric(m)}
              style={{
                padding: "4px 10px",
                fontSize: 12,
                fontWeight: metric === m ? 600 : 400,
                background: metric === m ? "#eff6ff" : "#fff",
                color: metric === m ? "#2563eb" : "#6b7280",
                border: "none",
                cursor: "pointer",
              }}
            >
              {m === "loc_net" ? "LOC" : m}
            </button>
          ))}
        </div>

        {/* Repo filter */}
        <select
          value={repo}
          onChange={(e) => setRepo(e.target.value as Repo)}
          style={{
            fontSize: 12,
            padding: "4px 8px",
            borderRadius: 6,
            border: "1px solid #e5e7eb",
            color: "#374151",
          }}
        >
          <option value="all">All repos</option>
          <option value="cave-runtime">Cave Runtime</option>
          <option value="pipeline-platform">Pipeline Platform</option>
        </select>
      </div>

      {/* Chart + legend */}
      <div style={{ display: "flex", alignItems: "center", gap: 24 }}>
        <DonutChart breakdown={breakdown} metric={metric} />
        <div style={{ flex: 1 }}>
          {(["qwen3", "sonnet", "burak", "other"] as const).map((a) => (
            <LegendRow
              key={a}
              author={a}
              value={breakdown[a]}
              total={total}
              metric={metric}
            />
          ))}
          {data.timestamps.latest && (
            <div style={{ fontSize: 11, color: "#9ca3af", marginTop: 8 }}>
              last commit: {new Date(data.timestamps.latest).toLocaleString()}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

export default ContributionSplit;
