async function selectTimelineEntry(index) {
  const entry = state.workspace.timeline?.entries?.[index];
  if (!entry) return;

  state.selectedTimelineIndex = index;
  state.selectedTimelineRunId = entry.id;
  setStatus("loading detail", "accent");

  try {
    const recentIndex = index + 1;
    const [dag, impact] = await Promise.all([
      api(`/api/timeline/dag/${recentIndex}`),
      api(`/api/timeline/impact/${recentIndex}`),
    ]);
    state.detailDag = dag;
    state.detailImpact = impact;
    renderTimeline(state.workspace.timeline);
    setStatus("ready", "accent");
  } catch (error) {
    state.detailDag = null;
    state.detailImpact = null;
    renderDetailPane();
    setStatus("detail error", "plain");
    els.runSummary.textContent = `Failed to load timeline detail: ${error.message}`;
  }
}

async function persistActiveTabQuery(queryText) {
  const tabId = activeTabId();
  if (!tabId) return;
  const session = await api(`/api/tabs/${encodeURIComponent(tabId)}/query`, {
    method: "POST",
    body: JSON.stringify({ query_text: queryText }),
  });
  state.workspace.session = session;
  renderQueryTabs();
}

function schedulePersistActiveTabQuery(queryText) {
  if (state.tabQueryPersistTimer) {
    window.clearTimeout(state.tabQueryPersistTimer);
  }
  state.tabQueryPersistTimer = window.setTimeout(async () => {
    try {
      await persistActiveTabQuery(queryText);
    } catch (error) {
      els.runSummary.textContent = `Failed to persist query text: ${error.message}`;
    }
  }, 300);
}

async function activateQueryTab(tabId) {
  if (!tabId) return;
  const session = await api(`/api/tabs/${encodeURIComponent(tabId)}/activate`, { method: "POST" });
  state.workspace.session = session;
  state.selectedResultRowIndex = null;
  syncEditorFromActiveTab();
  renderQueryTabs();
  renderActiveTabResult();
  syncRunSummaryFromActiveTab();
  await refreshGraphSubgraph();
}

async function createQueryTab() {
  const session = await api("/api/tabs", { method: "POST", body: JSON.stringify({}) });
  state.workspace.session = session;
  state.selectedResultRowIndex = null;
  syncEditorFromActiveTab();
  renderQueryTabs();
  renderActiveTabResult();
  syncRunSummaryFromActiveTab();
}

async function duplicateActiveTabToNewTab() {
  const sourceTab = activeTab();
  if (!sourceTab) return;
  await createQueryTab();
  const targetTab = activeTab();
  if (!targetTab) return;
  const queryText = sourceTab.query_text || getEditorValue() || "";
  if (queryText) {
    const session = await api(`/api/tabs/${encodeURIComponent(targetTab.id)}/query`, {
      method: "POST",
      body: JSON.stringify({ query_text: queryText }),
    });
    state.workspace.session = session;
    setEditorValue(queryText);
  }
  if (sourceTab.title) {
    const session = await api(`/api/tabs/${encodeURIComponent(activeTabId())}/rename`, {
      method: "POST",
      body: JSON.stringify({ title: `${sourceTab.title} copy` }),
    });
    state.workspace.session = session;
  }
  renderQueryTabs();
  renderActiveTabResult();
  syncRunSummaryFromActiveTab();
}

async function renameActiveQueryTab() {
  const tab = activeTab();
  if (!tab) return;
  const nextTitle = window.prompt("Rename query tab", tab.title || "Query");
  if (nextTitle === null) return;
  const session = await api(`/api/tabs/${encodeURIComponent(tab.id)}/rename`, {
    method: "POST",
    body: JSON.stringify({ title: nextTitle }),
  });
  state.workspace.session = session;
  renderQueryTabs();
}

async function closeActiveQueryTab() {
  const tab = activeTab();
  if (!tab) return;
  const session = await api(`/api/tabs/${encodeURIComponent(tab.id)}`, { method: "DELETE" });
  state.workspace.session = session;
  state.selectedResultRowIndex = null;
  syncEditorFromActiveTab();
  renderQueryTabs();
  renderActiveTabResult();
  syncRunSummaryFromActiveTab();
  renderGraphPane();
}

