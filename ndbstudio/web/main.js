window.__ndbStudioBooted = true;

const UI_PREFS_KEY = "ndbstudio-web-ui-prefs-v1";

const defaultUiPrefs = {
  workspaceTab: "results",
  runMode: "run",
  timelineFilter: "",
  timelineModeFilter: "",
  timelinePinnedOnly: false,
  graphMode: "dataset",
  graphTypeFilter: "",
  graphHopLimit: 1,
  graphLimit: 50,
  graphVisualScale: 1,
  graphLayout: "radial",
  graphMinimapCollapsed: false,
  sidebarCollapsed: false,
  dbDraftPath: "",
  theme: "light",
  onboardingDismissed: false,
  referenceLastSection: "nql_basics",
};

const state = {
  navView: "launcher",
  workspace: {
    dbPath: "",
    projectOpen: false,
    launcherMode: true,
    pendingDbPath: null,
    session: null,
    schema: null,
    graph: null,
    graphSubgraph: null,
    timeline: null,
    projects: [],
    savedQueries: [],
    sessionRestored: false,
    timelineCount: 0,
  },
  reference: null,
  uiPrefs: loadUiPrefs(),
  selectedTimelineIndex: null,
  selectedTimelineRunId: null,
  detailMode: "dag",
  detailDag: null,
  detailImpact: null,
  selectedGraphNodeId: null,
  selectedResultRowIndex: null,
  graphSearch: "",
  graphVisualScale: 1,
  graphPanOffset: { x: 0, y: 0 },
  dagVisualScale: 1,
  graphResultDirty: false,
  // interactive graph redesign state
  graphSim: null,
  graphNodePositions: new Map(),
  graphMoveNode: null,
  graphContextMenu: null,
  graphHiddenNodes: new Set(),
  graphExpandedNodes: [],  // nodes added via double-click expand
  graphExpandedEdges: [],  // edges added via double-click expand
  graphColorMap: new Map(),
  graphAnimFrame: null,
  graphSvgRefs: null,  // { svg, rootG, edgeG, nodeG, svgWrap, edgeEls: Map, nodeEls: Map }
  graphCurrentSubgraph: null,
  graphInspectorCollapsed: false,
  tabQueryPersistTimer: null,
  resultSortColumn: null,
  resultSortDirection: "asc",
  resultColumnFilters: {},
  autocomplete: { visible: false, items: [], activeIndex: 0, trigger: null },
};

