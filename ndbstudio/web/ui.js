// T2-1: Export functions
function exportCsv(headers, rows) {
  const escape = (v) => `"${String(v).replace(/"/g, '""')}"`;
  const lines = [headers.map(escape).join(",")];
  for (const row of rows) {
    lines.push(row.map(escape).join(","));
  }
  return lines.join("\n");
}

function exportJson(headers, rows) {
  const data = rows.map((row) => {
    const obj = {};
    headers.forEach((h, i) => { obj[h] = row[i]; });
    return obj;
  });
  return JSON.stringify(data, null, 2);
}

function downloadBlob(content, filename, mimeType) {
  const blob = new Blob([content], { type: mimeType });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

// T3-1: Query history
function getUniqueQueryHistory() {
  const entries = state.workspace.timeline?.entries || [];
  const seen = new Set();
  const unique = [];
  for (let i = entries.length - 1; i >= 0 && unique.length < 20; i--) {
    const q = entries[i].query.trim();
    if (!seen.has(q)) {
      seen.add(q);
      unique.push({ query: q, mode: entries[i].run_mode });
    }
  }
  return unique;
}

function renderHistoryDropdown(filter = "") {
  const items = getUniqueQueryHistory();
  const lowerFilter = filter.toLowerCase();
  const filtered = lowerFilter
    ? items.filter((item) => item.query.toLowerCase().includes(lowerFilter))
    : items;

  if (!filtered.length) {
    els.historyList.innerHTML = `<div class="ac-hint">No matching queries</div>`;
    return;
  }

  els.historyList.innerHTML = filtered
    .map(
      (item, i) =>
        `<button type="button" class="history-item" data-history-index="${i}"><span class="history-item-mode">${escapeHtml(item.mode)}</span>${escapeHtml(item.query)}</button>`
    )
    .join("");

  els.historyList.querySelectorAll("[data-history-index]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const idx = Number(btn.dataset.historyIndex);
      const item = filtered[idx];
      if (item) {
        setEditorValue(item.query);
        schedulePersistActiveTabQuery(item.query);
        els.historyDropdown.style.display = "none";
        showToast("Query loaded from history");
      }
    });
  });
}

// T3-3: Modal for creating nodes/edges
function showModal(html) {
  els.modalContainer.innerHTML = `<div class="modal-backdrop"><div class="modal-panel">${html}</div></div>`;
  els.modalContainer.querySelector(".modal-backdrop").addEventListener("click", (e) => {
    if (e.target === e.currentTarget) closeModal();
  });
}

function closeModal() {
  els.modalContainer.innerHTML = "";
}

function showAddNodeModal() {
  const schema = state.workspace.schema;
  const labelOptions = (schema?.node_types || [])
    .map((t) => `<option value="${escapeHtml(t.name)}">${escapeHtml(t.name)}</option>`)
    .join("");

  showModal(`
    <h2>Add Node</h2>
    <div class="modal-field">
      <label>Label</label>
      <input id="modalNodeLabel" type="text" list="modalLabelList" placeholder="Person" />
      <datalist id="modalLabelList">${labelOptions}</datalist>
    </div>
    <h3>Properties</h3>
    <div id="modalProps" class="modal-props"></div>
    <button type="button" class="ghost-button" id="modalAddProp">+ Property</button>
    <div class="modal-actions">
      <button type="button" class="ghost-button" id="modalCancel">Cancel</button>
      <button type="button" id="modalConfirm">Create Node</button>
    </div>
  `);

  document.getElementById("modalCancel").addEventListener("click", closeModal);
  document.getElementById("modalAddProp").addEventListener("click", () => {
    const row = document.createElement("div");
    row.className = "modal-prop-row";
    row.innerHTML = `<input type="text" placeholder="key" /><input type="text" placeholder="value" /><button type="button" class="ghost-button">x</button>`;
    row.querySelector("button").addEventListener("click", () => row.remove());
    document.getElementById("modalProps").appendChild(row);
  });
  document.getElementById("modalConfirm").addEventListener("click", async () => {
    const label = document.getElementById("modalNodeLabel").value.trim();
    if (!label) { showToast("Label is required"); return; }
    const props = [];
    document.querySelectorAll("#modalProps .modal-prop-row").forEach((row) => {
      const inputs = row.querySelectorAll("input");
      const key = inputs[0].value.trim();
      const val = inputs[1].value.trim();
      if (key) props.push(`${key}: "${val}"`);
    });
    const propsStr = props.length ? ` {${props.join(", ")}}` : "";
    const nql = `add (n:${label}${propsStr})`;
    try {
      await api("/api/query/run", { method: "POST", body: JSON.stringify({ query: nql, run_mode: "run" }) });
      closeModal();
      showToast(`Node created: ${label}`);
      await refreshWorkbench();
    } catch (err) {
      showToast(`Error: ${err.message}`);
    }
  });
}

function showEditNodeModal(nodeId) {
  const sub = state.graphCurrentSubgraph;
  const node = sub?.nodes?.find((n) => n.id === nodeId);
  if (!node) { showToast("Node not found in current subgraph"); return; }

  const existingProps = (node.properties || []).map((p, i) => `
    <div class="modal-prop-row">
      <input type="text" placeholder="key" value="${escapeHtml(p.key)}" />
      <input type="text" placeholder="value" value="${escapeHtml(p.value)}" />
      <button type="button" class="ghost-button">x</button>
    </div>
  `).join("");

  showModal(`
    <h2>Edit Node</h2>
    <div class="modal-field">
      <label>ID</label>
      <input type="text" value="${escapeHtml(node.id)}" disabled />
    </div>
    <div class="modal-field">
      <label>Label</label>
      <input type="text" value="${escapeHtml(node.label || "")}" disabled />
    </div>
    <h3>Properties</h3>
    <div id="modalProps" class="modal-props">${existingProps}</div>
    <button type="button" class="ghost-button" id="modalAddProp">+ Property</button>
    <div class="modal-actions">
      <button type="button" class="ghost-button" id="modalCancel">Cancel</button>
      <button type="button" id="modalConfirm">Save Changes</button>
    </div>
  `);

  document.getElementById("modalCancel").addEventListener("click", closeModal);
  document.querySelectorAll("#modalProps .modal-prop-row button").forEach((btn) => {
    btn.addEventListener("click", () => btn.closest(".modal-prop-row").remove());
  });
  document.getElementById("modalAddProp").addEventListener("click", () => {
    const row = document.createElement("div");
    row.className = "modal-prop-row";
    row.innerHTML = `<input type="text" placeholder="key" /><input type="text" placeholder="value" /><button type="button" class="ghost-button">x</button>`;
    row.querySelector("button").addEventListener("click", () => row.remove());
    document.getElementById("modalProps").appendChild(row);
  });
  document.getElementById("modalConfirm").addEventListener("click", async () => {
    const sets = [];
    document.querySelectorAll("#modalProps .modal-prop-row").forEach((row) => {
      const inputs = row.querySelectorAll("input");
      const key = inputs[0].value.trim();
      const val = inputs[1].value.trim();
      if (key) sets.push(`n.${key} = "${val}"`);
    });
    if (!sets.length) { showToast("No properties to update"); return; }
    const nql = `update (n) set ${sets.join(", ")} where n.__id = "${nodeId}"`;
    try {
      await api("/api/query/run", { method: "POST", body: JSON.stringify({ query: nql, run_mode: "run" }) });
      closeModal();
      showToast("Node properties updated");
      await refreshWorkbench();
    } catch (err) {
      showToast(`Error: ${err.message}`);
    }
  });
}

function showAddEdgeModal() {
  const schema = state.workspace.schema;
  const edgeOptions = (schema?.edge_types || [])
    .map((t) => `<option value="${escapeHtml(t.name)}">${escapeHtml(t.name)}</option>`)
    .join("");

  showModal(`
    <h2>Add Edge</h2>
    <div class="modal-field">
      <label>Source Node ID</label>
      <input id="modalEdgeSource" type="text" placeholder="node-uuid" />
    </div>
    <div class="modal-field">
      <label>Edge Type</label>
      <input id="modalEdgeType" type="text" list="modalEdgeTypeList" placeholder="KNOWS" />
      <datalist id="modalEdgeTypeList">${edgeOptions}</datalist>
    </div>
    <div class="modal-field">
      <label>Target Node ID</label>
      <input id="modalEdgeTarget" type="text" placeholder="node-uuid" />
    </div>
    <h3>Properties</h3>
    <div id="modalProps" class="modal-props"></div>
    <button type="button" class="ghost-button" id="modalAddProp">+ Property</button>
    <div class="modal-actions">
      <button type="button" class="ghost-button" id="modalCancel">Cancel</button>
      <button type="button" id="modalConfirm">Create Edge</button>
    </div>
  `);

  document.getElementById("modalCancel").addEventListener("click", closeModal);
  document.getElementById("modalAddProp").addEventListener("click", () => {
    const row = document.createElement("div");
    row.className = "modal-prop-row";
    row.innerHTML = `<input type="text" placeholder="key" /><input type="text" placeholder="value" /><button type="button" class="ghost-button">x</button>`;
    row.querySelector("button").addEventListener("click", () => row.remove());
    document.getElementById("modalProps").appendChild(row);
  });

  // Pre-fill source from selected graph node
  if (state.selectedGraphNodeId) {
    document.getElementById("modalEdgeSource").value = state.selectedGraphNodeId;
  }

  document.getElementById("modalConfirm").addEventListener("click", async () => {
    const source = document.getElementById("modalEdgeSource").value.trim();
    const edgeType = document.getElementById("modalEdgeType").value.trim();
    const target = document.getElementById("modalEdgeTarget").value.trim();
    if (!source || !edgeType || !target) { showToast("All fields are required"); return; }
    const props = [];
    document.querySelectorAll("#modalProps .modal-prop-row").forEach((row) => {
      const inputs = row.querySelectorAll("input");
      const key = inputs[0].value.trim();
      const val = inputs[1].value.trim();
      if (key) props.push(`${key}: "${val}"`);
    });
    const propsStr = props.length ? ` {${props.join(", ")}}` : "";
    // Use ADD edge NQL syntax - source and target by node id
    const nql = `add (s) -[:${edgeType}${propsStr}]-> (t) where s.__id = "${source}" and t.__id = "${target}"`;
    try {
      await api("/api/query/run", { method: "POST", body: JSON.stringify({ query: nql, run_mode: "run" }) });
      closeModal();
      showToast(`Edge created: ${edgeType}`);
      await refreshWorkbench();
    } catch (err) {
      showToast(`Error: ${err.message}`);
    }
  });
}

function activeTab() {
  return state.workspace.session?.tabs?.find((tab) => tab.id === state.workspace.session?.active_tab_id) || null;
}

function activeProject() {
  return (state.workspace.projects || []).find((project) => project.db_path === state.workspace.dbPath) || null;
}

function activeTabId() {
  return activeTab()?.id || null;
}

function activeResult() {
  return activeTab()?.last_result || null;
}