async function loadTimelineEntryToEditor(index) {
  const entry = state.workspace.timeline?.entries?.[index];
  const tabId = activeTabId();
  if (!entry || !tabId) return;
  const session = await api(`/api/tabs/${encodeURIComponent(tabId)}/query`, {
    method: "POST",
    body: JSON.stringify({ query_text: entry.query }),
  });
  state.workspace.session = session;
  setEditorValue(entry.query);
  els.runMode.value = entry.run_mode;
  state.uiPrefs.runMode = entry.run_mode;
  saveUiPrefs();
  state.selectedTimelineIndex = index;
  state.selectedTimelineRunId = entry.id;
  state.selectedResultRowIndex = null;
  renderQueryTabs();
  renderTimeline(state.workspace.timeline);
  refreshGraphDirtyState();
  renderGraphPane();
  els.runSummary.textContent = `Loaded run #${index + 1} into the active tab.`;
}

async function toggleTimelinePin(index) {
  setStatus("pinning", "accent");
  try {
    await api(`/api/timeline/pin/${index + 1}`, { method: "POST" });
    await refreshWorkbench();
    setStatus("ready", "accent");
  } catch (error) {
    setStatus("pin error", "plain");
    els.runSummary.textContent = `Failed to toggle pin: ${error.message}`;
  }
}

async function rerunTimelineIndex(index) {
  const entry = state.workspace.timeline?.entries?.[index];
  if (!entry) return;

  const selectedMode = els.timelineRerunMode.value || entry.run_mode;
  setStatus("rerunning", "accent");
  try {
    const result = await api(`/api/timeline/rerun/${index + 1}`, {
      method: "POST",
      body: JSON.stringify({ run_mode: selectedMode }),
    });
    if (result.graph_hint) {
      state.selectedGraphNodeId = result.graph_hint.focus_node_id || result.graph_hint.node_ids?.[0] || null;
    }
    setWorkspaceTab("results");
    await refreshWorkbench();
    els.runSummary.textContent = `Reran #${index + 1} in ${selectedMode} · ${result.summary} · ${result.duration_ms.toFixed(1)} ms`;
    setStatus("ready", "accent");
  } catch (error) {
    setStatus("rerun error", "plain");
    els.runSummary.textContent = `Failed to rerun selected timeline entry: ${error.message}`;
  }
}

async function refreshGraphSubgraph() {
  if (!state.workspace.projectOpen) {
    els.graphMeta.textContent = "launcher";
    els.graphPane.innerHTML = `<p class="empty">Open or create a project to load graph data.</p>`;
    return;
  }
  const params = new URLSearchParams({
    depth: String(state.uiPrefs.graphDepth),
    limit: String(state.uiPrefs.graphLimit),
  });
  if (state.selectedGraphNodeId) {
    params.set("focus_node_id", state.selectedGraphNodeId);
  }
  state.workspace.graphSubgraph = await api(`/api/graph/subgraph?${params.toString()}`);
  if (state.workspace.graphSubgraph.focus_node_id) {
    state.selectedGraphNodeId = state.workspace.graphSubgraph.focus_node_id;
  }
  renderGraphPane();
}