const els = {
  dbPathBadge: document.getElementById("dbPathBadge"),
  appShell: document.getElementById("appShell"),
  projectLauncher: document.getElementById("projectLauncher"),
  launcherStatus: document.getElementById("launcherStatus"),
  launcherProjectList: document.getElementById("launcherProjectList"),
  launcherPathInput: document.getElementById("launcherPathInput"),
  launcherOpenPathButton: document.getElementById("launcherOpenPathButton"),
  launcherCreatePathButton: document.getElementById("launcherCreatePathButton"),
  launcherCreateButton: document.getElementById("launcherCreateButton"),
  projectMenuButton: document.getElementById("projectMenuButton"),
  projectMenuDropdown: document.getElementById("projectMenuDropdown"),
  projectMenuCurrent: document.getElementById("projectMenuCurrent"),
  projectMenuLauncherButton: document.getElementById("projectMenuLauncherButton"),
  projectMenuCreateButton: document.getElementById("projectMenuCreateButton"),
  projectMenuCloseButton: document.getElementById("projectMenuCloseButton"),
  projectHeadline: document.getElementById("projectHeadline"),
  projectContextBadge: document.getElementById("projectContextBadge"),
  // These badges were removed from the topbar to reduce cognitive load.
  // Kept as null-safe references so renderSessionMeta() can check them.
  nextStepBadge: document.getElementById("nextStepBadge"),
  dbPathBadge: document.getElementById("dbPathBadge"),
  sessionBadge: document.getElementById("sessionBadge"),
  statusBadge: document.getElementById("statusBadge"),
  dbPathInput: document.getElementById("dbPathInput"),
  openDbButton: document.getElementById("openDbButton"),
  projectSelect: document.getElementById("projectSelect"),
  newProjectButton: document.getElementById("newProjectButton"),
  editProjectButton: document.getElementById("editProjectButton"),
  deleteProjectButton: document.getElementById("deleteProjectButton"),
  saveQueryButton: document.getElementById("saveQueryButton"),
  savedQueriesButton: document.getElementById("savedQueriesButton"),
  savedQueriesDropdown: document.getElementById("savedQueriesDropdown"),
  savedQueriesList: document.getElementById("savedQueriesList"),
  referenceButton: document.getElementById("referenceButton"),
  runButton: document.getElementById("runButton"),
  runMode: document.getElementById("runMode"),
  cmEditor: document.getElementById("cmEditor"),
  queryTabs: document.getElementById("queryTabs"),
  newTabButton: document.getElementById("newTabButton"),
  renameTabButton: document.getElementById("renameTabButton"),
  closeTabButton: document.getElementById("closeTabButton"),
  runSummary: document.getElementById("runSummary"),
  resultMeta: document.getElementById("resultMeta"),
  resultsSummaryTitle: document.getElementById("resultsSummaryTitle"),
  resultsSummaryText: document.getElementById("resultsSummaryText"),
  resultsSummaryContext: document.getElementById("resultsSummaryContext"),
  resultsSummaryCapability: document.getElementById("resultsSummaryCapability"),
  resultsSelectionHint: document.getElementById("resultsSelectionHint"),
  onboardingCard: document.getElementById("onboardingCard"),
  onboardingText: document.getElementById("onboardingText"),
  onboardingLoadButton: document.getElementById("onboardingLoadButton"),
  onboardingRunButton: document.getElementById("onboardingRunButton"),
  onboardingReferenceButton: document.getElementById("onboardingReferenceButton"),
  onboardingDismissButton: document.getElementById("onboardingDismissButton"),
  resultsFocusGraphButton: document.getElementById("resultsFocusGraphButton"),
  resultsRunDetailButton: document.getElementById("resultsRunDetailButton"),
  resultsPinRunButton: document.getElementById("resultsPinRunButton"),
  resultsSaveFindingButton: document.getElementById("resultsSaveFindingButton"),
  resultsSaveQueryButton: document.getElementById("resultsSaveQueryButton"),
  resultsOpenTabButton: document.getElementById("resultsOpenTabButton"),
  resultsHead: document.querySelector("#resultsTable thead"),
  resultsBody: document.querySelector("#resultsTable tbody"),
  statsMeta: document.getElementById("statsMeta"),
  statsPane: document.getElementById("statsPane"),
  schemaMeta: document.getElementById("schemaMeta"),
  schemaPane: document.getElementById("schemaPane"),
  suggestedQueriesMeta: document.getElementById("suggestedQueriesMeta"),
  suggestedQueriesPane: document.getElementById("suggestedQueriesPane"),
  schemaSidebar: document.getElementById("schemaSidebar"),
  sidebarToggle: document.getElementById("sidebarToggle"),
  graphMeta: document.getElementById("graphMeta"),
  graphModeTitle: document.getElementById("graphModeTitle"),
  graphModeText: document.getElementById("graphModeText"),
  graphModeSource: document.getElementById("graphModeSource"),
  graphModeScope: document.getElementById("graphModeScope"),
  graphPane: document.getElementById("graphPane"),
  graphTypeFilter: document.getElementById("graphTypeFilter"),
  graphSearchInput: document.getElementById("graphSearchInput"),
  graphSearchNextButton: document.getElementById("graphSearchNextButton"),
  graphSearchMeta: document.getElementById("graphSearchMeta"),
  graphReloadButton: document.getElementById("graphReloadButton"),
  graphExportSvgButton: document.getElementById("graphExportSvgButton"),
  graphHopSelect: document.getElementById("graphHopSelect"),
  graphLimitSelect: document.getElementById("graphLimitSelect"),
  graphLegend: document.getElementById("graphLegend"),
  graphInspectorPanel: document.getElementById("graphInspectorPanel"),
  graphInspectorToggle: document.getElementById("graphInspectorToggle"),
  graphInspectorContent: document.getElementById("graphInspectorContent"),
  graphFitButton: document.getElementById("graphFitButton"),
  graphResetButton: document.getElementById("graphResetButton"),
  graphContextMenu: document.getElementById("graphContextMenu"),
  timelineMeta: document.getElementById("timelineMeta"),
  timelinePane: document.getElementById("timelinePane"),
  timelineFilterInput: document.getElementById("timelineFilterInput"),
  timelineModeFilter: document.getElementById("timelineModeFilter"),
  timelinePinnedOnly: document.getElementById("timelinePinnedOnly"),
  timelineRerunMode: document.getElementById("timelineRerunMode"),
  timelineRerunSelected: document.getElementById("timelineRerunSelected"),
  timelineLoadSelected: document.getElementById("timelineLoadSelected"),
  detailMeta: document.getElementById("detailMeta"),
  detailPane: document.getElementById("detailPane"),
  detailDagButton: document.getElementById("detailDagButton"),
  detailImpactButton: document.getElementById("detailImpactButton"),
  findingsCaptureButton: document.getElementById("findingsCaptureButton"),
  findingsMeta: document.getElementById("findingsMeta"),
  findingsPane: document.getElementById("findingsPane"),
  projectNotesInput: document.getElementById("projectNotesInput"),
  projectNotesSaveButton: document.getElementById("projectNotesSaveButton"),
  exportButton: document.getElementById("exportButton"),
  exportDropdown: document.getElementById("exportDropdown"),
  themeToggle: document.getElementById("themeToggle"),
  historyButton: document.getElementById("historyButton"),
  historyDropdown: document.getElementById("historyDropdown"),
  historySearch: document.getElementById("historySearch"),
  historyList: document.getElementById("historyList"),
  toastContainer: document.getElementById("toastContainer"),
  modalContainer: document.getElementById("modalContainer"),
  layoutRadialButton: document.getElementById("layoutRadialButton"),
  layoutForceButton: document.getElementById("layoutForceButton"),
  tabRaw: document.getElementById("tabRaw"),
  viewRaw: document.getElementById("viewRaw"),
  rawContent: document.getElementById("rawContent"),
  rawCopyButton: document.getElementById("rawCopyButton"),
  tabResults: document.getElementById("tabResults"),
  tabGraph: document.getElementById("tabGraph"),
  tabTimeline: document.getElementById("tabTimeline"),
  tabDetail: document.getElementById("tabDetail"),
  tabFindings: document.getElementById("tabFindings"),
  viewResults: document.getElementById("viewResults"),
  viewGraph: document.getElementById("viewGraph"),
  viewTimeline: document.getElementById("viewTimeline"),
  viewDetail: document.getElementById("viewDetail"),
  viewFindings: document.getElementById("viewFindings"),
};

