// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Embedded single-page application for cave-portal.
//!
//! Returns a self-contained HTML document that communicates with the
//! portal API endpoints defined in routes.rs. No build step required —
//! Tailwind CSS is loaded via CDN and all JS is vanilla.

pub fn embedded_ui() -> &'static str {
    r##"<!DOCTYPE html>
<html lang="en" class="h-full bg-gray-950">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>CAVE Platform Portal</title>
  <script src="https://cdn.tailwindcss.com"></script>
  <script>
    tailwind.config = {
      darkMode: 'class',
      theme: {
        extend: {
          colors: {
            cave: { 50:'#f0f9ff', 100:'#e0f2fe', 500:'#0ea5e9', 600:'#0284c7', 700:'#0369a1', 900:'#0c4a6e' }
          }
        }
      }
    }
  </script>
  <style>
    /* Scrollbar */
    ::-webkit-scrollbar { width: 6px; }
    ::-webkit-scrollbar-track { background: #111827; }
    ::-webkit-scrollbar-thumb { background: #374151; border-radius: 3px; }
    /* Sidebar link active */
    .nav-link.active { background: rgba(14,165,233,0.15); color: #38bdf8; }
    /* Health pill colours */
    .health-healthy  { background:#052e16; color:#4ade80; }
    .health-degraded { background:#422006; color:#fb923c; }
    .health-unhealthy{ background:#3f0000; color:#f87171; }
    .health-unknown  { background:#1c1917; color:#a8a29e; }
    /* Fade-in */
    @keyframes fadeIn { from{opacity:0;transform:translateY(6px)} to{opacity:1;transform:none} }
    .fade-in { animation: fadeIn .25s ease both; }
  </style>
</head>
<body class="h-full flex dark text-gray-100 font-sans antialiased">

<!-- ── Sidebar ──────────────────────────────────────────────────── -->
<aside id="sidebar"
       class="w-64 flex-shrink-0 bg-gray-900 border-r border-gray-800 flex flex-col overflow-y-auto transition-all duration-200">

  <!-- Logo -->
  <div class="h-16 flex items-center gap-3 px-4 border-b border-gray-800 flex-shrink-0">
    <div class="w-8 h-8 rounded-lg bg-cave-600 flex items-center justify-center text-white font-bold text-sm select-none">C</div>
    <span class="font-semibold text-gray-100 tracking-tight">CAVE Platform</span>
  </div>

  <!-- Search shortcut -->
  <div class="px-3 py-3">
    <button onclick="openSearch()"
            class="w-full flex items-center gap-2 px-3 py-2 rounded-lg bg-gray-800 hover:bg-gray-700 text-gray-400 text-sm transition-colors">
      <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M21 21l-4.35-4.35M17 11A6 6 0 1 1 5 11a6 6 0 0 1 12 0z"/>
      </svg>
      <span>Search</span>
      <kbd class="ml-auto text-xs bg-gray-700 rounded px-1">⌘K</kbd>
    </button>
  </div>

  <!-- Dashboard link -->
  <nav class="px-2 mb-1">
    <a href="#" onclick="showView('dashboard')"
       class="nav-link flex items-center gap-2 px-3 py-2 rounded-lg text-sm text-gray-300 hover:bg-gray-800 hover:text-white transition-colors active">
      <svg class="w-4 h-4 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M3 12l2-2m0 0l7-7 7 7M5 10v10a1 1 0 001 1h3m10-11l2 2m-2-2v10a1 1 0 01-1 1h-3m-6 0a1 1 0 001-1v-4a1 1 0 011-1h2a1 1 0 011 1v4a1 1 0 001 1m-6 0h6"/>
      </svg>
      Dashboard
    </a>
  </nav>

  <!-- Static nav: Parity -->
  <nav class="px-2 mb-1">
    <a href="#" onclick="showView('parity')"
       class="nav-link flex items-center gap-2 px-3 py-2 rounded-lg text-sm text-gray-300 hover:bg-gray-800 hover:text-white transition-colors">
      <svg class="w-4 h-4 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z"/>
      </svg>
      Parity
    </a>
  </nav>

  <!-- Static nav: Qwen Daemon -->
  <nav class="px-2 mb-1">
    <a href="#" onclick="showView('local-llm')"
       class="nav-link flex items-center gap-2 px-3 py-2 rounded-lg text-sm text-gray-300 hover:bg-gray-800 hover:text-white transition-colors">
      <svg class="w-4 h-4 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"/>
      </svg>
      Qwen Daemon
    </a>
  </nav>

  <!-- Static nav: Contributions -->
  <nav class="px-2 mb-1">
    <a href="#" onclick="showView('contribution')"
       class="nav-link flex items-center gap-2 px-3 py-2 rounded-lg text-sm text-gray-300 hover:bg-gray-800 hover:text-white transition-colors">
      <svg class="w-4 h-4 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M11 3.055A9.001 9.001 0 1020.945 13H11V3.055z"/>
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M20.488 9H15V3.512A9.025 9.025 0 0120.488 9z"/>
      </svg>
      Contributions
    </a>
  </nav>

  <!-- Dynamic nav groups -->
  <div id="nav-groups" class="px-2 flex-1 space-y-4 pb-4"></div>
</aside>

<!-- ── Main ─────────────────────────────────────────────────────── -->
<div class="flex-1 flex flex-col min-w-0">

  <!-- Top bar -->
  <header class="h-16 flex items-center gap-4 px-6 border-b border-gray-800 bg-gray-900 flex-shrink-0">
    <button onclick="toggleSidebar()" class="text-gray-400 hover:text-gray-200">
      <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 6h16M4 12h16M4 18h16"/>
      </svg>
    </button>

    <!-- Global search (collapsed) -->
    <button onclick="openSearch()"
            class="flex-1 max-w-sm flex items-center gap-2 px-3 py-2 rounded-lg bg-gray-800 hover:bg-gray-700 text-gray-400 text-sm transition-colors">
      <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M21 21l-4.35-4.35M17 11A6 6 0 1 1 5 11a6 6 0 0 1 12 0z"/>
      </svg>
      Search all modules…
    </button>

    <div class="flex-1"></div>

    <!-- Notifications bell -->
    <div class="relative">
      <button onclick="toggleNotifications()"
              class="relative p-2 rounded-lg hover:bg-gray-800 text-gray-400 hover:text-gray-200 transition-colors">
        <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6 6 0 10-12 0v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9"/>
        </svg>
        <span id="notif-badge"
              class="absolute top-1 right-1 w-2 h-2 rounded-full bg-red-500 hidden"></span>
      </button>

      <!-- Notification panel -->
      <div id="notif-panel"
           class="hidden absolute right-0 top-12 w-80 bg-gray-800 border border-gray-700 rounded-xl shadow-2xl z-50 overflow-hidden">
        <div class="px-4 py-3 border-b border-gray-700 flex items-center justify-between">
          <span class="font-semibold text-sm">Notifications</span>
          <button onclick="markAllRead()" class="text-xs text-cave-500 hover:text-cave-400">Mark all read</button>
        </div>
        <div id="notif-list" class="max-h-96 overflow-y-auto divide-y divide-gray-700/50"></div>
      </div>
    </div>

    <!-- Avatar -->
    <div class="w-8 h-8 rounded-full bg-cave-700 flex items-center justify-center text-sm font-medium select-none">
      U
    </div>
  </header>

  <!-- Page content -->
  <main id="main-content" class="flex-1 overflow-y-auto bg-gray-950 p-6">

    <!-- Dashboard view -->
    <section id="view-dashboard" class="fade-in">
      <!-- Summary stat cards -->
      <div id="stat-cards" class="grid grid-cols-2 sm:grid-cols-4 gap-4 mb-8"></div>

      <!-- Module health grid -->
      <h2 class="text-sm font-semibold text-gray-400 uppercase tracking-wider mb-4">Modules</h2>
      <div id="module-grid" class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-4"></div>
    </section>

    <!-- Module detail view -->
    <section id="view-module" class="fade-in hidden">
      <div id="module-detail"></div>
    </section>

    <!-- Parity view -->
    <section id="view-parity" class="fade-in hidden">
      <div class="flex items-center justify-between mb-6">
        <h1 class="text-xl font-bold text-gray-100">Upstream Parity</h1>
        <button onclick="loadParity()"
                class="text-xs px-3 py-1.5 rounded-lg bg-gray-800 hover:bg-gray-700 text-gray-300 transition-colors">
          Refresh
        </button>
      </div>
      <p class="text-sm text-gray-400 mb-6">
        Honest, filesystem-backed parity metrics for every reimplemented module.
        <span class="text-cave-400">file</span>, <span class="text-purple-400">function</span>,
        <span class="text-yellow-400">test</span>, and <span class="text-orange-400">surface</span> coverage —
        plus stub counts.
      </p>
      <div id="parity-grid" class="grid grid-cols-1 lg:grid-cols-2 xl:grid-cols-3 gap-6"></div>
    </section>

    <!-- Qwen Daemon view -->
    <section id="view-local-llm" class="fade-in hidden">
      <div class="flex items-center justify-between mb-6">
        <div>
          <h1 class="text-xl font-bold text-gray-100">Qwen Daemon</h1>
          <p class="text-sm text-gray-400 mt-1">24/7 Qwen3-Coder-Next amele scheduler — tier-1 draft generation from parity manifests</p>
        </div>
        <div class="flex items-center gap-3">
          <span id="llm-status-badge" class="text-xs px-2 py-1 rounded-full bg-gray-700 text-gray-400">checking…</span>
          <button onclick="loadLocalLLM()"
                  class="text-xs px-3 py-1.5 rounded-lg bg-gray-800 hover:bg-gray-700 text-gray-300 transition-colors">
            Refresh
          </button>
        </div>
      </div>
      <!-- Metric cards -->
      <div id="llm-metrics" class="grid grid-cols-2 sm:grid-cols-4 lg:grid-cols-7 gap-3 mb-6"></div>
      <!-- Repo tabs -->
      <div id="llm-tabs" class="flex gap-0 border-b border-gray-700 mb-4"></div>
      <!-- Queue table -->
      <div class="flex items-center gap-3 mb-3">
        <span class="text-sm font-semibold text-gray-300" id="llm-queue-label">Queue</span>
        <select id="llm-status-filter" onchange="renderLLMQueue()"
                class="text-xs px-2 py-1 rounded-lg bg-gray-800 border border-gray-700 text-gray-300">
          <option value="all">All statuses</option>
          <option value="pending">Pending</option>
          <option value="in_progress">In Progress</option>
          <option value="done">Done</option>
          <option value="stuck">Stuck</option>
        </select>
      </div>
      <div id="llm-queue" class="overflow-x-auto"></div>
    </section>

    <!-- Contributions view -->
    <section id="view-contribution" class="fade-in hidden">
      <div class="flex items-center justify-between mb-6">
        <h1 class="text-xl font-bold text-gray-100">Contributions</h1>
        <div class="flex items-center gap-2">
          <select id="contrib-period" onchange="loadContribution()"
                  class="text-xs px-2 py-1 rounded-lg bg-gray-800 border border-gray-700 text-gray-300">
            <option value="1">Last 1 day</option>
            <option value="7" selected>Last 7 days</option>
            <option value="30">Last 30 days</option>
          </select>
          <select id="contrib-repo" onchange="loadContribution()"
                  class="text-xs px-2 py-1 rounded-lg bg-gray-800 border border-gray-700 text-gray-300">
            <option value="all">All repos</option>
            <option value="cave-runtime">cave-runtime</option>
            <option value="pipeline-platform">pipeline-platform</option>
            <option value="muleforge">muleforge</option>
          </select>
          <button onclick="loadContribution()"
                  class="text-xs px-3 py-1.5 rounded-lg bg-gray-800 hover:bg-gray-700 text-gray-300 transition-colors">
            Refresh
          </button>
        </div>
      </div>
      <div class="grid grid-cols-1 lg:grid-cols-2 gap-6">
        <!-- Donut chart -->
        <div class="bg-gray-900 border border-gray-800 rounded-xl p-6">
          <h2 class="text-sm font-semibold text-gray-400 uppercase tracking-wider mb-4">Commit Attribution</h2>
          <div id="contrib-donut" class="flex items-center justify-center" style="min-height:200px"></div>
        </div>
        <!-- Legend / breakdown -->
        <div class="bg-gray-900 border border-gray-800 rounded-xl p-6">
          <h2 class="text-sm font-semibold text-gray-400 uppercase tracking-wider mb-4">Breakdown</h2>
          <div id="contrib-legend" class="space-y-3"></div>
        </div>
      </div>
    </section>

  </main>
</div>

<!-- ── Search modal ──────────────────────────────────────────────── -->
<div id="search-modal"
     class="hidden fixed inset-0 z-[100] flex items-start justify-center pt-24 px-4 bg-black/60 backdrop-blur-sm"
     onclick="closeSearch(event)">
  <div class="w-full max-w-xl bg-gray-800 rounded-2xl shadow-2xl border border-gray-700 overflow-hidden" onclick="event.stopPropagation()">
    <div class="flex items-center gap-3 px-4 py-3 border-b border-gray-700">
      <svg class="w-5 h-5 text-gray-400 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M21 21l-4.35-4.35M17 11A6 6 0 1 1 5 11a6 6 0 0 1 12 0z"/>
      </svg>
      <input id="search-input" type="text" placeholder="Search modules, features…"
             class="flex-1 bg-transparent text-gray-100 placeholder-gray-500 outline-none text-sm"
             oninput="doSearch(this.value)" />
      <kbd class="text-xs bg-gray-700 rounded px-1.5 py-0.5 text-gray-400">Esc</kbd>
    </div>
    <div id="search-results" class="max-h-96 overflow-y-auto py-2"></div>
    <div class="px-4 py-2 border-t border-gray-700 flex gap-4 text-xs text-gray-500">
      <span><kbd class="bg-gray-700 rounded px-1">↑↓</kbd> navigate</span>
      <span><kbd class="bg-gray-700 rounded px-1">↵</kbd> open</span>
      <span><kbd class="bg-gray-700 rounded px-1">Esc</kbd> close</span>
    </div>
  </div>
</div>

<script>
// ── State ──────────────────────────────────────────────────────────
let dashboardData = null;
let notifications = [];
let parityData = null;
let currentView = 'dashboard';
let llmData = null;
let llmRepoTab = 'all';
let contribData = null;
let llmPollTimer = null;
let contribPollTimer = null;

// ── Bootstrap ─────────────────────────────────────────────────────
document.addEventListener('DOMContentLoaded', () => {
  loadNav();
  loadDashboard();
  loadNotifications();
  document.addEventListener('keydown', (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key === 'k') { e.preventDefault(); openSearch(); }
    if (e.key === 'Escape') { closeSearch(); closeNotifications(); }
  });
});

// ── Navigation ────────────────────────────────────────────────────
async function loadNav() {
  try {
    const res = await fetch('/api/v1/portal/nav');
    const groups = await res.json();
    const container = document.getElementById('nav-groups');
    container.innerHTML = '';
    groups.forEach(group => {
      const section = document.createElement('div');
      section.innerHTML = `
        <p class="px-3 mb-1 text-[10px] font-semibold uppercase tracking-widest text-gray-500">
          ${escHtml(group.label)}
        </p>
        ${group.items.map(item => `
          <a href="#" onclick="showModule('${escAttr(item.id)}')"
             class="nav-link flex items-center gap-2 px-3 py-1.5 rounded-lg text-sm text-gray-400
                    hover:bg-gray-800 hover:text-white transition-colors mb-0.5">
            ${iconSvg(item.icon)}
            <span class="truncate">${escHtml(item.label)}</span>
            ${item.badge_count ? `<span class="ml-auto text-xs bg-gray-700 rounded-full px-1.5">${item.badge_count}</span>` : ''}
          </a>`).join('')}
      `;
      container.appendChild(section);
    });
  } catch(e) { console.error('nav load failed', e); }
}

// ── Dashboard ─────────────────────────────────────────────────────
async function loadDashboard() {
  try {
    const res = await fetch('/api/v1/portal/dashboard');
    dashboardData = await res.json();
    renderStatCards(dashboardData);
    renderModuleGrid(dashboardData.modules);
  } catch(e) { console.error('dashboard load failed', e); }
}

function renderStatCards(data) {
  const cards = [
    { label:'Total Modules', value: data.total_modules,  color:'text-gray-100', bg:'bg-gray-800' },
    { label:'Healthy',       value: data.healthy_count,  color:'text-green-400', bg:'bg-gray-800' },
    { label:'Degraded',      value: data.degraded_count, color:'text-orange-400', bg:'bg-gray-800' },
    { label:'Unhealthy',     value: data.unhealthy_count,color:'text-red-400',  bg:'bg-gray-800' },
  ];
  document.getElementById('stat-cards').innerHTML = cards.map(c => `
    <div class="${c.bg} rounded-xl p-4 border border-gray-800">
      <p class="text-xs text-gray-400 mb-1">${c.label}</p>
      <p class="text-3xl font-bold ${c.color}">${c.value}</p>
    </div>`).join('');
}

function renderModuleGrid(modules) {
  document.getElementById('module-grid').innerHTML = modules.map(m => `
    <div onclick="showModule('${escAttr(m.module)}')"
         class="group cursor-pointer bg-gray-900 border border-gray-800 rounded-xl p-4
                hover:border-cave-700 hover:bg-gray-800/60 transition-all fade-in">
      <div class="flex items-start justify-between mb-3">
        <div class="w-9 h-9 rounded-lg bg-gray-800 group-hover:bg-gray-700 flex items-center justify-center transition-colors">
          ${categoryEmoji(m.category)}
        </div>
        <span class="text-[11px] rounded-full px-2 py-0.5 font-medium ${healthClass(m.health)}">
          ${m.health}
        </span>
      </div>
      <p class="font-semibold text-sm text-gray-100 mb-0.5 truncate">${escHtml(m.display_name)}</p>
      <p class="text-xs text-gray-500 truncate mb-3">↳ ${escHtml(m.upstream_replacement)}</p>
      <div class="flex items-center justify-between text-xs text-gray-500">
        <span class="capitalize">${escHtml(m.category)}</span>
        <span class="text-green-400">${escHtml(m.key_metric_value)}</span>
      </div>
    </div>`).join('');
}

// ── Parity ────────────────────────────────────────────────────────
async function loadParity() {
  try {
    const res = await fetch('/api/portal/parity');
    parityData = await res.json();
    renderParityGrid(parityData.modules || []);
  } catch(e) {
    document.getElementById('parity-grid').innerHTML =
      '<p class="text-red-400 text-sm col-span-full">Failed to load parity data.</p>';
  }
}

function pct(score) { return Math.round((score || 0) * 100); }

function parityBar(label, score, color) {
  const p = pct(score);
  const w = p > 0 ? Math.max(p, 4) : 0;
  return `
    <div class="flex items-center gap-2 text-xs">
      <span class="w-20 text-gray-400 shrink-0">${label}</span>
      <div class="flex-1 bg-gray-800 rounded-full h-1.5 overflow-hidden">
        <div class="h-full rounded-full ${color} transition-all" style="width:${w}%"></div>
      </div>
      <span class="w-8 text-right font-mono ${p === 100 ? 'text-green-400' : p >= 80 ? 'text-cave-400' : p >= 50 ? 'text-yellow-400' : 'text-red-400'}">${p}%</span>
    </div>`;
}

function renderParityGrid(modules) {
  if (!modules.length) {
    document.getElementById('parity-grid').innerHTML =
      '<p class="text-gray-500 text-sm col-span-full">No parity data collected yet. Start the server and refresh.</p>';
    return;
  }
  document.getElementById('parity-grid').innerHTML = modules.map(m => {
    const overall = pct(m.overall);
    const overallColor = overall >= 80 ? 'text-green-400' : overall >= 50 ? 'text-yellow-400' : 'text-red-400';
    const stubBadge = m.stubs_detected > 0
      ? `<span class="ml-auto text-[10px] font-mono bg-red-900/60 text-red-400 rounded px-1.5 py-0.5">
           ${m.stubs_detected} stub${m.stubs_detected !== 1 ? 's' : ''}
         </span>`
      : `<span class="ml-auto text-[10px] font-mono bg-green-900/40 text-green-500 rounded px-1.5 py-0.5">no stubs</span>`;
    const gaps = (m.gaps || []).slice(0, 4);
    const gapList = gaps.length ? `
      <div class="mt-3 pt-3 border-t border-gray-800">
        <p class="text-[10px] text-gray-500 uppercase tracking-wider mb-2">Top gaps</p>
        ${gaps.map(g => `
          <div class="flex items-center gap-2 text-[11px] text-gray-400 mb-1">
            <span class="w-14 text-gray-600 shrink-0 capitalize">${g.kind}</span>
            <code class="truncate">${escHtml(g.upstream)}</code>
          </div>`).join('')}
        ${(m.gaps||[]).length > 4 ? `<p class="text-[10px] text-gray-600 mt-1">+${(m.gaps||[]).length - 4} more</p>` : ''}
      </div>` : '';
    return `
      <div class="bg-gray-900 border border-gray-800 rounded-xl p-4 hover:border-gray-700 transition-colors fade-in">
        <div class="flex items-center gap-2 mb-3">
          <div class="w-8 h-8 rounded-lg bg-gray-800 flex items-center justify-center text-sm font-bold ${overallColor}">
            ${overall}
          </div>
          <div class="flex-1 min-w-0">
            <p class="text-sm font-semibold text-gray-100 truncate">${escHtml(m.module)}</p>
            <p class="text-[10px] text-gray-500 truncate">${escHtml(m.upstream_ref || '')}</p>
          </div>
          ${stubBadge}
        </div>
        <div class="space-y-1.5">
          ${parityBar('file',     m.file_parity?.score,     'bg-cave-500')}
          ${parityBar('function', m.function_parity?.score, 'bg-purple-500')}
          ${parityBar('test',     m.test_parity?.score,     'bg-yellow-500')}
          ${parityBar('surface',  m.surface_parity?.score,  'bg-orange-500')}
        </div>
        <div class="mt-3 pt-3 border-t border-gray-800 grid grid-cols-2 gap-x-4 text-[10px] text-gray-500">
          <span>${m.file_parity?.matched ?? 0}/${m.file_parity?.total ?? 0} files</span>
          <span>${m.function_parity?.matched ?? 0}/${m.function_parity?.total ?? 0} functions</span>
          <span>${m.test_parity?.matched ?? 0}/${m.test_parity?.total ?? 0} tests</span>
          <span>${m.surface_parity?.matched ?? 0}/${m.surface_parity?.total ?? 0} surfaces</span>
        </div>
        ${gapList}
      </div>`;
  }).join('');
}

// ── Module detail ─────────────────────────────────────────────────
async function showModule(moduleId) {
  showView('module');
  setActiveNav(moduleId);
  document.getElementById('module-detail').innerHTML = `
    <div class="flex items-center gap-3 mb-6">
      <button onclick="showView('dashboard')" class="text-gray-500 hover:text-gray-300">
        <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
        </svg>
      </button>
      <h1 class="text-xl font-bold text-gray-100" id="module-title">Loading…</h1>
    </div>
    <div id="module-content" class="space-y-6"></div>`;

  try {
    const res = await fetch('/api/v1/portal/modules');
    const modules = await res.json();
    const mod = modules.find(m => m.module === moduleId);
    if (!mod) { document.getElementById('module-title').textContent = moduleId + ' (not found)'; return; }

    document.getElementById('module-title').textContent = mod.display_name;
    document.getElementById('module-content').innerHTML = `
      <div class="grid grid-cols-1 md:grid-cols-3 gap-4">
        <div class="bg-gray-900 border border-gray-800 rounded-xl p-4">
          <p class="text-xs text-gray-500 mb-1">Health</p>
          <span class="text-sm font-medium rounded-full px-2 py-0.5 ${healthClass(mod.health)}">${mod.health}</span>
        </div>
        <div class="bg-gray-900 border border-gray-800 rounded-xl p-4">
          <p class="text-xs text-gray-500 mb-1">Category</p>
          <p class="text-sm font-medium text-gray-100 capitalize">${escHtml(mod.category)}</p>
        </div>
        <div class="bg-gray-900 border border-gray-800 rounded-xl p-4">
          <p class="text-xs text-gray-500 mb-1">Replaces</p>
          <p class="text-sm font-medium text-gray-100">${escHtml(mod.upstream_replacement)}</p>
        </div>
      </div>
      <div class="bg-gray-900 border border-gray-800 rounded-xl p-4">
        <p class="text-xs text-gray-500 mb-3 font-semibold uppercase tracking-wider">API Endpoints</p>
        <div class="space-y-2">
          <div class="flex items-center gap-3">
            <span class="text-xs bg-green-900 text-green-400 rounded px-1.5 py-0.5 font-mono">GET</span>
            <code class="text-xs text-gray-400 font-mono">/api/${escHtml(mod.module)}/health</code>
          </div>
        </div>
      </div>
      <div class="bg-gray-900 border border-gray-800 rounded-xl p-4">
        <p class="text-xs text-gray-500 mb-3 font-semibold uppercase tracking-wider">Stats</p>
        <pre class="text-xs text-gray-300 font-mono">${escHtml(JSON.stringify(mod.stats, null, 2))}</pre>
      </div>`;
  } catch(e) { document.getElementById('module-title').textContent = 'Error loading module'; }
}

// ── Notifications ─────────────────────────────────────────────────
async function loadNotifications() {
  try {
    const res = await fetch('/api/v1/portal/notifications');
    notifications = await res.json();
    const unread = notifications.filter(n => !n.read).length;
    const badge = document.getElementById('notif-badge');
    badge.classList.toggle('hidden', unread === 0);
    renderNotifications();
  } catch(e) { console.error('notifications load failed', e); }
}

function renderNotifications() {
  const list = document.getElementById('notif-list');
  if (!notifications.length) {
    list.innerHTML = '<p class="px-4 py-6 text-sm text-gray-500 text-center">No notifications</p>';
    return;
  }
  list.innerHTML = notifications.map(n => `
    <div class="px-4 py-3 hover:bg-gray-700/40 transition-colors ${n.read ? 'opacity-60' : ''}">
      <div class="flex items-start gap-2">
        <span class="w-2 h-2 mt-1.5 rounded-full flex-shrink-0 ${severityDot(n.severity)}"></span>
        <div class="min-w-0">
          <p class="text-sm font-medium text-gray-100 leading-tight">${escHtml(n.title)}</p>
          <p class="text-xs text-gray-400 mt-0.5 leading-snug">${escHtml(n.body)}</p>
          <p class="text-[10px] text-gray-600 mt-1 uppercase tracking-wide">${escHtml(n.module)}</p>
        </div>
      </div>
    </div>`).join('');
}

function toggleNotifications() {
  const panel = document.getElementById('notif-panel');
  panel.classList.toggle('hidden');
}

function closeNotifications() {
  document.getElementById('notif-panel').classList.add('hidden');
}

function markAllRead() {
  notifications = notifications.map(n => ({...n, read: true}));
  document.getElementById('notif-badge').classList.add('hidden');
  renderNotifications();
}

// ── Search ────────────────────────────────────────────────────────
let searchTimer = null;

function openSearch() {
  document.getElementById('search-modal').classList.remove('hidden');
  setTimeout(() => document.getElementById('search-input').focus(), 50);
}

function closeSearch(e) {
  if (e && e.target !== document.getElementById('search-modal')) return;
  document.getElementById('search-modal').classList.add('hidden');
  document.getElementById('search-input').value = '';
  document.getElementById('search-results').innerHTML = '';
}

function doSearch(q) {
  clearTimeout(searchTimer);
  if (!q.trim()) { document.getElementById('search-results').innerHTML = ''; return; }
  searchTimer = setTimeout(async () => {
    try {
      const res = await fetch('/api/v1/portal/search?q=' + encodeURIComponent(q));
      const results = await res.json();
      const container = document.getElementById('search-results');
      if (!results.length) {
        container.innerHTML = '<p class="px-4 py-4 text-sm text-gray-500">No results for "' + escHtml(q) + '"</p>';
        return;
      }
      container.innerHTML = results.map(r => `
        <a href="#" onclick="showModule('${escAttr(r.module)}'); closeSearch({})"
           class="flex items-center gap-3 px-4 py-2.5 hover:bg-gray-700/50 transition-colors">
          <div class="w-8 h-8 rounded-lg bg-gray-700 flex items-center justify-center text-xs">
            ${categoryEmoji(r.module)}
          </div>
          <div class="min-w-0">
            <p class="text-sm font-medium text-gray-100 truncate">${escHtml(r.title)}</p>
            <p class="text-xs text-gray-500 truncate">${escHtml(r.description)}</p>
          </div>
          <span class="ml-auto text-[10px] bg-gray-700 rounded px-1.5 py-0.5 text-gray-400 flex-shrink-0">${escHtml(r.kind)}</span>
        </a>`).join('');
    } catch(e) { console.error('search failed', e); }
  }, 200);
}

// ── View helpers ──────────────────────────────────────────────────
function showView(name) {
  ['dashboard','module','parity','local-llm','contribution'].forEach(v => {
    const el = document.getElementById('view-' + v);
    if (el) el.classList.toggle('hidden', v !== name);
  });
  currentView = name;
  clearInterval(llmPollTimer); clearInterval(contribPollTimer);
  if (name === 'dashboard') { setActiveNav('dashboard'); if (!dashboardData) loadDashboard(); }
  if (name === 'parity') { loadParity(); }
  if (name === 'local-llm') { loadLocalLLM(); llmPollTimer = setInterval(loadLocalLLM, 30000); }
  if (name === 'contribution') { loadContribution(); contribPollTimer = setInterval(loadContribution, 30000); }
}

// ── Qwen Daemon ────────────────────────────────────────────────────
const LLM_STATUS_COLORS = {pending:'#6b7280',in_progress:'#2563eb',done:'#16a34a',stuck:'#dc2626'};

async function loadLocalLLM() {
  try {
    const r = await fetch('/api/v1/local-llm/queue');
    if (r.ok) { llmData = await r.json(); renderLLMView(); return; }
  } catch(_) {}
  // fallback: read from git log
  try {
    const r = await fetch('/api/v1/local-llm/commits?limit=50');
    if (r.ok) { const d = await r.json(); renderLLMFromCommits(d); return; }
  } catch(_) {}
  renderLLMUnavailable();
}

function renderLLMUnavailable() {
  document.getElementById('llm-status-badge').textContent = 'API pending';
  document.getElementById('llm-status-badge').className = 'text-xs px-2 py-1 rounded-full bg-yellow-900 text-yellow-400';
  document.getElementById('llm-metrics').innerHTML = '';
  document.getElementById('llm-queue').innerHTML = `
    <div class="bg-gray-900 border border-gray-800 rounded-xl p-8 text-center">
      <p class="text-gray-400 text-sm">Queue API not yet wired — see <code class="text-cave-400">/api/v1/local-llm/queue</code></p>
      <p class="text-gray-600 text-xs mt-2">Check daemon log: <code>~/Library/Logs/cave-local-llm-daemon.log</code></p>
    </div>`;
  // Show recent commits from git log via the portal search API as fallback
  loadLLMCommitsFallback();
}

async function loadLLMCommitsFallback() {
  try {
    const r = await fetch('/api/v1/portal/search?q=qwen-amele');
    if (!r.ok) return;
    const results = await r.json();
    if (!results.length) return;
    document.getElementById('llm-queue').innerHTML = `
      <div class="bg-gray-900 border border-gray-800 rounded-xl overflow-hidden mt-4">
        <div class="px-4 py-3 border-b border-gray-800">
          <span class="text-sm font-semibold text-gray-300">Recent [qwen-amele] activity</span>
        </div>
        <div class="divide-y divide-gray-800">
          ${results.slice(0,20).map(r => `
            <div class="px-4 py-2.5 flex items-start gap-3">
              <span class="text-xs bg-purple-900 text-purple-300 rounded px-1.5 py-0.5 font-mono shrink-0 mt-0.5">tier1</span>
              <div class="min-w-0">
                <p class="text-sm text-gray-200 truncate">${escHtml(r.title||r.description||r.module)}</p>
              </div>
            </div>`).join('')}
        </div>
      </div>`;
  } catch(_) {}
}

function renderLLMView() {
  if (!llmData) { renderLLMUnavailable(); return; }
  const items = llmData.items || [];
  const metrics = llmData.metrics || {};
  document.getElementById('llm-status-badge').textContent = 'LIVE';
  document.getElementById('llm-status-badge').className = 'text-xs px-2 py-1 rounded-full bg-green-900 text-green-400';
  const statDefs = [
    {label:'Ticks', key:'daemon_ticks_total', color:'#7c3aed'},
    {label:'Tier-1 Commits', key:'tier1_commits_total', color:'#16a34a'},
    {label:'Escalations', key:'tier2_escalations_total', color:'#dc2626'},
    {label:'Pending', key:'queue_pending', color:'#6b7280'},
    {label:'In Progress', key:'queue_in_progress', color:'#2563eb'},
    {label:'Done', key:'queue_done', color:'#16a34a'},
    {label:'Stuck', key:'queue_stuck', color:'#dc2626'},
  ];
  document.getElementById('llm-metrics').innerHTML = statDefs.map(s => `
    <div class="bg-gray-900 border border-gray-800 rounded-xl p-3 text-center">
      <div class="text-2xl font-bold" style="color:${s.color}">${metrics[s.key]??0}</div>
      <div class="text-xs text-gray-500 mt-1">${s.label}</div>
    </div>`).join('');
  // Map repo_path (full path or short name) → display label
  function repoLabel(r) {
    if (r === 'all') return 'All';
    if (r.includes('muleforge')) return 'Muleforge';
    if (r.includes('pipeline')) return 'Pipeline';
    if (r.includes('cave-runtime')) return 'Cave Runtime';
    return r.split('/').pop() || r;
  }
  function repoKey(item) {
    return item.repo_path ? String(item.repo_path) : (item.repo || 'cave-runtime');
  }
  const repos = ['all',...new Set(items.map(repoKey))];
  document.getElementById('llm-tabs').innerHTML = repos.map(r => `
    <button onclick="llmRepoTab='${r}';renderLLMQueue()" style="
      padding:8px 16px;font-size:13px;background:none;border:none;
      border-bottom:${llmRepoTab===r?'2px solid #2563eb':'2px solid transparent'};
      color:${llmRepoTab===r?'#2563eb':'#6b7280'};cursor:pointer">
      ${repoLabel(r)}
      <span style="margin-left:4px;font-size:11px;padding:1px 5px;border-radius:8px;
        background:${llmRepoTab===r?'#dbeafe':'#374151'};color:${llmRepoTab===r?'#2563eb':'#9ca3af'}">
        ${r==='all'?items.length:items.filter(i=>repoKey(i)===r).length}
      </span>
    </button>`).join('');
  renderLLMQueue();
}

function renderLLMQueue() {
  const items = (llmData&&llmData.items)||[];
  const filter = document.getElementById('llm-status-filter').value;
  function repoKey(item) { return item.repo_path ? String(item.repo_path) : (item.repo || 'cave-runtime'); }
  let filtered = llmRepoTab==='all'?items:items.filter(i=>repoKey(i)===llmRepoTab);
  if (filter!=='all') filtered = filtered.filter(i=>i.status===filter);
  document.getElementById('llm-queue-label').textContent = \`Queue (\${filtered.length})\`;
  if (!filtered.length) {
    document.getElementById('llm-queue').innerHTML = `<div class="py-8 text-center text-gray-500 text-sm">No items matching filter.</div>`;
    return;
  }
  document.getElementById('llm-queue').innerHTML = `
    <table style="width:100%;border-collapse:collapse;font-size:13px;background:#161b22;border:1px solid #30363d;border-radius:8px;overflow:hidden">
      <thead><tr style="background:#21262d">
        ${['Crate','Upstream','Function','Status','Attempts','Last Error','Updated'].map(h=>
          `<th style="padding:10px 14px;text-align:left;font-weight:600;color:#e6edf3;border-bottom:1px solid #30363d">${h}</th>`
        ).join('')}
      </tr></thead>
      <tbody>
        ${filtered.map(item => {
          const c = LLM_STATUS_COLORS[item.status]||'#6b7280';
          return `<tr style="border-bottom:1px solid #21262d">
            <td style="padding:10px 14px;font-weight:600;color:#e6edf3">${escHtml(item.crate_name||'')}</td>
            <td style="padding:10px 14px;color:#8b949e;font-size:12px">${escHtml(item.upstream_repo||'')}</td>
            <td style="padding:10px 14px;font-family:monospace;color:#e6edf3">${escHtml(item.upstream_fn||'')}</td>
            <td style="padding:10px 14px">
              <span style="padding:2px 8px;border-radius:12px;font-size:11px;font-weight:600;
                background:${c}22;color:${c};border:1px solid ${c}44">${item.status}</span>
            </td>
            <td style="padding:10px 14px;text-align:center;color:#e6edf3">${item.attempts||0}</td>
            <td style="padding:10px 14px;color:#f85149;font-size:11px;max-width:180px;
              overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${escHtml(item.last_error||'—')}</td>
            <td style="padding:10px 14px;color:#8b949e;font-size:11px">${item.updated_at?new Date(item.updated_at).toLocaleString():'—'}</td>
          </tr>`;
        }).join('')}
      </tbody>
    </table>`;
}

// ── Contributions ──────────────────────────────────────────────────
const CONTRIB_COLORS = {qwen3:'#7c3aed',sonnet:'#2563eb',burak:'#16a34a',other:'#9ca3af'};

async function loadContribution() {
  const days = document.getElementById('contrib-period').value;
  const repo = document.getElementById('contrib-repo').value;
  try {
    const r = await fetch(\`/api/v1/attribution?days=\${days}&repo=\${repo}\`);
    if (r.ok) { contribData = await r.json(); renderContribution(); return; }
  } catch(_) {}
  renderContributionUnavailable();
}

function renderContributionUnavailable() {
  document.getElementById('contrib-donut').innerHTML =
    `<p class="text-gray-500 text-sm text-center">Attribution API not yet wired.<br><code class="text-cave-400 text-xs">/api/v1/attribution</code></p>`;
  document.getElementById('contrib-legend').innerHTML =
    `<p class="text-gray-600 text-xs">Expected: qwen3, sonnet, burak, other breakdown</p>`;
}

function renderContribution() {
  if (!contribData) { renderContributionUnavailable(); return; }
  // API returns by_commits:{qwen3,sonnet,burak,other}; map to {name:{commits:N}}.
  const raw = contribData.by_commits || contribData.authors || {};
  const authors = (typeof Object.values(raw)[0] === 'number')
    ? Object.fromEntries(Object.entries(raw).map(([k,v]) => [k, {commits: v}]))
    : raw;
  const total = Object.values(authors).reduce((s,v)=>s+(v.commits||0),0);
  if (!total) { renderContributionUnavailable(); return; }
  // SVG donut
  const size = 180, cx = size/2, cy = size/2, r = 65, stroke = 28;
  const circumference = 2 * Math.PI * r;
  let offset = 0;
  const arcs = Object.entries(authors).map(([name, d]) => {
    const pct = (d.commits||0)/total;
    const dash = pct * circumference;
    const gap  = circumference - dash;
    const arc  = \`<circle cx="\${cx}" cy="\${cy}" r="\${r}"
      fill="none" stroke="\${CONTRIB_COLORS[name]||'#9ca3af'}" stroke-width="\${stroke}"
      stroke-dasharray="\${dash.toFixed(2)} \${gap.toFixed(2)}"
      stroke-dashoffset="-\${offset.toFixed(2)}"
      transform="rotate(-90 \${cx} \${cy})"
      style="cursor:pointer" title="\${name}: \${d.commits} commits"/>\`;
    offset += dash;
    return arc;
  });
  document.getElementById('contrib-donut').innerHTML = \`
    <svg width="\${size}" height="\${size}" viewBox="0 0 \${size} \${size}">
      \${arcs.join('')}
      <text x="\${cx}" y="\${cy-6}" text-anchor="middle" fill="#e6edf3" font-size="20" font-weight="700">\${total}</text>
      <text x="\${cx}" y="\${cy+14}" text-anchor="middle" fill="#8b949e" font-size="11">commits</text>
    </svg>\`;
  // Legend
  document.getElementById('contrib-legend').innerHTML = Object.entries(authors).map(([name, d]) => {
    const pct = total ? Math.round((d.commits||0)/total*100) : 0;
    return \`
      <div class="flex items-center gap-3">
        <div style="width:12px;height:12px;border-radius:50%;background:\${CONTRIB_COLORS[name]||'#9ca3af'};flex-shrink:0"></div>
        <div class="flex-1">
          <div class="flex items-center justify-between">
            <span class="text-sm font-medium text-gray-200 capitalize">\${escHtml(name)}</span>
            <span class="text-sm font-bold text-gray-100">\${d.commits||0}</span>
          </div>
          <div class="mt-1 h-1.5 rounded-full bg-gray-800">
            <div style="width:\${pct}%;height:100%;border-radius:9999px;background:\${CONTRIB_COLORS[name]||'#9ca3af'}"></div>
          </div>
          <span class="text-xs text-gray-500">\${pct}% of commits</span>
        </div>
      </div>\`;
  }).join('');
}

function setActiveNav(id) {
  document.querySelectorAll('.nav-link').forEach(el => el.classList.remove('active'));
}

function toggleSidebar() {
  const sidebar = document.getElementById('sidebar');
  sidebar.classList.toggle('w-64');
  sidebar.classList.toggle('w-0');
  sidebar.classList.toggle('overflow-hidden');
}

// ── Utilities ─────────────────────────────────────────────────────
function healthClass(h) {
  const map = { healthy:'health-healthy', degraded:'health-degraded',
                unhealthy:'health-unhealthy', unknown:'health-unknown' };
  return map[h] || 'health-unknown';
}

function severityDot(s) {
  const map = { critical:'bg-red-500', warning:'bg-orange-400', info:'bg-blue-400' };
  return map[s] || 'bg-gray-500';
}

function categoryEmoji(cat) {
  const map = {
    security:'🔒', observability:'📊', 'dev-tools':'🛠', platform:'⚙️', ai:'🤖',
    // fall back to module id lookup
    secrets:'🔑', certs:'📜', vulns:'🐛', sbom:'📦', sign:'✍️', forensics:'🔍',
    pii:'👤', scan:'🔬', policy:'📋', dast:'🕷', pam:'🚪',
    status:'🟢', uptime:'⏱', alerts:'🔔', slo:'🎯', incidents:'🚨', profiler:'📈',
    lint:'✅', docs:'📖', changelog:'📝', devlake:'📐', workflows:'⚡', scaffold:'🏗',
    flags:'🚩', cost:'💰', registry:'🗄', chat:'💬', chaos:'⚠️', backup:'💾',
    'ai-obs':'🤖',
  };
  return `<span class="text-base leading-none">${map[cat] || '📦'}</span>`;
}

function iconSvg(icon) {
  const paths = {
    shield: 'M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z',
    'chart-bar': 'M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z',
    wrench: 'M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z M15 12a3 3 0 11-6 0 3 3 0 016 0z',
    cog: 'M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z M15 12a3 3 0 11-6 0 3 3 0 016 0z',
    'cpu-chip': 'M9 3H7a2 2 0 00-2 2v2M9 3h6M9 3V1m6 2h2a2 2 0 012 2v2m0 0h2m-2 0v6m0 0h2m-2 0v2a2 2 0 01-2 2h-2m0 0H9m6 0v2M9 21H7a2 2 0 01-2-2v-2m0 0H3m2 0v-6m0 0H3m2 0V9a2 2 0 012-2h2m0 0V5',
    cube: 'M20 7l-8-4-8 4m16 0l-8 4m8-4v10l-8 4m0-10L4 7m8 4v10M4 7v10l8 4',
  };
  const d = paths[icon] || paths.cube;
  return `<svg class="w-4 h-4 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="${d}"/>
  </svg>`;
}

function escHtml(s) {
  if (s == null) return '';
  return String(s)
    .replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;')
    .replace(/"/g,'&quot;').replace(/'/g,'&#39;');
}
function escAttr(s) { return escHtml(s); }
</script>
</body>
</html>"##
}