async function refreshWorkbench(requestedDbPath = null) {
  const previousSelectedRowIndex = state.selectedResultRowIndex;
  const openOptions = requestedDbPath
    ? { method: "POST", body: JSON.stringify({ db_path: requestedDbPath }) }
    : { method: "POST" };

  const sessionOpen = await api("/api/session/open", openOptions);
  const [session, projects] = await Promise.all([
    api("/api/session/state"),
    loadProjectsSnapshot(),
  ]);
  const hasProjectOpen = Boolean(sessionOpen.project_open);
  const [schema, graph, timeline] = hasProjectOpen
    ? await Promise.all([
        api("/api/schema"),
        api("/api/graph/snapshot"),
        api("/api/timeline"),
      ])
    : [null, null, null];

  state.workspace.dbPath = sessionOpen.db_path;
  state.workspace.projectOpen = hasProjectOpen;
  state.workspace.launcherMode = Boolean(sessionOpen.launcher_mode);
  state.workspace.pendingDbPath = sessionOpen.pending_db_path || null;
  state.workspace.session = session;
  state.workspace.schema = schema;
  state.workspace.graph = graph;
  state.workspace.timeline = timeline;
  state.workspace.projects = projects.length ? projects : (sessionOpen.projects || []);
  state.workspace.savedQueries = sessionOpen.saved_queries || [];
  state.workspace.sessionRestored = Boolean(sessionOpen.session_restored);
  state.workspace.timelineCount = sessionOpen.timeline_count || 0;
  syncViewFromLocation();
  state.selectedResultRowIndex = previousSelectedRowIndex;
  if (timeline) {
    syncSelectedTimelineIndex();
  } else {
    state.selectedTimelineIndex = null;
    state.selectedTimelineRunId = null;
  }

  // Restore per-project UI prefs from server
  if (sessionOpen.ui_preferences) {
    applyServerUiPrefs(sessionOpen.ui_preferences);
  }

  const active = activeTab();
  if (active?.last_result?.graph_hint) {
    state.selectedGraphNodeId = active.last_result.graph_hint.focus_node_id || active.last_result.graph_hint.node_ids?.[0] || state.selectedGraphNodeId;
  } else {
    state.selectedGraphNodeId = null;
  }

  const currentDraft = state.workspace.pendingDbPath || state.workspace.dbPath || state.uiPrefs.dbDraftPath || "";
  els.dbPathInput.value = currentDraft;
  els.launcherPathInput.value = currentDraft;
  state.uiPrefs.dbDraftPath = currentDraft;

  // Apply restored prefs to UI controls
  els.runMode.value = state.uiPrefs.runMode;
  els.graphDepthSelect.value = String(state.uiPrefs.graphDepth);
  els.graphLimitSelect.value = String(state.uiPrefs.graphLimit);
  els.graphTypeFilter.value = state.uiPrefs.graphTypeFilter || "";
  els.graphSearchInput.value = state.graphSearch || "";

  renderSessionMeta();
  renderProjectList();
  renderProjectMenu();
  renderLauncher();
  renderSavedQueries();
  renderQueryTabs();
  syncEditorFromActiveTab();
  renderActiveTabResult();
  syncRunSummaryFromActiveTab();
  renderStats(schema);
  renderSchema(schema);
  renderSuggestedQueries(schema);
  renderFindingsPane();
  renderTimeline(timeline);
  renderSidebar();
  state.graphResultDirty = false;
  if (hasProjectOpen) {
    await refreshGraphSubgraph();
  } else {
    els.graphMeta.textContent = "launcher";
    els.graphPane.innerHTML = `<p class="empty">Open or create a project to load graph data.</p>`;
  }

  if (hasProjectOpen && state.selectedTimelineIndex !== null) {
    await selectTimelineEntry(state.selectedTimelineIndex);
  } else {
    renderDetailPane();
  }
}

async function openDatabase(dbPath) {
  const nextPath = (dbPath || els.launcherPathInput?.value || els.dbPathInput.value || "").trim();
  if (!nextPath) {
    setStatus("missing db path", "plain");
    els.runSummary.textContent = "Enter a database path to open.";
    els.launcherStatus.textContent = "Enter a database path to open or create a project.";
    return;
  }

  setStatus("opening db", "accent");
  els.openDbButton.disabled = true;
  if (els.launcherOpenPathButton) els.launcherOpenPathButton.disabled = true;
  els.launcherStatus.textContent = `Opening ${nextPath}...`;
  const previousPath = state.workspace.dbPath;
  try {
    await refreshWorkbench(nextPath);
    if (state.workspace.projectOpen) {
      setNavView("workbench");
      els.runSummary.textContent = previousPath && previousPath !== nextPath
        ? `Opened ${nextPath} and restored session state.`
        : `Opened ${nextPath}.`;
      els.launcherStatus.textContent = `Opened ${nextPath}.`;
      setStatus("ready", "accent");
    } else {
      els.runSummary.textContent = `Path not found: ${nextPath}. Choose whether to create the project there or open another one.`;
      els.launcherStatus.textContent = `Path not found: ${nextPath}. Create a project there or choose another path.`;
      setStatus("path pending", "plain");
    }
  } catch (error) {
    setStatus("open error", "plain");
    els.runSummary.textContent = `Failed to open DB: ${error.message}`;
    els.launcherStatus.textContent = `Failed to open DB: ${error.message}`;
    els.dbPathInput.value = previousPath || nextPath;
    if (els.launcherPathInput) {
      els.launcherPathInput.value = previousPath || nextPath;
    }
  } finally {
    els.openDbButton.disabled = false;
    if (els.launcherOpenPathButton) els.launcherOpenPathButton.disabled = false;
  }
}