function loadUiPrefs() {
  try {
    const raw = window.localStorage.getItem(UI_PREFS_KEY);
    if (!raw) return { ...defaultUiPrefs };
    return { ...defaultUiPrefs, ...JSON.parse(raw) };
  } catch {
    return { ...defaultUiPrefs };
  }
}

let _uiPrefsSaveTimer = null;
function saveUiPrefs() {
  window.localStorage.setItem(UI_PREFS_KEY, JSON.stringify(state.uiPrefs));
  // Debounced save to backend (per-project persistence)
  clearTimeout(_uiPrefsSaveTimer);
  _uiPrefsSaveTimer = setTimeout(() => {
    api("/api/session/ui-prefs", {
      method: "POST",
      body: JSON.stringify({
        theme: state.uiPrefs.theme || "light",
        workspace_tab: state.uiPrefs.workspaceTab || "results",
        run_mode: state.uiPrefs.runMode || "run",
        graph_layout: state.uiPrefs.graphLayout || "radial",
        graph_depth: state.uiPrefs.graphHopLimit || 1,
        graph_limit: state.uiPrefs.graphLimit || 50,
        graph_type_filter: state.uiPrefs.graphTypeFilter || "",
        sidebar_collapsed: Boolean(state.uiPrefs.sidebarCollapsed),
      }),
    }).catch(() => {});
  }, 500);
}