function activeFindingContext() {
  const result = activeResult();
  const activeProjectRecord = activeProject();
  const selectedRun = state.selectedTimelineIndex !== null
    ? state.workspace.timeline?.entries?.[state.selectedTimelineIndex] || null
    : null;
  const rowIndex = state.selectedResultRowIndex;
  const rowHint = rowIndex !== null ? result?.row_graph_hints?.[rowIndex] || null : null;
  const selectedRowValues = rowIndex !== null && result?.rows?.[rowIndex] ? result.rows[rowIndex] : null;
  const headers = result?.headers || [];
  const rowSummary = selectedRowValues && headers.length
    ? selectedRowValues
        .slice(0, Math.min(headers.length, 3))
        .map((value, index) => `${headers[index]}=${value}`)
        .join(" · ")
    : "";
  const fallbackTitle = rowSummary
    ? `Finding: ${rowSummary}`
    : result?.summary
      ? `Finding: ${result.summary}`
      : `Finding from ${activeProjectRecord?.name || "project"}`;

  return {
    title: fallbackTitle.slice(0, 120),
    body: rowSummary
      ? `Observation\n${rowSummary}\n\nWhy it matters\n`
      : "Observation\n\nWhy it matters\n",
    tabId: activeTabId(),
    runId: selectedRun?.id || null,
    queryText: activeTab()?.query_text || "",
    summary: result?.summary || selectedRun?.summary || "",
    rowIndex,
    graphFocusNodeId: rowHint?.focus_node_id || rowHint?.node_ids?.[0] || state.selectedGraphNodeId || null,
  };
}

function currentGraphHint() {
  const result = activeResult();
  if (!result?.graph_hint) return null;

  const rowHint =
    state.selectedResultRowIndex !== null
      ? result?.row_graph_hints?.[state.selectedResultRowIndex] || null
      : null;

  // Use global hint (all result nodes) as base, row hint only for focus
  if (rowHint) {
    return {
      ...result.graph_hint,
      focus_node_id: rowHint.focus_node_id || rowHint.node_ids?.[0] || result.graph_hint.focus_node_id,
    };
  }
  return result.graph_hint;
}

function refreshGraphDirtyState() {
  // No-op: result-focus mode was removed, graph always shows dataset
}

function setWorkspaceTab(tab) {
  state.uiPrefs.workspaceTab = tab;
  saveUiPrefs();
  renderWorkspaceTabs();
}

async function api(path, options = {}) {
  const headers = { ...(options.headers || {}) };
  if (options.body !== undefined && !headers["Content-Type"]) {
    headers["Content-Type"] = "application/json";
  }
  const response = await fetch(path, {
    headers,
    ...options,
  });

  if (!response.ok) {
    const text = await response.text().catch(() => "");
    throw new Error(text || `HTTP ${response.status} on ${path}`);
  }

  const contentType = response.headers.get("content-type") || "";
  if (contentType.includes("application/json")) {
    return response.json();
  }
  return response.text();
}

async function ensureReferenceData() {
  if (state.reference) return state.reference;
  state.reference = await api("/api/reference");
  return state.reference;
}

function renderReferenceSection(sectionKey) {
  const payload = state.reference?.sections?.[sectionKey];
  if (!payload) {
    return `<p class="empty">Reference section not available.</p>`;
  }

  return `
    <div class="reference-section-copy">
      <p class="timeline-summary">${escapeHtml(payload.intro)}</p>
    </div>
    <div class="reference-item-list">
      ${payload.items.map((item, index) => `
        <article class="reference-item-card">
          <div>
            <h4>${escapeHtml(item.label)}</h4>
            <p class="timeline-summary">${escapeHtml(item.description)}</p>
            <code>${escapeHtml(item.snippet)}</code>
          </div>
          <div class="reference-item-actions">
            ${item.kind === "query" ? `<button type="button" class="ghost-button compact-button" data-reference-load="${sectionKey}:${index}">Load</button>` : ""}
            ${item.kind === "query" && item.runnable ? `<button type="button" class="ghost-button compact-button" data-reference-run="${sectionKey}:${index}">Run</button>` : ""}
          </div>
        </article>
      `).join("")}
    </div>
  `;
}

function bindReferenceActions() {
  els.modalContainer.querySelectorAll("[data-reference-tab]").forEach((button) => {
    button.addEventListener("click", () => {
      state.uiPrefs.referenceLastSection = button.dataset.referenceTab;
      saveUiPrefs();
      openReferenceCenter();
    });
  });

  els.modalContainer.querySelectorAll("[data-reference-load]").forEach((button) => {
    button.addEventListener("click", () => {
      const [sectionKey, indexRaw] = button.dataset.referenceLoad.split(":");
      const item = state.reference?.sections?.[sectionKey]?.items?.[Number(indexRaw)];
      if (!item) return;
      setEditorValue(item.snippet);
      schedulePersistActiveTabQuery(item.snippet);
      closeModal();
      showToast(`Loaded: ${item.label}`);
    });
  });

  els.modalContainer.querySelectorAll("[data-reference-run]").forEach((button) => {
    button.addEventListener("click", async () => {
      const [sectionKey, indexRaw] = button.dataset.referenceRun.split(":");
      const item = state.reference?.sections?.[sectionKey]?.items?.[Number(indexRaw)];
      if (!item) return;
      setEditorValue(item.snippet);
      schedulePersistActiveTabQuery(item.snippet);
      closeModal();
      await runQuery();
    });
  });
}

async function openReferenceCenter() {
  await ensureReferenceData();
  const sections = state.reference?.sections || {};
  const activeSection = state.uiPrefs.referenceLastSection || "nql_basics";
  const tabs = [
    ["nql_basics", "NQL Basics"],
    ["algorithms", "Algorithms"],
    ["embeddings", "Embeddings"],
    ["examples", "Examples"],
    ["test_db", "Test DB"],
  ];
  showModal(`
    <div class="reference-modal">
      <div class="reference-modal-header">
        <div>
          <p class="panel-kicker">Reference Center</p>
          <h3>Help for NQL and algorithms</h3>
        </div>
        <button data-action="cancel" class="ghost-button compact-button" type="button">Close</button>
      </div>
      <div class="reference-tabs">
        ${tabs.map(([key, label]) => `
          <button type="button" class="ghost-button compact-button ${activeSection === key ? "is-active" : ""}" data-reference-tab="${key}">
            ${label}
          </button>
        `).join("")}
      </div>
      <div class="reference-panel">
        ${renderReferenceSection(activeSection)}
      </div>
    </div>
  `);
  bindReferenceActions();
}

function renderOnboardingCard() {
  const shouldShow = state.workspace.projectOpen
    && !state.uiPrefs.onboardingDismissed
    && (state.workspace.timelineCount || 0) < 2
    && !activeResult();

  els.onboardingCard.hidden = !shouldShow;
  if (!shouldShow) return;

  const starter = starterQuery();
  const projectLabel = state.workspace.dbPath ? "This project is ready." : "Open a project first.";
  els.onboardingText.textContent = `${projectLabel} Start with a simple query, inspect Results, then open Graph or Timeline to deepen the analysis.`;
  els.onboardingLoadButton.dataset.starterQuery = starter;
  els.onboardingRunButton.dataset.starterQuery = starter;
}

function renderGuidedEmptyState(title, description, options = {}) {
  const actions = [];
  if (options.loadStarter) {
    actions.push(`<button type="button" class="ghost-button compact-button" data-guided-action="load-starter">Load starter query</button>`);
  }
  if (options.runStarter) {
    actions.push(`<button type="button" class="ghost-button compact-button" data-guided-action="run-starter">Run starter query</button>`);
  }
  if (options.openReference) {
    actions.push(`<button type="button" class="ghost-button compact-button" data-guided-action="open-reference">Open reference</button>`);
  }
  return `
    <article class="empty-state-card">
      <div>
        <p class="panel-kicker">Start here</p>
        <h4>${escapeHtml(title)}</h4>
        <p class="timeline-summary">${escapeHtml(description)}</p>
      </div>
      ${actions.length ? `<div class="empty-state-actions">${actions.join("")}</div>` : ""}
    </article>
  `;
}

function bindGuidedEmptyStateActions(container) {
  if (!container) return;
  container.querySelectorAll("[data-guided-action]").forEach((button) => {
    button.addEventListener("click", async () => {
      const action = button.dataset.guidedAction;
      if (action === "open-reference") {
        await openReferenceCenter();
        return;
      }
      const query = starterQuery();
      setEditorValue(query);
      schedulePersistActiveTabQuery(query);
      if (action === "run-starter") {
        await runQuery();
      } else {
        showToast("Starter query loaded");
      }
    });
  });
}