async function closeProject() {
  if (!state.workspace.projectOpen) {
    setNavView("launcher");
    renderLauncher();
    return;
  }

  setStatus("closing project", "accent");
  try {
    await api("/api/projects/close", { method: "POST" });
    await refreshWorkbench();
    setNavView("launcher");
    renderLauncher();
    els.runSummary.textContent = "Project closed.";
    els.launcherStatus.textContent = "Project closed. Choose another project to continue.";
    setStatus("launcher", "plain");
  } catch (error) {
    setStatus("close error", "plain");
    els.runSummary.textContent = `Failed to close project: ${error.message}`;
  }
}

async function saveCurrentQuery() {
  const query = getEditorValue().trim();
  if (!query) {
    showToast("No query to save");
    return false;
  }
  const suggestedName = activeTab()?.title || "";
  const name = prompt("Save query as:", suggestedName);
  if (!name || !name.trim()) return false;
  await api("/api/queries/save", { method: "POST", body: JSON.stringify({ name: name.trim(), query }) });
  await refreshWorkbench();
  showToast(`Query "${name.trim()}" saved`);
  return true;
}

async function runQuery() {
  if (!state.workspace.projectOpen) {
    setStatus("no project", "plain");
    els.runSummary.textContent = "No project is open. Create or open a project first.";
    renderLauncher();
    return;
  }

  const query = getEditorValue().trim();
  if (!query) {
    setStatus("empty query", "plain");
    return;
  }

  setStatus("running", "accent");
  els.runButton.disabled = true;
  state.graphExpandedNodes = [];
  state.graphExpandedEdges = [];

  try {
    const result = await api("/api/query/run", {
      method: "POST",
      body: JSON.stringify({
        query,
        run_mode: els.runMode.value,
      }),
    });

    if (result.graph_hint) {
      state.selectedGraphNodeId = result.graph_hint.focus_node_id || result.graph_hint.node_ids?.[0] || null;
    }
    state.selectedResultRowIndex = result.row_graph_hints?.findIndex((hint) => Boolean(hint));
    if (state.selectedResultRowIndex === -1) {
      state.selectedResultRowIndex = result.rows?.length ? 0 : null;
    }

    setWorkspaceTab("results");
    await refreshWorkbench();
    if (!result.graph_hint) {
      await refreshGraphSubgraph();
    }
    els.runSummary.textContent = `${result.summary} · ${result.duration_ms.toFixed(1)} ms`;
    setStatus("ready", "accent");
  } catch (error) {
    els.runSummary.textContent = `Query failed: ${error.message}`;
    renderTable(["error"], [[error.message]]);
    els.resultMeta.textContent = "error";
    setWorkspaceTab("results");
    setStatus("error", "plain");
  } finally {
    els.runButton.disabled = false;
  }
}