// T1-3: Toast notification
function showToast(message) {
  const toast = document.createElement("div");
  toast.className = "toast";
  toast.textContent = message;
  els.toastContainer.appendChild(toast);
  setTimeout(() => toast.remove(), 2000);
}

// W22: Modal dialog helpers
function showModal(html, onClose) {
  els.modalContainer.innerHTML = `<div class="modal-overlay"><div class="modal-dialog">${html}</div></div>`;
  els.modalContainer.querySelector(".modal-overlay").addEventListener("click", (e) => {
    if (e.target === e.currentTarget) closeModal();
  });
  const cancelBtn = els.modalContainer.querySelector("[data-action='cancel']");
  if (cancelBtn) cancelBtn.addEventListener("click", closeModal);
  if (onClose) els.modalContainer._onClose = onClose;
}

function closeModal() {
  els.modalContainer.innerHTML = "";
  if (els.modalContainer._onClose) {
    els.modalContainer._onClose();
    delete els.modalContainer._onClose;
  }
}

function showCreateProjectDialog(preset = {}) {
  const presetName = preset.name || "";
  const presetPath = preset.dbPath || "";
  const presetDescription = preset.description || "";
  showModal(`
    <h3>Create New Project</h3>
    <label class="field-label">Name <span style="color:var(--color-error)">*</span></label>
    <input id="newProjectName" class="path-input" type="text" placeholder="My Graph DB" value="${escapeHtml(presetName)}" autofocus />
    <label class="field-label">Path <small>(optional, defaults to ~/.ndstudio/databases/)</small></label>
    <input id="newProjectPath" class="path-input" type="text" placeholder="auto-generated" value="${escapeHtml(presetPath)}" />
    <label class="field-label">Description</label>
    <input id="newProjectDesc" class="path-input" type="text" placeholder="Optional description" value="${escapeHtml(presetDescription)}" />
    <div class="modal-actions">
      <button data-action="cancel" class="ghost-button" type="button">Cancel</button>
      <button id="confirmCreateProject" type="button">Create</button>
    </div>
  `);
  const confirmBtn = els.modalContainer.querySelector("#confirmCreateProject");
  confirmBtn.addEventListener("click", async () => {
    const name = els.modalContainer.querySelector("#newProjectName").value.trim();
    if (!name) { showToast("Name is required"); return; }
    const dbPath = els.modalContainer.querySelector("#newProjectPath").value.trim() || undefined;
    const description = els.modalContainer.querySelector("#newProjectDesc").value.trim() || undefined;
    confirmBtn.disabled = true;
    els.launcherStatus.textContent = dbPath
      ? `Creating project "${name}" at ${dbPath}...`
      : `Creating project "${name}"...`;
    try {
      const created = await api("/api/projects/create", {
        method: "POST",
        body: JSON.stringify({ name, db_path: dbPath, description }),
      });
      closeModal();
      const createdPath = created?.db_path || dbPath || "";
      setNavView("workbench");
      await refreshWorkbench(createdPath || null);
      els.launcherStatus.textContent = `Project "${name}" created${createdPath ? ` at ${createdPath}` : ""}.`;
      showToast(`Project "${name}" created`);
    } catch (err) {
      els.launcherStatus.textContent = `Create failed: ${err.message}`;
      showToast(`Create failed: ${err.message}`);
      confirmBtn.disabled = false;
    }
  });
}