function setStatus(label, kind = "accent") {
  els.statusBadge.textContent = label;
  els.statusBadge.className = `badge ${kind === "accent" ? "badge-accent" : ""}`.trim();
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function timelineMatchesFilters(entry) {
  const search = state.uiPrefs.timelineFilter.trim().toLowerCase();
  const matchesSearch =
    !search ||
    entry.query.toLowerCase().includes(search) ||
    (entry.summary || "").toLowerCase().includes(search) ||
    entry.id.toLowerCase().includes(search);
  const matchesMode = !state.uiPrefs.timelineModeFilter || entry.run_mode === state.uiPrefs.timelineModeFilter;
  const matchesPinned = !state.uiPrefs.timelinePinnedOnly || entry.pinned;
  return matchesSearch && matchesMode && matchesPinned;
}

function filteredTimelineEntries() {
  const entries = state.workspace.timeline?.entries || [];
  return entries
    .map((entry, actualIndex) => ({ entry, actualIndex }))
    .filter(({ entry }) => timelineMatchesFilters(entry));
}

function syncSelectedTimelineIndex() {
  const entries = state.workspace.timeline?.entries || [];

  if (state.selectedTimelineRunId) {
    const matchIndex = entries.findIndex((entry) => entry.id === state.selectedTimelineRunId);
    if (matchIndex >= 0) {
      state.selectedTimelineIndex = matchIndex;
      return;
    }
  }

  if (state.selectedTimelineIndex !== null && entries[state.selectedTimelineIndex]) {
    state.selectedTimelineRunId = entries[state.selectedTimelineIndex].id;
    return;
  }

  state.selectedTimelineIndex = entries.length ? 0 : null;
  state.selectedTimelineRunId = entries.length ? entries[0].id : null;
}

function renderWorkspaceTabs() {
  const active = state.uiPrefs.workspaceTab;
  const tabs = [
    [els.tabResults, els.viewResults, "results"],
    [els.tabRaw, els.viewRaw, "raw"],
    [els.tabGraph, els.viewGraph, "graph"],
    [els.tabTimeline, els.viewTimeline, "timeline"],
    [els.tabDetail, els.viewDetail, "detail"],
    [els.tabFindings, els.viewFindings, "findings"],
  ];

  tabs.forEach(([button, view, id]) => {
    const isActive = active === id;
    button.classList.toggle("is-active", isActive);
    view.classList.toggle("is-active", isActive);
    button.setAttribute("aria-selected", String(isActive));
  });
}

function renderSidebar() {
  els.schemaSidebar.classList.toggle("is-collapsed", state.uiPrefs.sidebarCollapsed);
  els.sidebarToggle.textContent = state.uiPrefs.sidebarCollapsed ? "Schema" : "Collapse";
}

function setNavView(view) {
  state.navView = view === "workbench" ? "workbench" : "launcher";
  syncBrowserRoute();
  renderNavigationShell();
}

function desiredRouteForView() {
  return state.navView === "workbench" ? "/workbench" : "/launcher";
}

function syncBrowserRoute() {
  const nextPath = desiredRouteForView();
  const currentPath = window.location.pathname || "/";
  if (currentPath !== nextPath) {
    window.history.pushState({ view: state.navView }, "", nextPath);
  }
}

function syncViewFromLocation() {
  const path = window.location.pathname || "/";
  if (path === "/launcher") {
    state.navView = "launcher";
  } else if (path === "/workbench" && state.workspace.projectOpen) {
    state.navView = "workbench";
  } else {
    state.navView = state.workspace.projectOpen ? "workbench" : "launcher";
  }
}

function renderNavigationShell() {
  const showLauncher = state.navView === "launcher";
  document.body.classList.toggle("launcher-open", showLauncher);
  document.body.classList.toggle("workbench-open", !showLauncher);
  els.projectLauncher.hidden = !showLauncher;
  els.appShell.hidden = showLauncher;
}

function renderProjectList() {
  const projects = state.workspace.projects || [];
  const currentPath = state.workspace.dbPath || "";
  if (!projects.length) {
    els.projectSelect.innerHTML = '<option value="">No projects</option>';
    return;
  }
  els.projectSelect.innerHTML = projects
    .map((p) => {
      const pin = p.pinned ? "\u2605 " : "";
      const selected = p.db_path === currentPath ? " selected" : "";
      return `<option value="${escapeHtml(p.db_path)}" title="${escapeHtml(p.db_path)}"${selected}>${pin}${escapeHtml(p.name)}</option>`;
    })
    .join("");
}

function renderProjectMenu() {
  const projects = state.workspace.projects || [];
  const activeProject = projects.find((project) => project.db_path === state.workspace.dbPath);
  els.projectMenuCurrent.textContent = activeProject?.name || (state.workspace.projectOpen ? "Current project" : "No project");
  els.projectMenuCloseButton.disabled = !state.workspace.projectOpen;
  els.editProjectButton.disabled = !state.workspace.projectOpen;
  els.deleteProjectButton.disabled = !state.workspace.projectOpen;
  els.projectMenuLauncherButton.textContent = state.navView === "launcher" ? "Open workbench" : "Back to launcher";
}

function graphSearchQuery() {
  return (state.graphSearch || "").trim().toLowerCase();
}

function nodeMatchesGraphSearch(node, query) {
  if (!query) return true;
  const haystacks = [
    node.display_label,
    node.label,
    node.entity_type,
    ...(node.properties || []).flatMap((property) => [property.key, property.value]),
  ]
    .filter(Boolean)
    .map((value) => String(value).toLowerCase());
  return haystacks.some((value) => value.includes(query));
}

function graphSearchMatches(subgraph) {
  const query = graphSearchQuery();
  if (!query || !subgraph?.nodes?.length) return [];
  return subgraph.nodes.filter((node) => nodeMatchesGraphSearch(node, query));
}

function centerGraphOnNode(nodeId) {
  const refs = state.graphSvgRefs;
  const position = state.graphNodePositions.get(nodeId);
  if (!refs || !position) return;
  state.graphPanOffset = {
    x: (refs.width / 2 - position.x) * state.graphVisualScale,
    y: (refs.height / 2 - position.y) * state.graphVisualScale,
  };
  updateGraphTransform();
}

function focusGraphSearchMatch(step = 1) {
  const subgraph = state.graphCurrentSubgraph;
  const matches = graphSearchMatches(subgraph);
  if (!matches.length) {
    showToast("No graph matches");
    return;
  }
  const currentIndex = matches.findIndex((node) => node.id === state.selectedGraphNodeId);
  const nextIndex = currentIndex >= 0
    ? (currentIndex + step + matches.length) % matches.length
    : 0;
  state.selectedGraphNodeId = matches[nextIndex].id;
  updateSvgPositions();
  updateGraphDetailPanel();
  centerGraphOnNode(state.selectedGraphNodeId);
  showToast(`Graph match ${nextIndex + 1} of ${matches.length}`);
}

function renderLauncher() {
  renderNavigationShell();

  if (state.navView !== "launcher") {
    return;
  }

  const pendingPath = state.workspace.pendingDbPath || state.uiPrefs.dbDraftPath || "";
  els.launcherPathInput.value = pendingPath;
  const projects = state.workspace.projects || [];

  els.launcherStatus.textContent = state.workspace.pendingDbPath
    ? `Path not found: ${state.workspace.pendingDbPath}. Create a new project here, edit the path, or choose a recent project.`
    : projects.length
      ? "Pick up a recent project or create a new one to continue the workbench."
      : "Choose a recent project or create a new one to start the workbench.";

  if (!projects.length) {
    els.launcherProjectList.innerHTML = '<p class="empty">No projects yet. Create one to start a research workspace.</p>';
    return;
  }

  els.launcherProjectList.innerHTML = projects
    .map((project) => `
      <article class="launcher-project-card ${project.pinned ? "is-pinned" : ""}" data-launcher-path="${escapeHtml(project.db_path)}">
        <div class="launcher-project-header">
          <strong>${escapeHtml(project.name)}</strong>
          ${project.pinned ? '<span class="badge">Pinned</span>' : ""}
        </div>
        <p class="timeline-summary">${escapeHtml(project.description || "No description yet.")}</p>
        <div class="launcher-project-meta">
          <span class="badge">Opened ${escapeHtml(formatProjectTime(project.last_opened_at))}</span>
          <span class="badge">Created ${escapeHtml(formatProjectTime(project.created_at))}</span>
        </div>
        <p class="timeline-summary"><code>${escapeHtml(project.db_path)}</code></p>
      </article>
    `)
    .join("");

  els.launcherProjectList.querySelectorAll("[data-launcher-path]").forEach((node) => {
    node.addEventListener("click", async () => {
      const dbPath = node.dataset.launcherPath;
      if (dbPath) {
        await openDatabase(dbPath);
      }
    });
  });
}

async function loadProjectsSnapshot() {
  try {
    const projects = await api("/api/projects");
    return Array.isArray(projects) ? projects : [];
  } catch {
    return state.workspace.projects || [];
  }
}

function suggestedProjectNameFromPath(dbPath) {
  const trimmed = (dbPath || "").trim();
  if (!trimmed) return "";
  const filename = trimmed.split("/").pop() || trimmed;
  return filename.replace(/\.db$/i, "").replace(/[-_]+/g, " ").trim();
}

function formatProjectTime(value) {
  if (!value) return "recently";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  const diffMs = Date.now() - parsed.getTime();
  const diffHours = Math.max(1, Math.round(diffMs / (1000 * 60 * 60)));
  if (diffHours < 24) return `${diffHours}h ago`;
  const diffDays = Math.round(diffHours / 24);
  if (diffDays < 30) return `${diffDays}d ago`;
  return parsed.toLocaleDateString();
}

function starterQuery() {
  const schema = state.workspace.schema;
  const topNodeType = [...(schema?.node_types || [])].sort((a, b) => b.count - a.count)[0];
  if (topNodeType) {
    return `find n from (n:${topNodeType.name}) limit 25`;
  }
  return "find n from (n) limit 25";
}

function renderSavedQueries() {
  const queries = state.workspace.savedQueries || [];
  if (!queries.length) {
    els.savedQueriesList.innerHTML = '<div class="history-empty">No saved queries</div>';
    return;
  }
  els.savedQueriesList.innerHTML = queries
    .map(
      (q) =>
        `<div class="history-item saved-query-item" data-query-id="${escapeHtml(q.id)}">
          <span class="saved-query-name">${escapeHtml(q.name)}</span>
          <code class="saved-query-preview">${escapeHtml(q.query.substring(0, 80))}</code>
          <button class="ghost-button saved-query-delete" data-query-id="${escapeHtml(q.id)}" type="button">x</button>
        </div>`
    )
    .join("");
}

function applyServerUiPrefs(serverPrefs) {
  if (!serverPrefs) return;
  const mapping = {
    theme: "theme",
    workspace_tab: "workspaceTab",
    run_mode: "runMode",
    graph_layout: "graphLayout",
    graph_depth: "graphDepth",
    graph_limit: "graphLimit",
    graph_type_filter: "graphTypeFilter",
    sidebar_collapsed: "sidebarCollapsed",
  };
  for (const [serverKey, clientKey] of Object.entries(mapping)) {
    if (serverPrefs[serverKey] !== undefined && serverPrefs[serverKey] !== null) {
      state.uiPrefs[clientKey] = serverPrefs[serverKey];
    }
  }
  // Apply theme without triggering backend save (prefs came FROM backend)
  document.documentElement.dataset.theme = state.uiPrefs.theme || "light";
  els.themeToggle.textContent = state.uiPrefs.theme === "dark" ? "Light" : "Dark";
  window.localStorage.setItem(UI_PREFS_KEY, JSON.stringify(state.uiPrefs));
}

function renderGraphTypeFilterOptions(subgraph) {
  const currentValue = state.uiPrefs.graphTypeFilter || "";
  const types = [...new Set((subgraph?.nodes || []).map((node) => node.entity_type || node.label).filter(Boolean))].sort();
  els.graphTypeFilter.innerHTML = ['<option value="">All types</option>']
    .concat(types.map((type) => `<option value="${escapeHtml(type)}">${escapeHtml(type)}</option>`))
    .join("");
  els.graphTypeFilter.value = types.includes(currentValue) ? currentValue : "";
  if (!types.includes(currentValue)) {
    state.uiPrefs.graphTypeFilter = "";
    saveUiPrefs();
  }
}

function renderSessionMeta() {
  if (!state.workspace.projectOpen) {
    if (els.dbPathBadge) els.dbPathBadge.textContent = "db: none";
    els.projectContextBadge.textContent = "No project";
    els.projectHeadline.textContent = "Open a project to start exploring and build a chain of findings.";
    if (els.nextStepBadge) els.nextStepBadge.textContent = "Next: open project";
    if (els.sessionBadge) els.sessionBadge.textContent = "session: launcher";
    return;
  }
  const project = activeProject();
  const displayName = project ? project.name : (state.workspace.dbPath || "none");
  const findingsCount = state.workspace.session?.findings?.length || 0;
  els.projectContextBadge.textContent = displayName;
  els.projectHeadline.textContent = project?.description
    ? project.description
    : `Active project: ${displayName}. Run a query to produce the next finding.`;
  const result = activeResult();
  if (els.nextStepBadge) {
    els.nextStepBadge.textContent = !result
      ? "Next: run query"
      : findingsCount === 0
        ? "Next: capture finding"
        : result.graph_hint
          ? "Next: inspect graph"
          : "Next: review results";
  }
  if (els.dbPathBadge) els.dbPathBadge.textContent = `db: ${displayName}`;
  if (els.sessionBadge) {
    els.sessionBadge.textContent = state.workspace.sessionRestored
      ? `session: restored · ${state.workspace.timelineCount} runs · ${findingsCount} findings`
      : `session: new · ${state.workspace.timelineCount} runs · ${findingsCount} findings`;
  }
}

function describeResultContext(tab, result) {
  const parts = [];
  if (tab?.title) {
    parts.push(`Tab: ${tab.title}`);
  } else {
    parts.push("Active tab");
  }
  parts.push(`Mode: ${result?.run_mode || tab?.last_run_mode || els.runMode.value || "run"}`);
  if (state.selectedTimelineIndex !== null) {
    parts.push(`Run #${state.selectedTimelineIndex + 1}`);
  }
  return parts.join(" · ");
}

function describeResultSummary(result) {
  if (!result) {
    return {
      title: state.workspace.projectOpen ? "Run a query to start." : "Open a project to start.",
      text: state.workspace.projectOpen
        ? "Use Results as the main lens for the current investigation. Start with a focused query."
        : "Choose or create a project first. The workbench will then guide the investigation from Results.",
      capability: "Waiting for results",
      selectionHint: state.workspace.projectOpen
        ? "Run a query to get a first finding."
        : "Open or create a project to continue.",
    };
  }

  const rowCount = result.row_count ?? (result.rows || []).length;
  const hasGraph = Boolean(result.graph_hint);
  const selectedRow = state.selectedResultRowIndex !== null ? state.selectedResultRowIndex + 1 : null;
  const headers = result.headers || [];
  const summaryText = result.error
    ? result.error
    : result.summary || `${rowCount} ${rowCount === 1 ? "row" : "rows"} returned.`;
  const title = rowCount
    ? `${rowCount} ${rowCount === 1 ? "result" : "results"} ready`
    : "Query executed";
  let capability = hasGraph ? "This result can be explored in Graph" : "Tabular result only";
  let selectionHint = hasGraph
    ? "Select a row to focus the current finding in Graph."
    : "Review the table first. This result does not expose graph context yet.";

  if (selectedRow && result.row_graph_hints?.[state.selectedResultRowIndex]) {
    capability = `Row ${selectedRow} is linked to Graph`;
    selectionHint = `Row ${selectedRow} is the current finding. Use Graph to inspect its neighborhood.`;
  } else if (selectedRow) {
    selectionHint = `Row ${selectedRow} is selected. Compare it with the rest of the table or open Run Detail.`;
  }

  if (headers.length && headers.length <= 3 && rowCount > 0 && !result.error) {
    return {
      title,
      text: `${summaryText} Columns: ${headers.join(", ")}.`,
      capability,
      selectionHint,
    };
  }

  return { title, text: summaryText, capability, selectionHint };
}

function updateResultActions(result) {
  const hasResult = Boolean(result);
  const hasGraph = Boolean(result?.graph_hint);
  const hasSelectedRun = state.selectedTimelineIndex !== null;

  els.resultsFocusGraphButton.disabled = !hasResult || !hasGraph;
  els.resultsRunDetailButton.disabled = !hasResult || !hasSelectedRun;
  els.resultsPinRunButton.disabled = !hasResult || !hasSelectedRun;
  els.resultsSaveFindingButton.disabled = !hasResult;
  els.resultsSaveQueryButton.disabled = !activeTab();
  els.resultsOpenTabButton.disabled = !activeTab();
}

function renderResultKnowledgeHub() {
  const tab = activeTab();
  const result = activeResult();
  const description = describeResultSummary(result);

  els.resultsSummaryTitle.textContent = description.title;
  els.resultsSummaryText.textContent = description.text;
  els.resultsSummaryCapability.textContent = description.capability;
  els.resultsSummaryContext.textContent = describeResultContext(tab, result);
  els.resultsSelectionHint.textContent = description.selectionHint;
  updateResultActions(result);
  renderOnboardingCard();
}

function renderFindingsPane() {
  const findings = state.workspace.session?.findings || [];
  const project = activeProject();

  if (els.projectNotesInput) {
    els.projectNotesInput.value = project?.notes || "";
    els.projectNotesInput.disabled = !state.workspace.projectOpen;
  }
  if (els.projectNotesSaveButton) {
    els.projectNotesSaveButton.disabled = !state.workspace.projectOpen;
  }
  if (els.findingsMeta) {
    els.findingsMeta.textContent = `${findings.length} finding${findings.length === 1 ? "" : "s"}`;
  }

  if (!state.workspace.projectOpen) {
    els.findingsPane.innerHTML = renderGuidedEmptyState(
      "Open a project first",
      "Findings live inside a project session so you can keep notes linked to runs, tabs and graph context.",
      { openReference: true }
    );
    bindGuidedEmptyStateActions(els.findingsPane);
    return;
  }

  if (!findings.length) {
    els.findingsPane.innerHTML = renderGuidedEmptyState(
      "No findings yet",
      "Capture the current result or a selected run to keep track of what matters and why it matters.",
      { runStarter: !activeResult(), openReference: true }
    );
    bindGuidedEmptyStateActions(els.findingsPane);
    return;
  }

  els.findingsPane.innerHTML = findings.map((finding) => {
    const meta = [
      finding.run_id ? `Run ${finding.run_id}` : null,
      finding.tab_id ? `Tab linked` : null,
      typeof finding.row_index === "number" ? `Row ${finding.row_index + 1}` : null,
      finding.updated_at ? `Updated ${finding.updated_at.slice(0, 16).replace("T", " ")}` : null,
    ].filter(Boolean).join(" · ");

    return `
      <article class="finding-card" data-finding-id="${escapeHtml(finding.id)}">
        <div class="finding-card-header">
          <div>
            <p class="panel-kicker">Finding</p>
            <h4>${escapeHtml(finding.title)}</h4>
            <p class="timeline-summary">${escapeHtml(meta || "Project note")}</p>
          </div>
          <div class="finding-card-actions">
            <button type="button" class="ghost-button compact-button" data-finding-load="${escapeHtml(finding.id)}">Load query</button>
            <button type="button" class="ghost-button compact-button" data-finding-edit="${escapeHtml(finding.id)}">Edit</button>
            <button type="button" class="ghost-button compact-button" data-finding-delete="${escapeHtml(finding.id)}">Delete</button>
          </div>
        </div>
        ${finding.summary ? `<p class="timeline-summary">${escapeHtml(finding.summary)}</p>` : ""}
        <div class="finding-body">${escapeHtml(finding.body || "No written note yet.").replaceAll("\n", "<br />")}</div>
      </article>
    `;
  }).join("");

  els.findingsPane.querySelectorAll("[data-finding-load]").forEach((button) => {
    button.addEventListener("click", () => {
      const finding = findings.find((item) => item.id === button.dataset.findingLoad);
      if (!finding?.query_text) {
        showToast("This finding has no query attached");
        return;
      }
      setEditorValue(finding.query_text);
      schedulePersistActiveTabQuery(finding.query_text);
      setWorkspaceTab("results");
      showToast("Finding query loaded");
    });
  });
  els.findingsPane.querySelectorAll("[data-finding-edit]").forEach((button) => {
    button.addEventListener("click", () => openFindingDialog(button.dataset.findingEdit));
  });
  els.findingsPane.querySelectorAll("[data-finding-delete]").forEach((button) => {
    button.addEventListener("click", async () => {
      try {
        await api(`/api/findings/${encodeURIComponent(button.dataset.findingDelete)}`, { method: "DELETE" });
        await refreshWorkbench();
        showToast("Finding removed");
      } catch (error) {
        showToast(`Delete failed: ${error.message}`);
      }
    });
  });
}

function openFindingDialog(findingId = null) {
  const finding = findingId
    ? (state.workspace.session?.findings || []).find((item) => item.id === findingId)
    : null;
  const draft = finding || activeFindingContext();
  showModal(`
    <div class="modal-shell">
      <div class="modal-header">
        <div>
          <p class="panel-kicker">Knowledge capture</p>
          <h3>${finding ? "Edit finding" : "Capture finding"}</h3>
        </div>
        <button data-action="cancel" class="ghost-button compact-button" type="button">Close</button>
      </div>
      <label class="field-label" for="findingTitleInput">Title</label>
      <input id="findingTitleInput" class="path-input" type="text" value="${escapeHtml(draft.title || "")}" />
      <label class="field-label" for="findingBodyInput">Note</label>
      <textarea id="findingBodyInput" class="path-input" rows="8">${escapeHtml(draft.body || "")}</textarea>
      <p class="timeline-summary">${escapeHtml(draft.summary || "This finding will be linked to the current tab and, when available, the selected run/row.")}</p>
      <div class="modal-actions">
        <button id="findingSaveConfirm" class="primary-button" type="button">${finding ? "Save changes" : "Save finding"}</button>
      </div>
    </div>
  `);

  els.modalContainer.querySelector("#findingSaveConfirm")?.addEventListener("click", async () => {
    const title = els.modalContainer.querySelector("#findingTitleInput")?.value || "";
    const body = els.modalContainer.querySelector("#findingBodyInput")?.value || "";
    try {
      if (finding) {
        await api(`/api/findings/${encodeURIComponent(finding.id)}`, {
          method: "PUT",
          body: JSON.stringify({ title, body }),
        });
      } else {
        const context = activeFindingContext();
        await api("/api/findings", {
          method: "POST",
          body: JSON.stringify({
            title,
            body,
            tab_id: context.tabId,
            run_id: context.runId,
            query_text: context.queryText,
            summary: context.summary,
            row_index: context.rowIndex,
            graph_focus_node_id: context.graphFocusNodeId,
          }),
        });
      }
      closeModal();
      await refreshWorkbench();
      setWorkspaceTab("findings");
      showToast(finding ? "Finding updated" : "Finding saved");
    } catch (error) {
      showToast(`Finding save failed: ${error.message}`);
    }
  });
}

function renderTable(headers = [], rows = []) {
  els.resultsHead.innerHTML = "";
  els.resultsBody.innerHTML = "";

  if (!headers.length) {
    state.selectedResultRowIndex = null;
    state.resultSortColumn = null;
    state.resultColumnFilters = {};
    els.resultsHead.innerHTML = "<tr><th>result</th></tr>";
    els.resultsBody.innerHTML = `<tr><td>${renderGuidedEmptyState(
      "No results yet",
      state.workspace.projectOpen
        ? "Run a starter query to begin the investigation, or open the reference center for quick NQL examples."
        : "Open a project first. Results will become the main lens for your current finding.",
      { loadStarter: state.workspace.projectOpen, runStarter: state.workspace.projectOpen, openReference: true }
    )}</td></tr>`;
    bindGuidedEmptyStateActions(els.resultsBody);
    return;
  }

  // T1-2: Sortable headers
  const head = document.createElement("tr");
  headers.forEach((header, colIndex) => {
    const th = document.createElement("th");
    th.textContent = header;
    th.className = "sortable";
    if (state.resultSortColumn === colIndex) {
      th.classList.add(state.resultSortDirection === "asc" ? "sort-asc" : "sort-desc");
    }
    th.addEventListener("click", () => {
      if (state.resultSortColumn === colIndex) {
        state.resultSortDirection = state.resultSortDirection === "asc" ? "desc" : "asc";
      } else {
        state.resultSortColumn = colIndex;
        state.resultSortDirection = "asc";
      }
      renderActiveTabResult();
    });
    head.appendChild(th);
  });
  els.resultsHead.appendChild(head);

  // T2-4: Column filter row
  const filterRow = document.createElement("tr");
  filterRow.className = "column-filter-row";
  headers.forEach((header, colIndex) => {
    const th = document.createElement("th");
    const input = document.createElement("input");
    input.type = "text";
    input.className = "column-filter-input";
    input.placeholder = "Filter...";
    input.value = state.resultColumnFilters[colIndex] || "";
    input.addEventListener("input", (e) => {
      state.resultColumnFilters[colIndex] = e.target.value;
      renderActiveTabResult();
    });
    th.appendChild(input);
    filterRow.appendChild(th);
  });
  els.resultsHead.appendChild(filterRow);

  // Apply column filters
  let filteredRows = rows.map((row, originalIndex) => ({ row, originalIndex }));
  for (const [colIdx, filterVal] of Object.entries(state.resultColumnFilters)) {
    const lowerFilter = (filterVal || "").toLowerCase();
    if (lowerFilter) {
      filteredRows = filteredRows.filter(({ row }) =>
        String(row[Number(colIdx)] ?? "").toLowerCase().includes(lowerFilter)
      );
    }
  }

  // Apply sort
  if (state.resultSortColumn !== null && state.resultSortColumn < headers.length) {
    const col = state.resultSortColumn;
    const dir = state.resultSortDirection === "asc" ? 1 : -1;
    filteredRows.sort((a, b) => {
      const va = a.row[col] ?? "";
      const vb = b.row[col] ?? "";
      const na = Number(va);
      const nb = Number(vb);
      if (!isNaN(na) && !isNaN(nb)) return (na - nb) * dir;
      return String(va).localeCompare(String(vb)) * dir;
    });
  }

  if (state.selectedResultRowIndex !== null && !rows[state.selectedResultRowIndex]) {
    state.selectedResultRowIndex = null;
  }

  // Update meta with filter info
  const hasFilters = Object.values(state.resultColumnFilters).some((v) => v);
  const filterSuffix = hasFilters ? ` (${filteredRows.length} of ${rows.length} filtered)` : "";

  filteredRows.forEach(({ row, originalIndex }) => {
    const tr = document.createElement("tr");
    tr.dataset.rowIndex = String(originalIndex);
    tr.classList.toggle("is-active", state.selectedResultRowIndex === originalIndex);
    row.forEach((cell) => {
      const td = document.createElement("td");
      td.textContent = cell;
      // T1-3: Double-click to copy cell value
      td.addEventListener("dblclick", () => {
        navigator.clipboard.writeText(String(cell)).then(() => showToast("Copied")).catch(() => {});
      });
      tr.appendChild(td);
    });
    els.resultsBody.appendChild(tr);
  });

  els.resultsBody.querySelectorAll("[data-row-index]").forEach((rowNode) => {
    rowNode.addEventListener("click", () => {
      const rowIndex = Number(rowNode.dataset.rowIndex);
      selectResultRow(rowIndex);
    });
  });

  return filterSuffix;
}

function renderStats(schema) {
  if (!schema) {
    els.statsPane.innerHTML = `<p class="empty">Stats not loaded.</p>`;
    return;
  }

  els.statsMeta.textContent = `${schema.total_nodes} nodes`;
  els.statsPane.innerHTML = `
    <div class="stats-grid">
      <article class="stat-card">
        <span class="panel-kicker">Nodes</span>
        <strong>${schema.total_nodes}</strong>
      </article>
      <article class="stat-card">
        <span class="panel-kicker">Edges</span>
        <strong>${schema.total_edges}</strong>
      </article>
      <article class="stat-card">
        <span class="panel-kicker">Avg degree</span>
        <strong>${schema.avg_degree.toFixed(2)}</strong>
      </article>
      <article class="stat-card">
        <span class="panel-kicker">Density</span>
        <strong>${schema.density.toFixed(4)}</strong>
      </article>
    </div>
  `;
}

function renderSchema(schema) {
  if (!schema) {
    els.schemaPane.innerHTML = `<p class="empty">Schema not loaded.</p>`;
    return;
  }

  const groups = [];
  groups.push(`
    <article class="schema-group">
      <h3>${escapeHtml(schema.db_name)}</h3>
      <p class="timeline-summary">${schema.node_types.length} node types · ${schema.edge_types.length} edge types</p>
    </article>
  `);

  for (const nodeType of schema.node_types) {
    groups.push(`
      <article class="schema-group">
        <h3 class="schema-type-link" data-schema-query="find n from (n:${escapeHtml(nodeType.name)}) limit 25">Node: ${escapeHtml(nodeType.name)}</h3>
        <p class="timeline-summary">${nodeType.count} records</p>
        <ul>${nodeType.properties.map((prop) => `<li>${escapeHtml(prop)}</li>`).join("") || '<li class="empty">No properties</li>'}</ul>
      </article>
    `);
  }

  for (const edgeType of schema.edge_types) {
    groups.push(`
      <article class="schema-group">
        <h3 class="schema-type-link" data-schema-query="find s, r, t from (s) -[r:${escapeHtml(edgeType.name)}]-> (t) limit 25">Edge: ${escapeHtml(edgeType.name)}</h3>
        <p class="timeline-summary">${edgeType.count} relations</p>
        <ul>${edgeType.properties.map((prop) => `<li>${escapeHtml(prop)}</li>`).join("") || '<li class="empty">No properties</li>'}</ul>
      </article>
    `);
  }

  els.schemaMeta.textContent = `${schema.node_types.length + schema.edge_types.length} types`;
  els.schemaPane.innerHTML = groups.join("");

  // T1-7: Click schema type to generate and execute exploration query
  els.schemaPane.querySelectorAll("[data-schema-query]").forEach((link) => {
    link.addEventListener("click", async () => {
      const query = link.dataset.schemaQuery;
      setEditorValue(query);
      schedulePersistActiveTabQuery(query);
      await runQuery();
    });
  });
}

function buildSuggestedQueries(schema) {
  if (!schema) return [];

  const suggestions = [];
  const topNodeTypes = [...(schema.node_types || [])]
    .sort((a, b) => b.count - a.count)
    .slice(0, 2);
  const topEdgeType = [...(schema.edge_types || [])]
    .sort((a, b) => b.count - a.count)[0];

  if (topNodeTypes[0]) {
    suggestions.push({
      title: `Explore ${topNodeTypes[0].name}`,
      summary: `Start with the most populated node type to build context quickly.`,
      query: `find n from (n:${topNodeTypes[0].name}) limit 25`,
    });
  }

  if (topEdgeType) {
    suggestions.push({
      title: `Inspect ${topEdgeType.name} relations`,
      summary: `Look at a representative relationship pattern from the current dataset.`,
      query: `find s, r, t from (s)-[r:${topEdgeType.name}]->(t) limit 25`,
    });
  }

  if (topNodeTypes[0]?.properties?.length) {
    const prop = topNodeTypes[0].properties[0];
    suggestions.push({
      title: `Sample ${topNodeTypes[0].name}.${prop}`,
      summary: `Use a simple projection to understand the most visible property on this type.`,
      query: `find n.${prop} from (n:${topNodeTypes[0].name}) limit 25`,
    });
  }

  if (topNodeTypes[1]) {
    suggestions.push({
      title: `Compare ${topNodeTypes[0]?.name || "nodes"} and ${topNodeTypes[1].name}`,
      summary: `Open a second type to contrast labels, properties and result shapes.`,
      query: `find n from (n:${topNodeTypes[1].name}) limit 25`,
    });
  }

  return suggestions.slice(0, 4);
}

function renderSuggestedQueries(schema) {
  const suggestions = buildSuggestedQueries(schema);
  els.suggestedQueriesMeta.textContent = `${suggestions.length} ideas`;

  if (!suggestions.length) {
    els.suggestedQueriesPane.innerHTML = `<p class="empty">Suggested queries will appear once the schema is loaded.</p>`;
    return;
  }

  els.suggestedQueriesPane.innerHTML = suggestions
    .map(
      (item, index) => `
        <article class="suggested-query-card">
          <div>
            <h4>${escapeHtml(item.title)}</h4>
            <p class="timeline-summary">${escapeHtml(item.summary)}</p>
            <code>${escapeHtml(item.query)}</code>
          </div>
          <div class="suggested-query-actions">
            <button type="button" class="ghost-button compact-button" data-suggested-load="${index}">Load</button>
            <button type="button" class="ghost-button compact-button" data-suggested-run="${index}">Run</button>
          </div>
        </article>
      `
    )
    .join("");

  els.suggestedQueriesPane.querySelectorAll("[data-suggested-load]").forEach((button) => {
    button.addEventListener("click", () => {
      const item = suggestions[Number(button.dataset.suggestedLoad)];
      if (!item) return;
      setEditorValue(item.query);
      schedulePersistActiveTabQuery(item.query);
      showToast(`Loaded: ${item.title}`);
    });
  });

  els.suggestedQueriesPane.querySelectorAll("[data-suggested-run]").forEach((button) => {
    button.addEventListener("click", async () => {
      const item = suggestions[Number(button.dataset.suggestedRun)];
      if (!item) return;
      setEditorValue(item.query);
      schedulePersistActiveTabQuery(item.query);
      await runQuery();
    });
  });
}

function renderQueryTabs() {
  const session = state.workspace.session;
  const tabs = session?.tabs || [];
  if (!tabs.length) {
    els.queryTabs.innerHTML = `<span class="empty">No tabs.</span>`;
    return;
  }

  els.queryTabs.innerHTML = tabs
    .map((tab) => {
      const isActive = tab.id === session.active_tab_id;
      const label = tab.title || "Query";
      const stamp = tab.last_executed_at ? new Date(tab.last_executed_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : "new";
      return `
        <button type="button" class="query-tab ${isActive ? "is-active" : ""}" data-tab-id="${escapeHtml(tab.id)}">
          <span class="query-tab-title">${escapeHtml(label)}</span>
          <span class="query-tab-meta">${escapeHtml(stamp)}</span>
        </button>
      `;
    })
    .join("");

  els.queryTabs.querySelectorAll("[data-tab-id]").forEach((button) => {
    button.addEventListener("click", async () => {
      await activateQueryTab(button.dataset.tabId);
    });
  });
}

function syncEditorFromActiveTab() {
  const tab = activeTab();
  if (!tab) {
    setEditorValue("");
    return;
  }
  setEditorValue(tab.query_text || "");
  const runMode = tab.last_run_mode || state.uiPrefs.runMode || "run";
  els.runMode.value = runMode;
}

function ensureSelectedResultRow(result) {
  const rowHints = result?.row_graph_hints || [];
  if (!result?.rows?.length) {
    state.selectedResultRowIndex = null;
    return;
  }
  if (state.selectedResultRowIndex !== null && result.rows[state.selectedResultRowIndex]) {
    return;
  }
  const firstGraphRow = rowHints.findIndex((hint) => Boolean(hint));
  state.selectedResultRowIndex = firstGraphRow >= 0 ? firstGraphRow : 0;
}

function selectResultRow(rowIndex) {
  const result = activeResult();
  if (!result?.rows?.[rowIndex]) {
    return;
  }

  state.selectedResultRowIndex = rowIndex;
  const rowHint = result.row_graph_hints?.[rowIndex] || null;
  if (rowHint) {
    state.selectedGraphNodeId = rowHint.focus_node_id || rowHint.node_ids?.[0] || state.selectedGraphNodeId;
  }

  renderActiveTabResult();
  // Update graph selection highlight without full rebuild
  if (state.graphSvgRefs) {
    updateSvgPositions();
    updateGraphDetailPanel();
  }
}

function renderActiveTabResult() {
  const result = activeResult();
  if (!result) {
    renderTable([], []);
    els.resultMeta.textContent = "0 rows";
    if (els.rawContent) els.rawContent.textContent = "No result";
    renderResultKnowledgeHub();
    return;
  }

  ensureSelectedResultRow(result);
  const filterSuffix = renderTable(result.headers || [], result.rows || []) || "";
  const selectedSuffix =
    state.selectedResultRowIndex !== null ? ` · row ${state.selectedResultRowIndex + 1} selected` : "";
  const rowHint = state.selectedResultRowIndex !== null ? result.row_graph_hints?.[state.selectedResultRowIndex] : null;
  const focusSuffix = rowHint ? " · graph-linked" : "";
  els.resultMeta.textContent = `${result.row_count ?? (result.rows || []).length} rows${filterSuffix}${selectedSuffix}${focusSuffix}`;
  if (els.rawContent) els.rawContent.textContent = JSON.stringify(result, null, 2);
  renderResultKnowledgeHub();
}

function syncRunSummaryFromActiveTab() {
  const tab = activeTab();
  const result = tab?.last_result || null;
  if (!result) {
    els.runSummary.textContent = "Ready.";
    return;
  }

  const summary = result.error || result.summary || "Ready.";
  const duration = result.duration_ms ? ` · ${result.duration_ms.toFixed(1)} ms` : "";
  els.runSummary.textContent = `${summary}${duration}`;
}

function graphRelations(subgraph, focusNode) {
  return subgraph.edges
    .filter((edge) => edge.source === focusNode.id || edge.target === focusNode.id)
    .map((edge) => {
      const outgoing = edge.source === focusNode.id;
      const neighborId = outgoing ? edge.target : edge.source;
      const neighbor = subgraph.nodes.find((node) => node.id === neighborId);
      return {
        edgeType: edge.edge_type,
        direction: outgoing ? "outgoing" : "incoming",
        neighborId,
        neighborLabel: neighbor?.display_label || neighbor?.label || "Unknown",
        neighborType: neighbor?.entity_type || neighbor?.label || "Unknown",
      };
    });
}

function selectedGraphNode(subgraph) {
  return subgraph?.nodes?.find((node) => node.id === state.selectedGraphNodeId) || null;
}

function renderEdgeDetailCard(edge) {
  let html = `<div class="detail-card"><h5 class="detail-card-title">${escapeHtml(edge.edge_type || "Edge")}</h5>`;
  html += `<table class="props-table"><tbody>`;
  html += `<tr><th>Source</th><td><code class="uuid">${escapeHtml(edge.source)}</code></td></tr>`;
  html += `<tr><th>Target</th><td><code class="uuid">${escapeHtml(edge.target)}</code></td></tr>`;
  if (edge.properties && Object.keys(edge.properties).length > 0) {
    for (const [k, v] of Object.entries(edge.properties)) {
      html += `<tr><th>${escapeHtml(k)}</th><td>${formatPropValue(v)}</td></tr>`;
    }
  }
  html += `</tbody></table></div>`;
  return html;
}

function renderNodeDetailCard(node, relations) {
  const properties = node.properties || [];
  const headline = node.display_label || node.label;
  const orderedProperties = [...properties].sort((left, right) => {
    const priority = ["name", "title", "display_name", "iri", "code"];
    const leftPriority = priority.indexOf(left.key);
    const rightPriority = priority.indexOf(right.key);
    return (leftPriority >= 0 ? leftPriority : 999) - (rightPriority >= 0 ? rightPriority : 999)
      || left.key.localeCompare(right.key);
  });
  const incoming = relations.filter((relation) => relation.direction === "incoming");
  const outgoing = relations.filter((relation) => relation.direction === "outgoing");
  const renderRelationList = (title, list) => `
    <article class="detail-card graph-neighbor-card">
      <h3>${title}</h3>
      ${
        list.length
          ? list
              .map(
                (rel) => `
                  <div class="graph-rel">
                    <div>
                      <div><strong>${escapeHtml(rel.neighborLabel)}</strong></div>
                      <p class="timeline-summary">${escapeHtml(rel.neighborType)} · <code>${escapeHtml(rel.edgeType)}</code></p>
                    </div>
                    <button type="button" data-focus-node="${escapeHtml(rel.neighborId)}">Focus</button>
                  </div>
                `
              )
              .join("")
          : `<p class="empty">No ${title.toLowerCase()} relations in the visible subgraph.</p>`
      }
    </article>
  `;
  return `
    <article class="detail-card graph-node-detail">
      <h3>Selected Node</h3>
      <p><strong>${escapeHtml(headline)}</strong></p>
      <div class="node-summary-chips">
        <span class="legend-chip legend-chip-target">${escapeHtml(node.entity_type || node.label)}</span>
        <span class="legend-chip">${relations.length} visible relations</span>
        <span class="legend-chip">degree ${node.degree ?? "?"}</span>
      </div>
      <p><code>${escapeHtml(node.id)}</code></p>
      <div class="node-property-list">
        ${
          orderedProperties.length
            ? orderedProperties
                .map(
                  (property) => `
                    <div class="node-property-row">
                      <span class="node-property-key">${escapeHtml(property.key)}</span>
                      <span class="node-property-value">${escapeHtml(property.value)}</span>
                    </div>
                  `
                )
                .join("")
            : `<p class="empty">No properties on this node.</p>`
        }
      </div>
      <p class="visual-hint">Use the relation cards to refocus without losing the current result context.</p>
    </article>
    <div class="graph-neighbor-columns">
      ${renderRelationList("Outgoing", outgoing)}
      ${renderRelationList("Incoming", incoming)}
    </div>
  `;
}

function buildResultFocusSubgraph() {
  const graphHint = currentGraphHint();
  const graph = state.workspace.graph;
  if (!graphHint || !graph) {
    return null;
  }

  const limit = Math.max(10, Number(state.uiPrefs.graphLimit || 50));
  const nodeMap = new Map((graph.nodes || []).map((node) => [node.id, node]));
  const edgeMap = new Map();
  const selectedNodeIds = new Set(graphHint.node_ids || []);

  for (const explicitEdge of graphHint.edges || []) {
    selectedNodeIds.add(explicitEdge.source);
    selectedNodeIds.add(explicitEdge.target);
    edgeMap.set(`${explicitEdge.source}->${explicitEdge.target}`, {
      source: explicitEdge.source,
      target: explicitEdge.target,
      edge_type: "result",
    });
  }

  for (const edge of graph.edges || []) {
    const touchesSelected = selectedNodeIds.has(edge.source) || selectedNodeIds.has(edge.target);
    if (!touchesSelected) continue;
    if (selectedNodeIds.size < limit) {
      selectedNodeIds.add(edge.source);
      selectedNodeIds.add(edge.target);
    }
    edgeMap.set(`${edge.source}->${edge.target}:${edge.edge_type}`, edge);
    if (selectedNodeIds.size >= limit) break;
  }

  const nodes = Array.from(selectedNodeIds)
    .slice(0, limit)
    .map((id) => nodeMap.get(id) || {
      id,
      label: "Result",
      display_label: "Result Node",
      entity_type: "Result",
      properties: [],
      degree: null,
    })
    .filter(Boolean);

  const visibleNodeIds = new Set(nodes.map((node) => node.id));
  const visibleEdges = Array.from(edgeMap.values()).filter(
    (edge) => visibleNodeIds.has(edge.source) && visibleNodeIds.has(edge.target)
  );

  if (!nodes.length) {
    return null;
  }

  return {
    focus_node_id: graphHint.focus_node_id || nodes[0].id,
    depth: 1,
    truncated: selectedNodeIds.size > nodes.length,
    nodes,
    edges: visibleEdges,
    note: graphHint.note || "Derived from the active query result.",
    source: "result-focus",
  };
}

function mergeExpandedNodes(subgraph) {
  if (!state.graphExpandedNodes.length && !state.graphExpandedEdges.length) return subgraph;
  const existingIds = new Set(subgraph.nodes.map((n) => n.id));
  const newNodes = state.graphExpandedNodes.filter((n) => !existingIds.has(n.id));
  const allIds = new Set([...existingIds, ...newNodes.map((n) => n.id)]);
  const existingEdgeKeys = new Set(subgraph.edges.map((e) => `${e.source}->${e.target}:${e.edge_type}`));
  const newEdges = state.graphExpandedEdges.filter((e) =>
    !existingEdgeKeys.has(`${e.source}->${e.target}:${e.edge_type}`) && allIds.has(e.source) && allIds.has(e.target)
  );
  if (!newNodes.length && !newEdges.length) return subgraph;
  return { ...subgraph, nodes: [...subgraph.nodes, ...newNodes], edges: [...subgraph.edges, ...newEdges] };
}

function currentGraphData() {
  const base = state.workspace.graphSubgraph;
  if (!base) return null;

  const graphHint = currentGraphHint();
  const graph = state.workspace.graph;
  if (!graphHint || !graph) return mergeExpandedNodes(base);

  const resultNodeIds = new Set(graphHint.node_ids || []);
  if (!resultNodeIds.size) return mergeExpandedNodes(base);

  // Build a focused subgraph: result nodes + their direct neighbors
  const allNodes = new Map((graph.nodes || []).map((n) => [n.id, n]));
  const allEdges = graph.edges || [];

  // Collect neighbor IDs (nodes connected to result nodes by an edge)
  const neighborIds = new Set();
  const relevantEdges = [];
  const edgeKeys = new Set();
  for (const edge of allEdges) {
    const srcIsResult = resultNodeIds.has(edge.source);
    const tgtIsResult = resultNodeIds.has(edge.target);
    if (srcIsResult || tgtIsResult) {
      const key = `${edge.source}->${edge.target}:${edge.edge_type}`;
      if (!edgeKeys.has(key)) {
        edgeKeys.add(key);
        relevantEdges.push(edge);
      }
      if (srcIsResult) neighborIds.add(edge.target);
      if (tgtIsResult) neighborIds.add(edge.source);
    }
  }

  // Also add graph_hint explicit edges
  for (const edge of graphHint.edges || []) {
    const key = `${edge.source}->${edge.target}:${edge.edge_type || "result"}`;
    if (!edgeKeys.has(key)) {
      edgeKeys.add(key);
      relevantEdges.push(edge);
    }
    neighborIds.add(edge.source);
    neighborIds.add(edge.target);
  }

  // Build node list: result nodes first, then neighbors
  const visibleIds = new Set([...resultNodeIds, ...neighborIds]);
  const nodes = [];
  for (const id of visibleIds) {
    const node = allNodes.get(id);
    if (node) nodes.push(node);
  }

  // Filter edges to only those between visible nodes
  const edges = relevantEdges.filter((e) => visibleIds.has(e.source) && visibleIds.has(e.target));

  if (!nodes.length) return mergeExpandedNodes(base);

  const focused = {
    focus_node_id: graphHint.focus_node_id || [...resultNodeIds][0] || nodes[0]?.id,
    depth: 1,
    truncated: false,
    nodes,
    edges,
    _resultNodeIds: resultNodeIds,
    note: "Focused on query result nodes and their neighbors.",
    source: "result-focus",
  };
  return mergeExpandedNodes(focused);
}

function describeGraphMode(subgraph) {
  if (!state.workspace.projectOpen) {
    return {
      title: "Open a project to load graph context",
      text: "Graph becomes useful after a project is open and a dataset is available.",
      source: "Source: launcher",
      scope: "Scope: no graph loaded",
    };
  }

  const result = activeResult();
  const rowHint =
    state.selectedResultRowIndex !== null
      ? result?.row_graph_hints?.[state.selectedResultRowIndex] || null
      : null;
  const graphHint = result?.graph_hint || null;
  const tabLabel = activeTab()?.title || "Active tab";
  const nodeCount = subgraph?.nodes?.length || 0;
  const edgeCount = subgraph?.edges?.length || 0;

  if (rowHint) {
    return {
      title: "Row Focus",
      text: `The graph is centered on the selected row from ${tabLabel}. This is the current finding in context.`,
      source: `Source: row ${state.selectedResultRowIndex + 1} · ${tabLabel}`,
      scope: `Scope: ${nodeCount} nodes · ${edgeCount} edges`,
    };
  }

  if (graphHint) {
    return {
      title: "Result Focus",
      text: `The graph is following the current query result from ${tabLabel}. Use it to inspect the result in context.`,
      source: `Source: result set · ${tabLabel}`,
      scope: `Scope: ${nodeCount} nodes · ${edgeCount} edges`,
    };
  }

  return {
    title: "Dataset view",
    text: "The graph shows the broader dataset context. Run a graph-compatible query to narrow the focus.",
    source: "Source: dataset",
    scope: `Scope: ${nodeCount} nodes · ${edgeCount} edges`,
  };
}

function renderGraphModeBanner(subgraph) {
  const mode = describeGraphMode(subgraph);
  els.graphModeTitle.textContent = mode.title;
  els.graphModeText.textContent = mode.text;
  els.graphModeSource.textContent = mode.source;
  els.graphModeScope.textContent = mode.scope;
}

function applyGraphTypeFilter(subgraph) {
  const typeFilter = (state.uiPrefs.graphTypeFilter || "").trim();
  if (!subgraph || !typeFilter) {
    return subgraph;
  }
  const nodes = (subgraph.nodes || []).filter((node) => (node.entity_type || node.label) === typeFilter);
  if (!nodes.length) {
    return { ...subgraph, nodes: [], edges: [] };
  }
  const allowedIds = new Set(nodes.map((node) => node.id));
  const edges = (subgraph.edges || []).filter((edge) => allowedIds.has(edge.source) && allowedIds.has(edge.target));
  const focusNodeId = allowedIds.has(subgraph.focus_node_id) ? subgraph.focus_node_id : nodes[0]?.id || null;
  return { ...subgraph, nodes, edges, focus_node_id: focusNodeId };
}

function renderGraphPane() {
  refreshGraphDirtyState();
  if (typeof teardownGraph === 'function') teardownGraph();

  const unfilteredSubgraph = currentGraphData();
  renderGraphModeBanner(unfilteredSubgraph);
  renderGraphTypeFilterOptions(unfilteredSubgraph);
  let subgraph = applyGraphTypeFilter(unfilteredSubgraph);

  // Filter hidden nodes
  if (subgraph && state.graphHiddenNodes.size > 0) {
    const nodes = subgraph.nodes.filter((n) => !state.graphHiddenNodes.has(n.id));
    const nodeIds = new Set(nodes.map((n) => n.id));
    const edges = subgraph.edges.filter((e) => nodeIds.has(e.source) && nodeIds.has(e.target));
    subgraph = { ...subgraph, nodes, edges };
  }

  els.layoutRadialButton.classList.toggle("is-active", state.uiPrefs.graphLayout === "radial");
  els.layoutForceButton.classList.toggle("is-active", state.uiPrefs.graphLayout === "force");

  if (!subgraph) {
    renderGraphModeBanner(subgraph);
    els.graphMeta.textContent = "0 nodes";
    els.graphSearchMeta.textContent = graphSearchQuery() ? "Search: waiting for graph" : "Search: off";
    els.graphPane.innerHTML = renderGuidedEmptyState(
      "Graph is waiting for context",
      state.workspace.projectOpen
        ? "Run a graph-compatible query or reload the dataset to inspect connections."
        : "Open a project first, then run a query to explore graph structure.",
      { loadStarter: state.workspace.projectOpen, runStarter: state.workspace.projectOpen, openReference: true }
    );
    bindGuidedEmptyStateActions(els.graphPane);
    els.graphLegend.innerHTML = "";
    els.graphDetailPanel.classList.add("is-empty");
    state.graphCurrentSubgraph = null;
    state.graphSvgRefs = null;
    return;
  }

  if (!subgraph.nodes.length) {
    renderGraphModeBanner(subgraph);
    els.graphMeta.textContent = "0 nodes";
    els.graphSearchMeta.textContent = graphSearchQuery() ? "Search: 0 matches" : "Search: off";
    els.graphPane.innerHTML = renderGuidedEmptyState(
      "No nodes match this graph view",
      "Try changing the filters, or run a different query to produce a graph-focused result.",
      { runStarter: true, openReference: true }
    );
    bindGuidedEmptyStateActions(els.graphPane);
    els.graphLegend.innerHTML = "";
    els.graphDetailPanel.classList.add("is-empty");
    state.graphCurrentSubgraph = null;
    state.graphSvgRefs = null;
    return;
  }

  if (!state.selectedGraphNodeId || !subgraph.nodes.some((n) => n.id === state.selectedGraphNodeId)) {
    state.selectedGraphNodeId = subgraph.focus_node_id || subgraph.nodes[0]?.id || null;
  }
  const searchMatchesInGraph = graphSearchMatches(subgraph);
  if (graphSearchQuery() && searchMatchesInGraph.length && !searchMatchesInGraph.some((node) => node.id === state.selectedGraphNodeId)) {
    state.selectedGraphNodeId = searchMatchesInGraph[0].id;
  }

  const filterLabel = state.uiPrefs.graphTypeFilter ? ` · ${state.uiPrefs.graphTypeFilter}` : "";
  const resultCount = subgraph._resultNodeIds ? subgraph._resultNodeIds.size : 0;
  const resultLabel = resultCount > 0 ? ` · ${resultCount} matched` : "";
  const searchLabel = graphSearchQuery() ? ` · ${searchMatchesInGraph.length} search matches` : "";
  els.graphMeta.textContent = `${subgraph.nodes.length} nodes · ${subgraph.edges.length} edges${resultLabel}${filterLabel}${searchLabel}`;
  if (els.graphSearchMeta) {
    els.graphSearchMeta.textContent = graphSearchQuery()
      ? `Search: ${searchMatchesInGraph.length} match${searchMatchesInGraph.length === 1 ? "" : "es"}`
      : "Search: off";
  }
  renderGraphModeBanner(subgraph);

  state.graphCurrentSubgraph = subgraph;

  // Initialize positions and build SVG
  const width = 920;
  const height = 620;
  initGraphPositions(subgraph, width, height);
  createGraphSvgDom(subgraph);
  updateGraphLegend(subgraph);
  updateGraphDetailPanel();
  if (graphSearchQuery() && state.selectedGraphNodeId) {
    centerGraphOnNode(state.selectedGraphNodeId);
  }

  // Create and start simulation
  state.graphSim = createForceSimulation(subgraph.nodes, subgraph.edges, { width, height });
  startGraphAnimation();
}

function renderTimeline(timeline) {
  if (!timeline) {
    els.timelinePane.innerHTML = renderGuidedEmptyState(
      "Timeline not loaded yet",
      "Timeline records each query run. Open a project and execute a query to build session history.",
      { runStarter: state.workspace.projectOpen, openReference: true }
    );
    bindGuidedEmptyStateActions(els.timelinePane);
    return;
  }

  syncSelectedTimelineIndex();
  const visibleEntries = filteredTimelineEntries();
  els.timelineMeta.textContent = `${visibleEntries.length}/${timeline.entries.length} entries`;

  if (!timeline.entries.length) {
    els.timelinePane.innerHTML = renderGuidedEmptyState(
      "No runs yet",
      "Run a first query to create session history, compare results over time, and unlock run detail.",
      { loadStarter: true, runStarter: true, openReference: true }
    );
    bindGuidedEmptyStateActions(els.timelinePane);
    renderDetailPane();
    return;
  }

  if (!visibleEntries.length) {
    els.timelinePane.innerHTML = renderGuidedEmptyState(
      "No runs match these filters",
      "Clear or adjust the timeline filters to recover the session context.",
      { openReference: false }
    );
    renderDetailPane();
    return;
  }

  els.timelinePane.innerHTML = visibleEntries
    .map(({ entry, actualIndex }) => `
      <article class="timeline-entry ${state.selectedTimelineIndex === actualIndex ? "is-active" : ""}" data-timeline-index="${actualIndex}">
        <div>
          <strong>#${actualIndex + 1}</strong><br />
          <code>${escapeHtml(entry.run_mode)}</code>
        </div>
        <div>
          <div>${escapeHtml(entry.query)}</div>
          <p class="timeline-summary">${escapeHtml(entry.summary || entry.error || "No summary")} · deps ${entry.depends_on.length}</p>
          <div class="timeline-actions">
            <button type="button" class="ghost-button" data-pin-index="${actualIndex}">
              ${entry.pinned ? "Unpin" : "Pin"}
            </button>
            <button type="button" class="ghost-button" data-rerun-index="${actualIndex}">Rerun</button>
            <button type="button" class="ghost-button" data-load-index="${actualIndex}">Load</button>
          </div>
        </div>
        <div>
          <span class="badge">${escapeHtml(entry.status)}</span>
          ${entry.pinned ? '<div class="timeline-pin">Pinned</div>' : ""}
        </div>
      </article>
    `)
    .join("");

  els.timelinePane.querySelectorAll("[data-timeline-index]").forEach((node) => {
    node.addEventListener("click", async () => {
      const index = Number(node.dataset.timelineIndex);
      await selectTimelineEntry(index);
      setWorkspaceTab("detail");
    });
  });

  els.timelinePane.querySelectorAll("[data-pin-index]").forEach((button) => {
    button.addEventListener("click", async (event) => {
      event.stopPropagation();
      await toggleTimelinePin(Number(button.dataset.pinIndex));
    });
  });

  els.timelinePane.querySelectorAll("[data-rerun-index]").forEach((button) => {
    button.addEventListener("click", async (event) => {
      event.stopPropagation();
      await rerunTimelineIndex(Number(button.dataset.rerunIndex));
    });
  });

  els.timelinePane.querySelectorAll("[data-load-index]").forEach((button) => {
    button.addEventListener("click", async (event) => {
      event.stopPropagation();
      await loadTimelineEntryToEditor(Number(button.dataset.loadIndex));
    });
  });

  renderDetailPane();
}

function dagVisualModel(dag) {
  const width = 760;
  const height = 340;
  const targetId = dag?.target_run_id || null;
  const nodes = dag?.nodes || [];
  const edges = dag?.edges || [];

  if (!targetId || !nodes.length) {
    return { width, height, positions: new Map(), targetId: null };
  }

  const incoming = new Map();
  const outgoing = new Map();
  nodes.forEach((node) => {
    incoming.set(node.run_id, []);
    outgoing.set(node.run_id, []);
  });
  edges.forEach((edge) => {
    if (incoming.has(edge.to_run_id)) incoming.get(edge.to_run_id).push(edge.from_run_id);
    if (outgoing.has(edge.from_run_id)) outgoing.get(edge.from_run_id).push(edge.to_run_id);
  });

  const levels = new Map([[targetId, 0]]);
  const queue = [targetId];
  while (queue.length) {
    const current = queue.shift();
    const currentLevel = levels.get(current) ?? 0;

    for (const prev of incoming.get(current) || []) {
      if (!levels.has(prev) || (levels.get(prev) ?? 0) > currentLevel - 1) {
        levels.set(prev, currentLevel - 1);
        queue.push(prev);
      }
    }

    for (const next of outgoing.get(current) || []) {
      if (!levels.has(next) || (levels.get(next) ?? 0) < currentLevel + 1) {
        levels.set(next, currentLevel + 1);
        queue.push(next);
      }
    }
  }

  nodes.forEach((node) => {
    if (!levels.has(node.run_id)) levels.set(node.run_id, 0);
  });

  const columns = new Map();
  for (const node of nodes) {
    const level = levels.get(node.run_id) ?? 0;
    if (!columns.has(level)) columns.set(level, []);
    columns.get(level).push(node);
  }

  const orderedLevels = [...columns.keys()].sort((a, b) => a - b);
  const left = 80;
  const right = width - 80;
  const span = Math.max(orderedLevels.length - 1, 1);
  const positions = new Map();

  orderedLevels.forEach((level, columnIndex) => {
    const columnNodes = columns.get(level) || [];
    const x = orderedLevels.length === 1 ? width / 2 : left + ((right - left) * columnIndex) / span;
    const verticalGap = height / (columnNodes.length + 1);
    columnNodes.forEach((node, rowIndex) => {
      positions.set(node.run_id, { x, y: verticalGap * (rowIndex + 1) });
    });
  });

  return { width, height, positions, targetId };
}

function renderDagVisual(dag) {
  const { width, height, positions, targetId } = dagVisualModel(dag);
  const nodes = dag?.nodes || [];
  const edges = dag?.edges || [];

  if (!targetId || !nodes.length) {
    return `<p class="empty">No DAG visual available.</p>`;
  }

  const lines = edges
    .map((edge) => {
      const from = positions.get(edge.from_run_id);
      const to = positions.get(edge.to_run_id);
      if (!from || !to) return "";
      const mx = (from.x + to.x) / 2;
      const my = (from.y + to.y) / 2;
      return `
        <g>
          <line class="dag-edge-line" x1="${from.x}" y1="${from.y}" x2="${to.x}" y2="${to.y}" />
          <text class="dag-edge-label" x="${mx}" y="${my - 6}" text-anchor="middle">${escapeHtml(edge.reason)}</text>
        </g>
      `;
    })
    .join("");

  const nodeMarks = nodes
    .map((node) => {
      const pos = positions.get(node.run_id);
      if (!pos) return "";
      const short = node.run_id.slice(0, 8);
      return `
        <g class="dag-visual-node ${node.run_id === targetId ? "is-target" : ""}" data-run-id="${escapeHtml(node.run_id)}">
          <rect x="${pos.x - 52}" y="${pos.y - 24}" width="104" height="48" rx="16" />
          <text x="${pos.x}" y="${pos.y - 5}" text-anchor="middle">${escapeHtml(node.run_mode)}</text>
          <text x="${pos.x}" y="${pos.y + 12}" text-anchor="middle">${escapeHtml(short)}</text>
        </g>
      `;
    })
    .join("");

  return `
    <div class="dag-visual-shell">
      <div class="visual-toolbar">
        <div class="visual-toolbar-group">
          <button type="button" class="ghost-button" data-dag-zoom="out">-</button>
          <button type="button" class="ghost-button" data-dag-zoom="in">+</button>
          <button type="button" class="ghost-button" data-dag-zoom="reset">Reset</button>
        </div>
        <span class="visual-zoom-label">${Math.round(state.dagVisualScale * 100)}%</span>
      </div>
      <svg class="dag-svg" viewBox="0 0 ${width} ${height}" role="img" aria-label="Timeline DAG visual">
        <g transform="translate(${width / 2} ${height / 2}) scale(${state.dagVisualScale}) translate(${-width / 2} ${-height / 2})">
          ${lines}
          ${nodeMarks}
        </g>
      </svg>
      <p class="visual-hint">Click a DAG node to select that run in the timeline.</p>
    </div>
  `;
}

function renderImpactVisual(impacted) {
  if (!impacted.length) {
    return `<p class="empty">No impacted dependent runs above threshold.</p>`;
  }

  return `
    <div class="impact-bars">
      ${impacted
        .map(
          (run) => `
            <article class="impact-bar-card">
              <div class="impact-bar-header">
                <code>${escapeHtml(run.run_mode)}</code>
                <span class="impact-score">${run.impact_score}</span>
              </div>
              <div class="impact-bar-track">
                <div class="impact-bar-fill" style="width: ${Math.max(6, run.impact_score)}%"></div>
              </div>
              <p>${escapeHtml(run.query)}</p>
              <p class="timeline-summary">${escapeHtml(run.run_id)}</p>
            </article>
          `
        )
        .join("")}
    </div>
  `;
}

function attachDetailHandlers() {
  els.detailPane.querySelectorAll("[data-run-id]").forEach((node) => {
    node.addEventListener("click", async () => {
      const runId = node.dataset.runId;
      const index = state.workspace.timeline?.entries?.findIndex((entry) => entry.id === runId) ?? -1;
      if (index >= 0) {
        await selectTimelineEntry(index);
      }
    });
  });
  els.detailPane.querySelectorAll("[data-dag-zoom]").forEach((button) => {
    button.addEventListener("click", () => {
      const action = button.dataset.dagZoom;
      if (action === "in") {
        state.dagVisualScale = Math.min(2.4, state.dagVisualScale + 0.2);
      } else if (action === "out") {
        state.dagVisualScale = Math.max(0.6, state.dagVisualScale - 0.2);
      } else {
        state.dagVisualScale = 1;
      }
      renderDetailPane();
    });
  });
}

function renderDetailPane() {
  const entry = state.workspace.timeline?.entries?.[state.selectedTimelineIndex];
  els.detailDagButton.classList.toggle("is-active", state.detailMode === "dag");
  els.detailImpactButton.classList.toggle("is-active", state.detailMode === "impact");

  if (!entry) {
    els.detailMeta.textContent = "no selection";
    els.detailPane.innerHTML = renderGuidedEmptyState(
      "Run detail appears after a query",
      "Select a timeline run to inspect lineage and impact, or create a first run to unlock this view.",
      { runStarter: state.workspace.projectOpen, openReference: true }
    );
    bindGuidedEmptyStateActions(els.detailPane);
    return;
  }

  els.detailMeta.textContent = `run #${state.selectedTimelineIndex + 1}`;

  const header = `
    <article class="detail-card">
      <h3>${escapeHtml(entry.query)}</h3>
      <p><code>${escapeHtml(entry.run_mode)}</code> · ${escapeHtml(entry.status)} · ${entry.duration_ms ? `${entry.duration_ms.toFixed(1)} ms` : "n/a"}</p>
      <p class="timeline-summary">${escapeHtml(entry.summary || entry.error || "No summary")}</p>
    </article>
  `;

  if (state.detailMode === "impact") {
    const impacted = state.detailImpact?.impacted || [];
    const body = impacted.length
      ? `
        <article class="detail-card dag-card">
          <h3>Impact Visual</h3>
          ${renderImpactVisual(impacted)}
          <p class="visual-hint">Higher score means stronger downstream impact from the selected run.</p>
        </article>
        <div class="impact-list">${impacted
          .map(
            (run) => `
            <article class="impact-row">
              <div><code>${escapeHtml(run.run_mode)}</code> · <span class="impact-score">${run.impact_score}</span></div>
              <p>${escapeHtml(run.query)}</p>
              <p class="timeline-summary">${escapeHtml(run.run_id)}</p>
            </article>`
          )
          .join("")}</div>`
      : `<p class="empty">No impacted dependent runs above threshold.</p>`;

    els.detailPane.innerHTML = `${header}<div class="detail-grid">${body}</div>`;
    attachDetailHandlers();
    return;
  }

  const dag = state.detailDag;
  const nodes = dag?.nodes || [];
  const edges = dag?.edges || [];

  els.detailPane.innerHTML = `
    ${header}
    <div class="detail-grid">
      <article class="detail-card dag-card">
        <h3>DAG Summary</h3>
        <p>${escapeHtml(dag?.summary || "No DAG summary available.")}</p>
        <div class="legend-row">
          <span class="legend-chip legend-chip-target">Target run</span>
          <span class="legend-chip legend-chip-related">Related runs</span>
        </div>
      </article>
      <article class="detail-card dag-card">
        <h3>DAG Visual</h3>
        ${renderDagVisual(dag)}
      </article>
      <article class="detail-card dag-card">
        <h3>Nodes</h3>
        <div class="dag-nodes">
          ${nodes.length
            ? nodes
                .map(
                  (node) => `
                    <div class="dag-node">
                      <div><code>${escapeHtml(node.run_mode)}</code> · ${escapeHtml(node.status)}</div>
                      <p class="timeline-summary">${escapeHtml(node.run_id)}</p>
                    </div>
                  `
                )
                .join("")
            : `<p class="empty">No DAG nodes.</p>`}
        </div>
      </article>
      <article class="detail-card dag-card">
        <h3>Edges</h3>
        <div class="dag-edges">
          ${edges.length
            ? edges
                .map(
                  (edge) => `
                    <div class="dag-node">
                      <p><code>${escapeHtml(edge.reason)}</code></p>
                      <p class="timeline-summary">${escapeHtml(edge.from_run_id)} → ${escapeHtml(edge.to_run_id)}</p>
                    </div>
                  `
                )
                .join("")
            : `<p class="empty">No local DAG edges.</p>`}
        </div>
      </article>
    </div>
  `;
  attachDetailHandlers();
}

bootstrap();