async function bootstrap() {
  initTheme();
  state.graphVisualScale = state.uiPrefs.graphVisualScale || 1;

  // Mount CodeMirror 6 editor
  if (typeof initCodeMirrorEditor === "function") {
    await initCodeMirrorEditor();
  }

  setStatus("syncing", "accent");
  renderTable([], []);
  renderProjectMenu();
  renderLauncher();
  renderSidebar();
  renderWorkspaceTabs();
  renderDetailPane();
  renderProjectList();

  els.runMode.value = state.uiPrefs.runMode;
  els.timelineFilterInput.value = state.uiPrefs.timelineFilter;
  els.timelineModeFilter.value = state.uiPrefs.timelineModeFilter;
  els.timelinePinnedOnly.checked = state.uiPrefs.timelinePinnedOnly;
  els.graphDepthSelect.value = String(state.uiPrefs.graphDepth);
  els.graphLimitSelect.value = String(state.uiPrefs.graphLimit);
  els.graphTypeFilter.value = state.uiPrefs.graphTypeFilter || "";
  els.dbPathInput.value = state.uiPrefs.dbDraftPath || "";

  try {
    const health = await api("/api/health");
    state.workspace.dbPath = health.db_path;
    const initialPath = (health.db_path || "").trim();
    if (initialPath) {
      await openDatabase(initialPath);
    } else {
      await refreshWorkbench();
    }
    setStatus("ready", "accent");
  } catch (error) {
    setStatus("offline", "plain");
    els.runSummary.textContent = `Failed to contact backend: ${error.message}`;
    els.launcherStatus.textContent = `Failed to contact backend: ${error.message}`;
  }
}

els.runButton.addEventListener("click", runQuery);
els.projectMenuButton.addEventListener("click", () => {
  const willShow = els.projectMenuDropdown.hidden;
  els.projectMenuDropdown.hidden = !willShow;
  if (willShow) {
    renderProjectMenu();
  }
});
els.projectMenuLauncherButton.addEventListener("click", async () => {
  if (state.navView === "launcher" && state.workspace.projectOpen) {
    setNavView("workbench");
  } else {
    setNavView("launcher");
  }
  renderLauncher();
  els.projectMenuDropdown.hidden = true;
});
els.projectMenuCreateButton.addEventListener("click", () => {
  showCreateProjectDialog({
    dbPath: (els.dbPathInput.value || state.workspace.pendingDbPath || "").trim(),
    name: suggestedProjectNameFromPath(els.dbPathInput.value || state.workspace.pendingDbPath || ""),
  });
  els.projectMenuDropdown.hidden = true;
});
els.projectMenuCloseButton.addEventListener("click", async () => {
  await closeProject();
  els.projectMenuDropdown.hidden = true;
});
els.openDbButton.addEventListener("click", () => openDatabase());
els.dbPathInput.addEventListener("input", (event) => {
  state.uiPrefs.dbDraftPath = event.target.value || "";
  if (els.launcherPathInput) {
    els.launcherPathInput.value = event.target.value || "";
  }
  saveUiPrefs();
});
els.dbPathInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    openDatabase();
  }
});
els.launcherPathInput.addEventListener("input", (event) => {
  const value = event.target.value || "";
  state.uiPrefs.dbDraftPath = value;
  els.dbPathInput.value = value;
  saveUiPrefs();
});
els.launcherPathInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    openDatabase(event.target.value || "");
  }
});
els.launcherOpenPathButton.addEventListener("click", async () => {
  await openDatabase(els.launcherPathInput.value || "");
});
els.launcherCreatePathButton.addEventListener("click", () => {
  const dbPath = (els.launcherPathInput.value || "").trim();
  showCreateProjectDialog({
    dbPath,
    name: suggestedProjectNameFromPath(dbPath),
  });
});
els.launcherCreateButton.addEventListener("click", () => {
  showCreateProjectDialog({
    dbPath: state.workspace.pendingDbPath || (els.launcherPathInput.value || "").trim(),
    name: suggestedProjectNameFromPath(state.workspace.pendingDbPath || els.launcherPathInput.value || ""),
  });
});
els.projectSelect.addEventListener("change", async (event) => {
  const value = event.target.value || "";
  if (!value) return;
  els.dbPathInput.value = value;
  if (els.launcherPathInput) {
    els.launcherPathInput.value = value;
  }
  state.uiPrefs.dbDraftPath = value;
  await openDatabase(value);
});

els.newProjectButton.addEventListener("click", () => showCreateProjectDialog({
  dbPath: (els.dbPathInput.value || "").trim(),
  name: suggestedProjectNameFromPath(els.dbPathInput.value || ""),
}));
els.editProjectButton.addEventListener("click", () => showEditProjectDialog());
els.deleteProjectButton.addEventListener("click", () => showDeleteProjectDialog());

els.saveQueryButton.addEventListener("click", async () => {
  try {
    await saveCurrentQuery();
  } catch (err) {
    showToast(`Failed to save: ${err.message}`);
  }
});