function showEditProjectDialog() {
  const currentPath = state.workspace.dbPath;
  const project = (state.workspace.projects || []).find((p) => p.db_path === currentPath);
  if (!project) { showToast("No active project"); return; }
  showModal(`
    <h3>Edit Project</h3>
    <label class="field-label">Name</label>
    <input id="editProjectName" class="path-input" type="text" value="${escapeHtml(project.name)}" />
    <label class="field-label">Description</label>
    <input id="editProjectDesc" class="path-input" type="text" value="${escapeHtml(project.description || "")}" />
    <label class="field-label">Notes</label>
    <textarea id="editProjectNotes" class="path-input" rows="3">${escapeHtml(project.notes || "")}</textarea>
    <label class="field-label">Tags <small>(comma-separated)</small></label>
    <input id="editProjectTags" class="path-input" type="text" value="${escapeHtml((project.tags || []).join(", "))}" />
    <label class="field-label">Pin/Favorite</label>
    <label class="check-inline"><input id="editProjectPinned" type="checkbox" ${project.pinned ? "checked" : ""} /><span>Pinned</span></label>
    <div class="modal-actions">
      <button data-action="cancel" class="ghost-button" type="button">Cancel</button>
      <button id="confirmEditProject" type="button">Save</button>
    </div>
  `);
  const confirmBtn = els.modalContainer.querySelector("#confirmEditProject");
  confirmBtn.addEventListener("click", async () => {
    const name = els.modalContainer.querySelector("#editProjectName").value.trim() || undefined;
    const description = els.modalContainer.querySelector("#editProjectDesc").value;
    const notes = els.modalContainer.querySelector("#editProjectNotes").value;
    const tagsRaw = els.modalContainer.querySelector("#editProjectTags").value;
    const tags = tagsRaw.split(",").map((t) => t.trim()).filter(Boolean);
    const pinChanged = els.modalContainer.querySelector("#editProjectPinned").checked !== project.pinned;
    confirmBtn.disabled = true;
    try {
      await api("/api/projects/update", {
        method: "PUT",
        body: JSON.stringify({ db_path: currentPath, name, description, notes, tags }),
      });
      if (pinChanged) {
        await api("/api/projects/pin", { method: "POST", body: JSON.stringify({ db_path: currentPath }) });
      }
      closeModal();
      await refreshWorkbench();
      showToast("Project updated");
    } catch (err) {
      showToast(`Update failed: ${err.message}`);
      confirmBtn.disabled = false;
    }
  });
}

function showDeleteProjectDialog() {
  const currentPath = state.workspace.dbPath;
  const project = (state.workspace.projects || []).find((p) => p.db_path === currentPath);
  if (!project) { showToast("No active project"); return; }
  showModal(`
    <h3>Remove Project</h3>
    <p>Project: <strong>${escapeHtml(project.name)}</strong></p>
    <p>Path: <code>${escapeHtml(project.db_path)}</code></p>
    <div class="delete-options">
      <button id="deleteRegistryOnly" type="button">Remove from list only</button>
      <button id="deleteWithFiles" class="danger-button" type="button">Delete files from disk</button>
    </div>
    <div id="deleteConfirmWrap" style="display:none">
      <label class="field-label">Type project name to confirm deletion:</label>
      <input id="deleteConfirmInput" class="path-input" type="text" placeholder="${escapeHtml(project.name)}" />
      <button id="deleteConfirmButton" class="danger-button" type="button" disabled>Confirm Delete</button>
    </div>
    <div class="modal-actions">
      <button data-action="cancel" class="ghost-button" type="button">Cancel</button>
    </div>
  `);

  els.modalContainer.querySelector("#deleteRegistryOnly").addEventListener("click", async () => {
    try {
      await api("/api/projects/remove", {
        method: "DELETE",
        body: JSON.stringify({ db_path: currentPath, delete_files: false }),
      });
      closeModal();
      await refreshWorkbench();
      showToast("Project removed from list");
    } catch (err) {
      showToast(`Remove failed: ${err.message}`);
    }
  });

  els.modalContainer.querySelector("#deleteWithFiles").addEventListener("click", () => {
    const wrap = els.modalContainer.querySelector("#deleteConfirmWrap");
    wrap.style.display = "block";
    const input = els.modalContainer.querySelector("#deleteConfirmInput");
    const btn = els.modalContainer.querySelector("#deleteConfirmButton");
    input.addEventListener("input", () => {
      btn.disabled = input.value.trim() !== project.name;
    });
    btn.addEventListener("click", async () => {
      if (input.value.trim() !== project.name) return;
      btn.disabled = true;
      try {
        await api("/api/projects/remove", {
          method: "DELETE",
          body: JSON.stringify({ db_path: currentPath, delete_files: true }),
        });
        closeModal();
        await refreshWorkbench();
        showToast("Project and files deleted");
      } catch (err) {
        showToast(`Delete failed: ${err.message}`);
        btn.disabled = false;
      }
    });
  });
}