els.savedQueriesButton.addEventListener("click", () => {
  const isVisible = els.savedQueriesDropdown.style.display !== "none";
  els.savedQueriesDropdown.style.display = isVisible ? "none" : "flex";
  if (!isVisible) renderSavedQueries();
});

els.savedQueriesList.addEventListener("click", async (event) => {
  const deleteBtn = event.target.closest(".saved-query-delete");
  if (deleteBtn) {
    const qid = deleteBtn.dataset.queryId;
    try {
      await api(`/api/queries/${encodeURIComponent(qid)}`, { method: "DELETE" });
      await refreshWorkbench();
      renderSavedQueries();
      showToast("Query deleted");
    } catch (err) {
      showToast(`Delete failed: ${err.message}`);
    }
    return;
  }
  const item = event.target.closest(".saved-query-item");
  if (item) {
    const qid = item.dataset.queryId;
    const query = (state.workspace.savedQueries || []).find((q) => q.id === qid);
    if (query) {
      setEditorValue(query.query);
      schedulePersistActiveTabQuery(query.query);
      els.savedQueriesDropdown.style.display = "none";
      showToast(`Loaded: ${query.name}`);
    }
  }
});

els.newTabButton.addEventListener("click", async () => {
  try {
    await createQueryTab();
  } catch (error) {
    els.runSummary.textContent = `Failed to create tab: ${error.message}`;
  }
});
els.renameTabButton.addEventListener("click", async () => {
  try {
    await renameActiveQueryTab();
  } catch (error) {
    els.runSummary.textContent = `Failed to rename tab: ${error.message}`;
  }
});
els.closeTabButton.addEventListener("click", async () => {
  try {
    await closeActiveQueryTab();
  } catch (error) {
    els.runSummary.textContent = `Failed to close tab: ${error.message}`;
  }
});

els.tabResults.addEventListener("click", () => setWorkspaceTab("results"));
els.tabRaw.addEventListener("click", () => setWorkspaceTab("raw"));
els.tabGraph.addEventListener("click", () => setWorkspaceTab("graph"));
els.tabTimeline.addEventListener("click", () => setWorkspaceTab("timeline"));
els.tabDetail.addEventListener("click", () => setWorkspaceTab("detail"));
els.tabFindings.addEventListener("click", () => setWorkspaceTab("findings"));
els.referenceButton.addEventListener("click", async () => {
  await openReferenceCenter();
});
els.onboardingLoadButton.addEventListener("click", () => {
  const query = els.onboardingLoadButton.dataset.starterQuery || starterQuery();
  setEditorValue(query);
  schedulePersistActiveTabQuery(query);
  showToast("Starter query loaded");
});
els.onboardingRunButton.addEventListener("click", async () => {
  const query = els.onboardingRunButton.dataset.starterQuery || starterQuery();
  setEditorValue(query);
  schedulePersistActiveTabQuery(query);
  await runQuery();
});
els.onboardingReferenceButton.addEventListener("click", async () => {
  await openReferenceCenter();
});
els.onboardingDismissButton.addEventListener("click", () => {
  state.uiPrefs.onboardingDismissed = true;
  saveUiPrefs();
  renderOnboardingCard();
});
els.resultsFocusGraphButton.addEventListener("click", async () => {
  if (!activeResult()?.graph_hint) return;
  setWorkspaceTab("graph");
  await refreshGraphSubgraph();
});
els.resultsRunDetailButton.addEventListener("click", async () => {
  if (state.selectedTimelineIndex === null) {
    showToast("Select a timeline run first");
    return;
  }
  await selectTimelineEntry(state.selectedTimelineIndex);
  setWorkspaceTab("detail");
});
els.resultsPinRunButton.addEventListener("click", async () => {
  if (state.selectedTimelineIndex === null) {
    showToast("Select a timeline run first");
    return;
  }
  await toggleTimelinePin(state.selectedTimelineIndex);
});
els.resultsSaveFindingButton.addEventListener("click", () => {
  openFindingDialog();
});
els.resultsSaveQueryButton.addEventListener("click", async () => {
  try {
    await saveCurrentQuery();
  } catch (err) {
    showToast(`Failed to save: ${err.message}`);
  }
});
els.resultsOpenTabButton.addEventListener("click", async () => {
  try {
    await duplicateActiveTabToNewTab();
    showToast("Opened query in a new tab");
  } catch (err) {
    showToast(`Failed to create a new tab: ${err.message}`);
  }
});
els.findingsCaptureButton.addEventListener("click", () => {
  openFindingDialog();
});
els.projectNotesSaveButton.addEventListener("click", async () => {
  const project = activeProject();
  if (!project) {
    showToast("Open a project first");
    return;
  }
  try {
    await api("/api/projects/update", {
      method: "PUT",
      body: JSON.stringify({
        db_path: project.db_path,
        notes: els.projectNotesInput.value || "",
      }),
    });
    await refreshWorkbench();
    showToast("Project notes saved");
  } catch (error) {
    showToast(`Project notes failed: ${error.message}`);
  }
});