// T3-2: Theme management
function applyTheme(theme) {
  document.documentElement.dataset.theme = theme;
  state.uiPrefs.theme = theme;
  els.themeToggle.textContent = theme === "dark" ? "Light" : "Dark";
  saveUiPrefs();
  // Sync CodeMirror 6 editor theme
  if (typeof syncEditorTheme === "function") {
    syncEditorTheme(theme === "dark");
  }
}

function initTheme() {
  const saved = state.uiPrefs.theme;
  // Only use system preference if no explicit saved preference
  if (!saved) {
    const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    applyTheme(prefersDark ? "dark" : "light");
  } else {
    applyTheme(saved);
  }
}

// T3-2: Theme toggle
els.themeToggle.addEventListener("click", () => {
  applyTheme(state.uiPrefs.theme === "dark" ? "light" : "dark");
});

// T2-1: Export button
els.exportDropdown.querySelectorAll("[data-export]").forEach((btn) => {
  btn.addEventListener("click", () => {
    const result = activeResult();
    if (!result?.headers?.length) { showToast("No data to export"); return; }
    const format = btn.dataset.export;
    if (format === "csv") {
      downloadBlob(exportCsv(result.headers, result.rows || []), "results.csv", "text/csv");
    } else {
      downloadBlob(exportJson(result.headers, result.rows || []), "results.json", "application/json");
    }
    els.exportDropdown.style.display = "none";
    showToast(`Exported as ${format.toUpperCase()}`);
  });
});
// Close export dropdown when clicking outside
document.addEventListener("click", (e) => {
  if (!e.target.closest(".export-group")) {
    els.exportDropdown.style.display = "none";
  }
  if (!e.target.closest(".history-group")) {
    els.historyDropdown.style.display = "none";
  }
  if (!e.target.closest(".saved-queries-group")) {
    els.savedQueriesDropdown.style.display = "none";
  }
  if (!e.target.closest(".project-menu-group")) {
    els.projectMenuDropdown.hidden = true;
  }
});

// T3-1: History dropdown
els.historyButton.addEventListener("click", () => {
  const isVisible = els.historyDropdown.style.display !== "none";
  els.historyDropdown.style.display = isVisible ? "none" : "flex";
  if (!isVisible) {
    els.historySearch.value = "";
    renderHistoryDropdown();
    els.historySearch.focus();
  }
});
els.historySearch.addEventListener("input", (e) => {
  renderHistoryDropdown(e.target.value);
});

// T3-4: Layout selector
els.layoutRadialButton.addEventListener("click", () => {
  state.uiPrefs.graphLayout = "radial";
  saveUiPrefs();
  renderGraphPane();
});
els.layoutForceButton.addEventListener("click", () => {
  state.uiPrefs.graphLayout = "force";
  saveUiPrefs();
  renderGraphPane();
});

// Graph toolbar: Fit, Reset, Detail toggle, Context menu dismiss
els.graphFitButton.addEventListener("click", () => graphFitToView());
els.graphResetButton.addEventListener("click", () => {
  state.graphNodePositions.clear();
  state.graphHiddenNodes.clear();
  state.graphExpandedNodes = [];
  state.graphExpandedEdges = [];
  state.graphPanOffset = { x: 0, y: 0 };
  state.graphVisualScale = 1;
  state.uiPrefs.graphVisualScale = 1;
  saveUiPrefs();
  renderGraphPane();
});
els.graphInspectorToggle.addEventListener("click", () => {
  state.graphInspectorCollapsed = !state.graphInspectorCollapsed;
  els.graphInspectorPanel.classList.toggle("is-collapsed", state.graphInspectorCollapsed);
  els.graphInspectorToggle.textContent = state.graphInspectorCollapsed ? "Expand" : "Collapse";
});
document.addEventListener("click", (e) => {
  if (state.graphContextMenu && !e.target.closest(".graph-context-menu")) {
    hideGraphContextMenu();
  }
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && state.graphContextMenu) {
    hideGraphContextMenu();
  }
});

// T1-1: Global keyboard shortcuts
document.addEventListener("keydown", (event) => {
  // Don't fire shortcuts when typing in inputs (except Cmd combos)
  const tag = event.target.tagName;
  const isInput = tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";

  if ((event.metaKey || event.ctrlKey) && !event.shiftKey) {
    switch (event.key) {
      case "1":
        event.preventDefault();
        setWorkspaceTab("results");
        return;
      case "2":
        event.preventDefault();
        setWorkspaceTab("graph");
        return;
      case "3":
        event.preventDefault();
        setWorkspaceTab("timeline");
        return;
      case "4":
        event.preventDefault();
        setWorkspaceTab("detail");
        return;
      case "5":
        event.preventDefault();
        setWorkspaceTab("findings");
        return;
      case "6":
        event.preventDefault();
        setWorkspaceTab("raw");
        return;
      case "f":
        if (state.uiPrefs.workspaceTab === "graph") {
          event.preventDefault();
          els.graphSearchInput.focus();
          els.graphSearchInput.select();
        }
        return;
      case "n":
        if (!isInput) {
          event.preventDefault();
          createQueryTab().catch((err) => {
            els.runSummary.textContent = `Failed to create tab: ${err.message}`;
          });
        }
        return;
      case "w":
        if (!isInput) {
          event.preventDefault();
          closeActiveQueryTab().catch((err) => {
            els.runSummary.textContent = `Failed to close tab: ${err.message}`;
          });
        }
        return;
    }
  }
});

window.addEventListener("popstate", () => {
  syncViewFromLocation();
  renderProjectMenu();
  renderLauncher();
});

if (els.rawCopyButton) {
  els.rawCopyButton.addEventListener("click", () => {
    navigator.clipboard.writeText(els.rawContent.textContent).then(() => {
      showToast("Copied raw JSON to clipboard");
    }).catch(err => {
      showToast("Failed to copy: " + err.message);
    });
  });
}

if (els.graphExportSvgButton) {
  els.graphExportSvgButton.addEventListener("click", () => {
    if (!window.exportGraphSvg) return;
    const svgContent = window.exportGraphSvg();
    if (!svgContent) {
      showToast("No graph available to export");
      return;
    }
    downloadBlob(svgContent, "graph.svg", "image/svg+xml");
    showToast("Exported Graph SVG");
  });
}

// Robust Popover Toggle
window.togglePopover = function(event, buttonEl) {
  event.stopPropagation(); // Prevent document click from immediately closing it
  
  const toolbarItem = buttonEl.closest('.toolbar-item');
  if (!toolbarItem) return;
  
  const isAlreadyOpen = toolbarItem.classList.contains("is-popover-open");
  
  // Close all popovers first
  document.querySelectorAll(".toolbar-item.is-popover-open").forEach(item => {
    item.classList.remove("is-popover-open");
    const iconBtn = item.querySelector('.icon-button');
    if (iconBtn) iconBtn.classList.remove("is-active");
  });
  
  // Toggle the clicked one
  if (!isAlreadyOpen) {
    toolbarItem.classList.add("is-popover-open");
    buttonEl.classList.add("is-active");
  }
};

// Close popovers when clicking outside
document.addEventListener("click", (e) => {
  if (!e.target.closest(".toolbar-popover") && !e.target.closest(".icon-button")) {
    document.querySelectorAll(".toolbar-item.is-popover-open").forEach(item => {
      item.classList.remove("is-popover-open");
      const iconBtn = item.querySelector('.icon-button');
      if (iconBtn) iconBtn.classList.remove("is-active");
    });
  }
});