els.sidebarToggle.addEventListener("click", () => {
  state.uiPrefs.sidebarCollapsed = !state.uiPrefs.sidebarCollapsed;
  saveUiPrefs();
  renderSidebar();
});

els.graphTypeFilter.addEventListener("change", (event) => {
  state.uiPrefs.graphTypeFilter = event.target.value || "";
  saveUiPrefs();
  renderGraphPane();
});
els.graphSearchInput.addEventListener("input", (event) => {
  state.graphSearch = event.target.value || "";
  renderGraphPane();
});
els.graphSearchInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    event.preventDefault();
    focusGraphSearchMatch(1);
    return;
  }
  if (event.key === "Escape") {
    event.preventDefault();
    state.graphSearch = "";
    els.graphSearchInput.value = "";
    renderGraphPane();
  }
});
els.graphSearchNextButton.addEventListener("click", () => {
  focusGraphSearchMatch(1);
});
els.graphReloadButton.addEventListener("click", async () => {
  state.graphNodePositions.clear();
  state.graphHiddenNodes.clear();
  state.graphExpandedNodes = [];
  state.graphExpandedEdges = [];
  setStatus("reloading", "accent");
  try {
    await refreshWorkbench();
    showToast("Database reloaded");
    setStatus("ready", "accent");
  } catch (err) {
    showToast(`Reload error: ${err.message}`);
    setStatus("error", "plain");
  }
});
els.graphDepthSelect.addEventListener("change", async (event) => {
  state.uiPrefs.graphDepth = Number(event.target.value || 1);
  saveUiPrefs();
  await refreshGraphSubgraph();
});
els.graphLimitSelect.addEventListener("change", async (event) => {
  state.uiPrefs.graphLimit = Number(event.target.value || 50);
  saveUiPrefs();
  await refreshGraphSubgraph();
});

els.timelineFilterInput.addEventListener("input", (event) => {
  state.uiPrefs.timelineFilter = event.target.value || "";
  saveUiPrefs();
  renderTimeline(state.workspace.timeline);
});
els.timelineModeFilter.addEventListener("change", (event) => {
  state.uiPrefs.timelineModeFilter = event.target.value || "";
  saveUiPrefs();
  renderTimeline(state.workspace.timeline);
});
els.timelinePinnedOnly.addEventListener("change", (event) => {
  state.uiPrefs.timelinePinnedOnly = Boolean(event.target.checked);
  saveUiPrefs();
  renderTimeline(state.workspace.timeline);
});
els.timelineRerunSelected.addEventListener("click", async () => {
  if (state.selectedTimelineIndex === null) return;
  await rerunTimelineIndex(state.selectedTimelineIndex);
});
els.timelineLoadSelected.addEventListener("click", async () => {
  if (state.selectedTimelineIndex === null) return;
  await loadTimelineEntryToEditor(state.selectedTimelineIndex);
});

els.detailDagButton.addEventListener("click", () => {
  state.detailMode = "dag";
  renderDetailPane();
});
els.detailImpactButton.addEventListener("click", () => {
  state.detailMode = "impact";
  renderDetailPane();
});
els.runMode.addEventListener("change", (event) => {
  state.uiPrefs.runMode = event.target.value;
  saveUiPrefs();
});

