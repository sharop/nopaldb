window.__ndbStudioBooted = true;

const UI_PREFS_KEY = "ndbstudio-web-ui-prefs-v1";

const defaultUiPrefs = {
  workspaceTab: "results",
  runMode: "run",
  timelineFilter: "",
  timelineModeFilter: "",
  timelinePinnedOnly: false,
  graphDataMode: "dataset",
  graphTypeFilter: "",
  graphDepth: 1,
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
  // Neo4j graph redesign state
  graphSim: null,
  graphNodePositions: new Map(),
  graphDragNode: null,
  graphContextMenu: null,
  graphHiddenNodes: new Set(),
  graphExpandedNodes: [],  // nodes added via double-click expand
  graphExpandedEdges: [],  // edges added via double-click expand
  graphColorMap: new Map(),
  graphAnimFrame: null,
  graphSvgRefs: null,  // { svg, rootG, edgeG, nodeG, svgWrap, edgeEls: Map, nodeEls: Map }
  graphCurrentSubgraph: null,
  graphDetailCollapsed: false,
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
  nextStepBadge: document.getElementById("nextStepBadge"),
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
  queryInput: document.getElementById("queryInput"),
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
  graphDepthSelect: document.getElementById("graphDepthSelect"),
  graphLimitSelect: document.getElementById("graphLimitSelect"),
  graphLegend: document.getElementById("graphLegend"),
  graphDetailPanel: document.getElementById("graphDetailPanel"),
  graphDetailToggle: document.getElementById("graphDetailToggle"),
  graphDetailContent: document.getElementById("graphDetailContent"),
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
  editorHighlight: document.getElementById("editorHighlight"),
  historyButton: document.getElementById("historyButton"),
  historyDropdown: document.getElementById("historyDropdown"),
  historySearch: document.getElementById("historySearch"),
  historyList: document.getElementById("historyList"),
  toastContainer: document.getElementById("toastContainer"),
  modalContainer: document.getElementById("modalContainer"),
  autocompleteWrap: document.getElementById("autocompleteWrap"),
  layoutRadialButton: document.getElementById("layoutRadialButton"),
  layoutForceButton: document.getElementById("layoutForceButton"),
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
        graph_depth: state.uiPrefs.graphDepth || 1,
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

// T2-2: NQL syntax highlighting — sequential tokenizer (no nesting)
const NQL_KEYWORD_SET = new Set([
  "find", "from", "where", "group", "by", "having", "order", "limit",
  "offset", "at", "timestamp", "export", "with", "add", "update", "set",
  "delete", "sketch", "commit", "create", "drop", "index", "explain",
  "asc", "desc", "and", "or", "not", "as", "on", "type", "in", "like",
  "hash", "btree", "fulltext", "csv", "json", "arrow", "parquet",
  "true", "false", "null",
]);
const NQL_FUNCTION_SET = new Set([
  // Aggregations
  "count", "sum", "avg", "min", "max",
  // Graph algorithms
  "pagerank", "betweenness", "clustering", "degree", "shortestpath",
  "community", "community_fast",
  // Node embeddings (E-0 to E-6)
  "has_embedding", "embedding_similarity", "knn_nodes", "similar_to",
  // Pattern embeddings (E-3)
  "pattern_has_embeddings", "pattern_embedding",
  // Path embeddings (E-7)
  "path_has_embeddings", "path_embedding",
  // Path similarity (E-8)
  "path_embedding_similarity",
  // Path kNN (E-9/E-10)
  "path_knn_references",
  // Path anomaly (E-10)
  "path_anomaly_score",
]);

function highlightNql(text) {
  // Tokenize then emit HTML — avoids nested <span> issues
  const tokens = [];
  let i = 0;
  while (i < text.length) {
    // Block comments
    if (text[i] === "/" && text[i + 1] === "*") {
      const end = text.indexOf("*/", i + 2);
      const slice = end >= 0 ? text.substring(i, end + 2) : text.substring(i);
      tokens.push({ type: "comment", text: slice });
      i += slice.length;
      continue;
    }
    // Line comments
    if (text[i] === "-" && text[i + 1] === "-") {
      const nl = text.indexOf("\n", i);
      const slice = nl >= 0 ? text.substring(i, nl) : text.substring(i);
      tokens.push({ type: "comment", text: slice });
      i += slice.length;
      continue;
    }
    // Strings (double or single quote)
    if (text[i] === '"' || text[i] === "'") {
      const quote = text[i];
      let j = i + 1;
      while (j < text.length && text[j] !== quote) {
        if (text[j] === "\\") j++; // skip escaped char
        j++;
      }
      const slice = text.substring(i, j + 1);
      tokens.push({ type: "string", text: slice });
      i = j + 1;
      continue;
    }
    // Numbers
    if (/\d/.test(text[i]) && (i === 0 || /[\s,()=<>!+\-*/]/.test(text[i - 1]))) {
      let j = i;
      while (j < text.length && /[\d.]/.test(text[j])) j++;
      tokens.push({ type: "number", text: text.substring(i, j) });
      i = j;
      continue;
    }
    // Words (identifiers, keywords, functions)
    if (/[A-Za-z_]/.test(text[i])) {
      let j = i;
      while (j < text.length && /[A-Za-z0-9_]/.test(text[j])) j++;
      const word = text.substring(i, j);
      const lower = word.toLowerCase();
      // Check if it's a function call (word followed by '(')
      let rest = text.substring(j);
      const isFnCall = NQL_FUNCTION_SET.has(lower) && /^\s*\(/.test(rest);
      if (isFnCall) {
        tokens.push({ type: "function", text: word });
      } else if (NQL_KEYWORD_SET.has(lower)) {
        tokens.push({ type: "keyword", text: word });
      } else {
        // Check if preceded by ':' — it's a label
        const prev = tokens.length ? tokens[tokens.length - 1] : null;
        if (prev && prev.type === "plain" && prev.text.endsWith(":")) {
          tokens.push({ type: "label", text: word });
        } else {
          tokens.push({ type: "plain", text: word });
        }
      }
      i = j;
      continue;
    }
    // Operators: ->, <-, -[, ]-, !=, <=, >=, <, >, =
    if ("-><=!".includes(text[i])) {
      let op = text[i];
      if (text[i] === "-" && text[i + 1] === ">") { op = "->"; }
      else if (text[i] === "<" && text[i + 1] === "-") { op = "<-"; }
      else if (text[i] === "-" && text[i + 1] === "[") { op = "-["; }
      else if (text[i] === "!" && text[i + 1] === "=") { op = "!="; }
      else if (text[i] === "<" && text[i + 1] === "=") { op = "<="; }
      else if (text[i] === ">" && text[i + 1] === "=") { op = ">="; }
      else if ("<>=".includes(text[i])) { op = text[i]; }
      else { tokens.push({ type: "plain", text: text[i] }); i++; continue; }
      tokens.push({ type: "operator", text: op });
      i += op.length;
      continue;
    }
    if (text[i] === "]" && text[i + 1] === "-") {
      tokens.push({ type: "operator", text: "]-" });
      i += 2;
      continue;
    }
    // Everything else (whitespace, parens, commas, colons, etc.)
    tokens.push({ type: "plain", text: text[i] });
    i++;
  }

  // Render tokens to HTML
  return tokens
    .map((tok) => {
      const safe = escapeHtml(tok.text);
      switch (tok.type) {
        case "keyword": return `<span class="hl-keyword">${safe}</span>`;
        case "function": return `<span class="hl-function">${safe}</span>`;
        case "string": return `<span class="hl-string">${safe}</span>`;
        case "number": return `<span class="hl-number">${safe}</span>`;
        case "comment": return `<span class="hl-comment">${safe}</span>`;
        case "operator": return `<span class="hl-operator">${safe}</span>`;
        case "label": return `<span class="hl-label">${safe}</span>`;
        default: return safe;
      }
    })
    .join("");
}

function syncEditorHighlight() {
  const text = els.queryInput.value;
  els.editorHighlight.innerHTML = highlightNql(text) + "\n";
  // Sync scroll position between textarea and overlay
  els.editorHighlight.scrollTop = els.queryInput.scrollTop;
  els.editorHighlight.scrollLeft = els.queryInput.scrollLeft;
  // Sync height — textarea may be resized by the user
  els.editorHighlight.style.height = els.queryInput.offsetHeight + "px";
}

const AUTOCOMPLETE_KEYWORDS = [
  "find", "from", "where", "group", "by", "having", "order", "limit",
  "offset", "as", "and", "or", "not", "in", "like", "at", "timestamp",
  "export", "json", "csv", "explain", "profile",
];

// T2-3 / W19: Autocomplete
function getAutocompleteContext() {
  const textarea = els.queryInput;
  const pos = textarea.selectionStart;
  const text = textarea.value.substring(0, pos);
  // Check for label trigger: after ":"
  const labelMatch = text.match(/:([A-Za-z0-9_]*)$/);
  if (labelMatch) {
    return { trigger: "label", prefix: labelMatch[1], start: pos - labelMatch[1].length };
  }
  // Check for property trigger: after "."
  const propMatch = text.match(/([A-Za-z_][A-Za-z0-9_]*)\.([A-Za-z0-9_]*)$/);
  if (propMatch) {
    return {
      trigger: "property",
      alias: propMatch[1],
      prefix: propMatch[2],
      start: pos - propMatch[2].length,
    };
  }
  // Check for general word trigger
  const keywordMatch = text.match(/(?:^|[\s,(])([A-Za-z_][A-Za-z0-9_]*)$/);
  if (keywordMatch) {
    return {
      trigger: "keyword",
      prefix: keywordMatch[1],
      start: pos - keywordMatch[1].length,
    };
  }
  return null;
}

function fuzzyScore(candidate, prefix) {
  const normalizedCandidate = String(candidate || "").toLowerCase();
  const normalizedPrefix = String(prefix || "").toLowerCase();
  if (!normalizedPrefix) return 1;
  if (normalizedCandidate.startsWith(normalizedPrefix)) return 100 - normalizedCandidate.length;
  const wordBoundaryIndex = normalizedCandidate.indexOf(normalizedPrefix);
  if (wordBoundaryIndex > 0 && /[_\-.]/.test(normalizedCandidate[wordBoundaryIndex - 1])) {
    return 70 - wordBoundaryIndex;
  }
  let cursor = 0;
  for (const char of normalizedPrefix) {
    cursor = normalizedCandidate.indexOf(char, cursor);
    if (cursor === -1) return -1;
    cursor += 1;
  }
  return 40 - normalizedCandidate.length;
}

function fuzzyMatchList(values, prefix) {
  return values
    .map((value) => ({ value, score: fuzzyScore(value, prefix) }))
    .filter((item) => item.score >= 0)
    .sort((a, b) => b.score - a.score || String(a.value).localeCompare(String(b.value)))
    .map((item) => item.value);
}

function inferAliasBindings(queryText) {
  const bindings = new Map();
  const text = String(queryText || "");
  const nodeRegex = /\(([A-Za-z_][A-Za-z0-9_]*)(?::([A-Za-z_][A-Za-z0-9_*]*))?/g;
  const edgeRegex = /\[([A-Za-z_][A-Za-z0-9_]*)(?::([A-Za-z_][A-Za-z0-9_*]*))?/g;
  for (const match of text.matchAll(nodeRegex)) {
    const [, alias, nodeType] = match;
    if (alias) bindings.set(alias, { kind: "node", type: nodeType || null });
  }
  for (const match of text.matchAll(edgeRegex)) {
    const [, alias, edgeType] = match;
    if (alias) bindings.set(alias, { kind: "edge", type: edgeType || null });
  }
  return bindings;
}

function getAutocompleteSuggestions(context) {
  const schema = state.workspace.schema;
  const prefix = (context.prefix || "").toLowerCase();

  if (context.trigger === "label") {
    if (!schema) return [];
    const labels = [
      ...(schema.node_types || []).map((t) => t.name),
      ...(schema.edge_types || []).map((t) => t.name),
      "*",
    ];
    return fuzzyMatchList(labels, prefix)
      .slice(0, 15)
      .map((label) => ({ label, insertText: label, meta: "Label" }));
  }

  if (context.trigger === "property") {
    if (!schema) return [];
    const aliasBindings = inferAliasBindings(els.queryInput.value.substring(0, els.queryInput.selectionStart));
    const allProps = new Set();
    const binding = context.alias ? aliasBindings.get(context.alias) : null;
    if (binding?.kind === "node" && binding.type) {
      const nodeType = (schema.node_types || []).find((item) => item.name === binding.type);
      for (const prop of nodeType?.properties || []) allProps.add(prop);
    } else if (binding?.kind === "edge" && binding.type) {
      const edgeType = (schema.edge_types || []).find((item) => item.name === binding.type);
      for (const prop of edgeType?.properties || []) allProps.add(prop);
    } else {
      for (const nt of schema.node_types || []) {
        for (const p of nt.properties || []) allProps.add(p);
      }
      for (const et of schema.edge_types || []) {
        for (const p of et.properties || []) allProps.add(p);
      }
    }
    const targetMeta = binding?.type
      ? `${binding.kind === "edge" ? "Edge" : "Node"} ${binding.type}${context.alias ? ` via ${context.alias}` : ""}`
      : context.alias
        ? `Property via ${context.alias}`
        : "Property";
    return fuzzyMatchList([...allProps], prefix)
      .slice(0, 15)
      .map((prop) => ({ label: prop, insertText: prop, meta: targetMeta }));
  }

  if (context.trigger === "keyword") {
    const values = [...new Set([...AUTOCOMPLETE_KEYWORDS, ...NQL_FUNCTION_SET])];
    return fuzzyMatchList(values, prefix)
      .slice(0, 12)
      .map((value) => ({
        label: value,
        insertText: value,
        meta: NQL_FUNCTION_SET.has(value) ? "Function" : "Keyword",
      }));
  }

  return [];
}

function positionAutocomplete() {
  const textarea = els.queryInput;
  const container = textarea.closest(".editor-container");
  if (!container) return;
  const mirror = document.createElement("div");
  const style = window.getComputedStyle(textarea);
  const propsToCopy = [
    "fontFamily", "fontSize", "fontWeight", "lineHeight", "letterSpacing",
    "paddingTop", "paddingRight", "paddingBottom", "paddingLeft",
    "borderTopWidth", "borderRightWidth", "borderBottomWidth", "borderLeftWidth",
    "whiteSpace", "wordWrap", "overflowWrap", "width",
  ];
  mirror.style.position = "absolute";
  mirror.style.visibility = "hidden";
  mirror.style.pointerEvents = "none";
  mirror.style.whiteSpace = "pre-wrap";
  mirror.style.wordBreak = "break-word";
  propsToCopy.forEach((prop) => {
    mirror.style[prop] = style[prop];
  });
  mirror.textContent = textarea.value.substring(0, textarea.selectionStart);
  const marker = document.createElement("span");
  marker.textContent = "\u200b";
  mirror.appendChild(marker);
  container.appendChild(mirror);
  const markerRect = marker.getBoundingClientRect();
  const containerRect = container.getBoundingClientRect();
  const top = markerRect.top - containerRect.top - textarea.scrollTop + Number.parseFloat(style.lineHeight || "22") + 8;
  const left = markerRect.left - containerRect.left - textarea.scrollLeft + 8;
  container.removeChild(mirror);
  els.autocompleteWrap.style.top = `${Math.min(Math.max(top, 8), textarea.offsetHeight - 8)}px`;
  els.autocompleteWrap.style.left = `${Math.max(8, Math.min(left, textarea.offsetWidth - 220))}px`;
}

function renderAutocomplete() {
  const ac = state.autocomplete;
  if (!ac.visible || !ac.items.length) {
    els.autocompleteWrap.style.display = "none";
    return;
  }
  positionAutocomplete();
  els.autocompleteWrap.style.display = "block";
  els.autocompleteWrap.innerHTML =
    `<div class="ac-hint">${
      ac.trigger === "label" ? "Labels"
        : ac.trigger === "property" ? "Properties"
          : "Keywords & functions"
    }</div>` +
    ac.items
      .map(
        (item, i) =>
          `<button type="button" class="ac-item ${i === ac.activeIndex ? "is-active" : ""}" data-ac-index="${i}">
            <span class="ac-item-label">${escapeHtml(item.label)}</span>
            ${item.meta ? `<span class="ac-item-meta">${escapeHtml(item.meta)}</span>` : ""}
          </button>`
      )
      .join("");

  els.autocompleteWrap.querySelectorAll("[data-ac-index]").forEach((btn) => {
    btn.addEventListener("mousedown", (e) => {
      e.preventDefault();
      insertAutocomplete(Number(btn.dataset.acIndex));
    });
  });
}

function insertAutocomplete(index) {
  const ac = state.autocomplete;
  const item = ac.items[index];
  if (!item) return;
  const context = getAutocompleteContext();
  if (!context) return;
  const textarea = els.queryInput;
  const before = textarea.value.substring(0, context.start);
  const after = textarea.value.substring(textarea.selectionStart);
  textarea.value = before + item.insertText + after;
  textarea.selectionStart = textarea.selectionEnd = context.start + item.insertText.length;
  state.autocomplete.visible = false;
  renderAutocomplete();
  syncEditorHighlight();
  schedulePersistActiveTabQuery(textarea.value);
}

function updateAutocomplete() {
  const context = getAutocompleteContext();
  if (!context) {
    state.autocomplete.visible = false;
    renderAutocomplete();
    return;
  }
  const items = getAutocompleteSuggestions(context);
  state.autocomplete = { visible: items.length > 0, items, activeIndex: 0, trigger: context.trigger };
  renderAutocomplete();
}

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
        els.queryInput.value = item.query;
        syncEditorHighlight();
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

// ── Neo4j-style Graph Visualization ──────────────────────────────────

const NODE_PALETTE = [
  "#4C8BF5", "#E94F3B", "#F5A623", "#7B61FF", "#1ABC9C",
  "#E91E9C", "#FF6B35", "#00BCD4", "#8BC34A", "#FF5252",
  "#536DFE", "#FFCA28", "#26A69A", "#AB47BC",
];

function getNodeColor(entityType) {
  if (!state.graphColorMap.has(entityType)) {
    const idx = state.graphColorMap.size % NODE_PALETTE.length;
    state.graphColorMap.set(entityType, NODE_PALETTE[idx]);
  }
  return state.graphColorMap.get(entityType);
}

function nodeRadius(node) {
  return Math.max(20, Math.min(32, 16 + (node.degree || 0) * 0.5));
}

// Force simulation: returns controller object
function createForceSimulation(nodes, edges, opts = {}) {
  const width = opts.width || 920;
  const height = opts.height || 620;
  const positions = state.graphNodePositions;
  let alpha = 1.0;
  const alphaDecay = 0.995;
  const alphaMin = 0.001;
  const velocityDecay = 0.6;
  const area = width * height;
  const k = Math.sqrt(area / Math.max(nodes.length, 1));
  const repulsion = k * k;
  const springLen = k * 0.8;
  const centerX = width / 2;
  const centerY = height / 2;
  const centerGravity = 0.01;
  const margin = 40;

  function tick() {
    if (alpha < alphaMin) return false;
    alpha *= alphaDecay;

    // Repulsive forces O(n²)
    for (let i = 0; i < nodes.length; i++) {
      const pi = positions.get(nodes[i].id);
      if (!pi) continue;
      pi.vx = (pi.vx || 0) * velocityDecay;
      pi.vy = (pi.vy || 0) * velocityDecay;
      for (let j = i + 1; j < nodes.length; j++) {
        const pj = positions.get(nodes[j].id);
        if (!pj) continue;
        let dx = pi.x - pj.x;
        let dy = pi.y - pj.y;
        const dist = Math.max(Math.sqrt(dx * dx + dy * dy), 1);
        const force = (repulsion / (dist * dist)) * alpha;
        const fx = (dx / dist) * force;
        const fy = (dy / dist) * force;
        pi.vx += fx;
        pi.vy += fy;
        pj.vx -= fx;
        pj.vy -= fy;
      }
    }

    // Spring forces along edges
    for (const edge of edges) {
      const ps = positions.get(edge.source);
      const pt = positions.get(edge.target);
      if (!ps || !pt) continue;
      const dx = ps.x - pt.x;
      const dy = ps.y - pt.y;
      const dist = Math.max(Math.sqrt(dx * dx + dy * dy), 1);
      const force = ((dist - springLen) / dist) * alpha * 0.3;
      const fx = dx * force;
      const fy = dy * force;
      ps.vx -= fx;
      ps.vy -= fy;
      pt.vx += fx;
      pt.vy += fy;
    }

    // Center gravity + apply velocities
    for (const node of nodes) {
      const p = positions.get(node.id);
      if (!p) continue;
      if (p.fx != null) { p.x = p.fx; p.y = p.fy; continue; }
      p.vx += (centerX - p.x) * centerGravity * alpha;
      p.vy += (centerY - p.y) * centerGravity * alpha;
      p.x += p.vx;
      p.y += p.vy;
      p.x = Math.max(margin, Math.min(width - margin, p.x));
      p.y = Math.max(margin, Math.min(height - margin, p.y));
    }
    return true;
  }

  return {
    tick,
    reheat(a = 0.3) { alpha = Math.max(alpha, a); },
    isRunning() { return alpha >= alphaMin; },
    pinNode(id) {
      const p = positions.get(id);
      if (p) { p.fx = p.x; p.fy = p.y; p.pinned = true; }
    },
    unpinNode(id) {
      const p = positions.get(id);
      if (p) { p.fx = null; p.fy = null; p.pinned = false; }
    },
    setNodePosition(id, x, y) {
      const p = positions.get(id);
      if (p) { p.x = x; p.y = y; p.fx = x; p.fy = y; }
    },
    alpha() { return alpha; },
  };
}

// Seed positions using radial layout (focus center, neighbors ring, rest outer)
function radialSeedPositions(subgraph, width, height) {
  const cx = width / 2;
  const cy = height / 2;
  const focusId = subgraph.focus_node_id || subgraph.nodes[0]?.id;
  const focus = subgraph.nodes.find((n) => n.id === focusId) || subgraph.nodes[0];
  if (!focus) return;

  state.graphNodePositions.set(focus.id, { x: cx, y: cy, vx: 0, vy: 0, fx: null, fy: null, pinned: false });

  const neighbors = subgraph.edges
    .filter((e) => e.source === focus.id || e.target === focus.id)
    .map((e) => (e.source === focus.id ? e.target : e.source));
  const uniqueNeighbors = [...new Set(neighbors)];
  const r1 = Math.min(220, Math.max(140, 34 * uniqueNeighbors.length));
  uniqueNeighbors.forEach((id, i) => {
    if (state.graphNodePositions.has(id)) return;
    const angle = (-Math.PI / 2) + (Math.PI * 2 * i) / Math.max(uniqueNeighbors.length, 1);
    state.graphNodePositions.set(id, { x: cx + Math.cos(angle) * r1, y: cy + Math.sin(angle) * r1, vx: 0, vy: 0, fx: null, fy: null, pinned: false });
  });

  const r2 = Math.min(300, r1 + 130);
  const rest = subgraph.nodes.filter((n) => !state.graphNodePositions.has(n.id));
  rest.forEach((node, i) => {
    const angle = (-Math.PI / 2) + (Math.PI * 2 * i) / Math.max(rest.length, 1);
    const orbit = r2 + ((i % 2) * 24);
    state.graphNodePositions.set(node.id, { x: cx + Math.cos(angle) * orbit, y: cy + Math.sin(angle) * orbit, vx: 0, vy: 0, fx: null, fy: null, pinned: false });
  });
}

// Seed positions using circular layout for force mode
function circularSeedPositions(subgraph, width, height) {
  const cx = width / 2;
  const cy = height / 2;
  subgraph.nodes.forEach((node, i) => {
    if (state.graphNodePositions.has(node.id)) return;
    const angle = (2 * Math.PI * i) / subgraph.nodes.length;
    const r = 100 + (i % 3) * 40;
    state.graphNodePositions.set(node.id, { x: cx + Math.cos(angle) * r, y: cy + Math.sin(angle) * r, vx: 0, vy: 0, fx: null, fy: null, pinned: false });
  });
}

function initGraphPositions(subgraph, width, height) {
  // Keep existing positions for nodes that are still present
  const oldPositions = new Map(state.graphNodePositions);
  state.graphNodePositions.clear();
  for (const node of subgraph.nodes) {
    if (oldPositions.has(node.id)) {
      state.graphNodePositions.set(node.id, oldPositions.get(node.id));
    }
  }
  // Seed new nodes
  const newNodes = subgraph.nodes.filter((n) => !state.graphNodePositions.has(n.id));
  if (newNodes.length === subgraph.nodes.length) {
    // All new — use layout seed
    if (state.uiPrefs.graphLayout === "radial") {
      radialSeedPositions(subgraph, width, height);
    } else {
      circularSeedPositions(subgraph, width, height);
    }
  } else if (newNodes.length > 0) {
    // Some new — place near existing center
    const cx = width / 2;
    const cy = height / 2;
    newNodes.forEach((node, i) => {
      const angle = (2 * Math.PI * i) / newNodes.length;
      state.graphNodePositions.set(node.id, { x: cx + Math.cos(angle) * 60, y: cy + Math.sin(angle) * 60, vx: 0, vy: 0, fx: null, fy: null, pinned: false });
    });
  }
}

// Detect parallel edges and compute edge paths
function computeEdgePaths(edges, positions) {
  const pairCount = new Map();
  const pairIndex = new Map();
  for (const edge of edges) {
    const key = [edge.source, edge.target].sort().join("|");
    pairCount.set(key, (pairCount.get(key) || 0) + 1);
  }
  const paths = [];
  for (const edge of edges) {
    const ps = positions.get(edge.source);
    const pt = positions.get(edge.target);
    if (!ps || !pt) continue;
    const key = [edge.source, edge.target].sort().join("|");
    const total = pairCount.get(key);
    const idx = pairIndex.get(key) || 0;
    pairIndex.set(key, idx + 1);

    const dx = pt.x - ps.x;
    const dy = pt.y - ps.y;
    const dist = Math.sqrt(dx * dx + dy * dy) || 1;
    // Shorten path by node radius to not overlap circles
    const rSrc = 22;
    const rTgt = 22;
    const x1 = ps.x + (dx / dist) * rSrc;
    const y1 = ps.y + (dy / dist) * rSrc;
    const x2 = pt.x - (dx / dist) * rTgt;
    const y2 = pt.y - (dy / dist) * rTgt;

    let d;
    if (total > 1) {
      const offset = ((idx - (total - 1) / 2) * 30);
      const nx = -dy / dist;
      const ny = dx / dist;
      const cx = (x1 + x2) / 2 + nx * offset;
      const cy = (y1 + y2) / 2 + ny * offset;
      d = `M ${x1} ${y1} Q ${cx} ${cy} ${x2} ${y2}`;
    } else {
      d = `M ${x1} ${y1} L ${x2} ${y2}`;
    }

    const mx = total > 1 ? (x1 + x2) / 2 + (-dy / dist) * ((idx - (total - 1) / 2) * 30) * 0.5 : (x1 + x2) / 2;
    const my = total > 1 ? (y1 + y2) / 2 + (dx / dist) * ((idx - (total - 1) / 2) * 30) * 0.5 : (y1 + y2) / 2;

    paths.push({ edge, d, mx, my, x1, y1, x2, y2 });
  }
  return paths;
}

function shouldShowGraphMinimap(subgraph) {
  return Boolean(subgraph?.nodes?.length > 12);
}

function buildGraphMinimap(refs, subgraph) {
  const minimap = document.createElement("div");
  minimap.className = `graph-minimap${state.uiPrefs.graphMinimapCollapsed ? " is-collapsed" : ""}`;
  minimap.innerHTML = `
    <div class="graph-minimap-header">
      <span class="panel-kicker">Minimap</span>
      <button type="button" class="ghost-button compact-button" data-minimap-toggle>
        ${state.uiPrefs.graphMinimapCollapsed ? "Expand" : "Collapse"}
      </button>
    </div>
    <div class="graph-minimap-body" ${state.uiPrefs.graphMinimapCollapsed ? "hidden" : ""}>
      <svg class="graph-minimap-svg" viewBox="0 0 180 120" role="img" aria-label="Graph minimap">
        <rect x="0" y="0" width="180" height="120" rx="12" class="graph-minimap-bg"></rect>
        <g class="graph-minimap-edge-layer"></g>
        <g class="graph-minimap-node-layer"></g>
        <rect class="graph-minimap-viewport" x="0" y="0" width="0" height="0" rx="6"></rect>
      </svg>
    </div>
  `;
  refs.svgWrap.parentElement.appendChild(minimap);
  const toggle = minimap.querySelector("[data-minimap-toggle]");
  toggle.addEventListener("click", () => {
    state.uiPrefs.graphMinimapCollapsed = !state.uiPrefs.graphMinimapCollapsed;
    saveUiPrefs();
    minimap.classList.toggle("is-collapsed", state.uiPrefs.graphMinimapCollapsed);
    const body = minimap.querySelector(".graph-minimap-body");
    body.hidden = state.uiPrefs.graphMinimapCollapsed;
    toggle.textContent = state.uiPrefs.graphMinimapCollapsed ? "Expand" : "Collapse";
  });
  const svg = minimap.querySelector(".graph-minimap-svg");
  svg.addEventListener("click", (event) => {
    const rect = svg.getBoundingClientRect();
    const x = ((event.clientX - rect.left) / rect.width) * 180;
    const y = ((event.clientY - rect.top) / rect.height) * 120;
    recenterFromMinimap(refs, subgraph, x, y);
  });
  refs.minimap = {
    shell: minimap,
    svg,
    edgeLayer: minimap.querySelector(".graph-minimap-edge-layer"),
    nodeLayer: minimap.querySelector(".graph-minimap-node-layer"),
    viewport: minimap.querySelector(".graph-minimap-viewport"),
  };
}

function graphBounds(positions, subgraph) {
  const xs = [];
  const ys = [];
  for (const node of subgraph.nodes || []) {
    const pos = positions.get(node.id);
    if (!pos) continue;
    xs.push(pos.x);
    ys.push(pos.y);
  }
  if (!xs.length || !ys.length) {
    return { minX: 0, minY: 0, maxX: 1, maxY: 1, width: 1, height: 1 };
  }
  const minX = Math.min(...xs);
  const minY = Math.min(...ys);
  const maxX = Math.max(...xs);
  const maxY = Math.max(...ys);
  return {
    minX,
    minY,
    maxX,
    maxY,
    width: Math.max(1, maxX - minX),
    height: Math.max(1, maxY - minY),
  };
}

function recenterFromMinimap(refs, subgraph, miniX, miniY) {
  const bounds = graphBounds(state.graphNodePositions, subgraph);
  const padding = 12;
  const scaleX = (180 - padding * 2) / bounds.width;
  const scaleY = (120 - padding * 2) / bounds.height;
  const scale = Math.min(scaleX, scaleY);
  const contentX = bounds.minX + ((miniX - padding) / Math.max(scale, 0.0001));
  const contentY = bounds.minY + ((miniY - padding) / Math.max(scale, 0.0001));
  state.graphPanOffset = {
    x: (refs.width / 2 - contentX) * state.graphVisualScale,
    y: (refs.height / 2 - contentY) * state.graphVisualScale,
  };
  updateGraphTransform();
  updateMinimap();
}

// Create the SVG DOM once (retained mode)
function createGraphSvgDom(subgraph) {
  const width = 920;
  const height = 620;
  els.graphPane.innerHTML = "";

  const shell = document.createElement("div");
  shell.className = "graph-visual-shell";

  // Zoom toolbar
  const toolbar = document.createElement("div");
  toolbar.className = "graph-zoom-toolbar";
  toolbar.innerHTML = `
    <button type="button" class="ghost-button" data-graph-zoom="out">-</button>
    <button type="button" class="ghost-button" data-graph-zoom="in">+</button>
    <button type="button" class="ghost-button" data-graph-zoom="reset">Reset</button>
    <span class="graph-zoom-label">${Math.round(state.graphVisualScale * 100)}%</span>
  `;
  shell.appendChild(toolbar);

  // SVG wrap
  const svgWrap = document.createElement("div");
  svgWrap.className = "graph-svg-wrap";
  svgWrap.id = "graphSvgWrap";

  const svgNS = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(svgNS, "svg");
  svg.setAttribute("class", "graph-svg");
  svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
  svg.setAttribute("role", "img");
  svg.setAttribute("aria-label", "Graph visual");

  // Defs: arrow marker + glow filter
  const defs = document.createElementNS(svgNS, "defs");
  defs.innerHTML = `
    <marker id="arrowhead" markerWidth="10" markerHeight="7" refX="9" refY="3.5" orient="auto" markerUnits="strokeWidth">
      <polygon points="0 0, 10 3.5, 0 7" class="graph-edge-arrow" />
    </marker>
    <marker id="arrowhead-result" markerWidth="10" markerHeight="7" refX="9" refY="3.5" orient="auto" markerUnits="strokeWidth">
      <polygon points="0 0, 10 3.5, 0 7" class="graph-edge-arrow is-result" />
    </marker>
    <filter id="glow" x="-30%" y="-30%" width="160%" height="160%">
      <feGaussianBlur stdDeviation="3" result="blur" />
      <feMerge><feMergeNode in="blur"/><feMergeNode in="SourceGraphic"/></feMerge>
    </filter>
  `;
  svg.appendChild(defs);

  // Root group for pan/zoom
  const rootG = document.createElementNS(svgNS, "g");
  rootG.setAttribute("transform", `translate(${width / 2 + state.graphPanOffset.x} ${height / 2 + state.graphPanOffset.y}) scale(${state.graphVisualScale}) translate(${-width / 2} ${-height / 2})`);

  // Edge group (below nodes in z-order)
  const edgeG = document.createElementNS(svgNS, "g");
  edgeG.setAttribute("class", "graph-edges-group");
  rootG.appendChild(edgeG);

  // Node group
  const nodeG = document.createElementNS(svgNS, "g");
  nodeG.setAttribute("class", "graph-nodes-group");
  rootG.appendChild(nodeG);

  svg.appendChild(rootG);
  svgWrap.appendChild(svg);
  shell.appendChild(svgWrap);
  els.graphPane.appendChild(shell);

  // Create node elements
  const nodeEls = new Map();
  for (const node of subgraph.nodes) {
    const g = document.createElementNS(svgNS, "g");
    g.setAttribute("class", "graph-node-group");
    g.setAttribute("data-node-id", node.id);

    const color = getNodeColor(node.entity_type || node.label || "Node");
    const r = nodeRadius(node);
    const circle = document.createElementNS(svgNS, "circle");
    circle.setAttribute("class", "graph-node-circle");
    circle.setAttribute("r", r);
    circle.setAttribute("fill", color + "22");
    circle.setAttribute("stroke", color);
    circle.setAttribute("stroke-width", "2.5");
    g.appendChild(circle);

    // Label text (below node)
    const label = (node.display_label || node.label || "Node").slice(0, 20);
    const textEl = document.createElementNS(svgNS, "text");
    textEl.setAttribute("class", "graph-node-label");
    textEl.setAttribute("text-anchor", "middle");
    textEl.setAttribute("dy", "0.35em");
    textEl.setAttribute("fill", color);
    textEl.setAttribute("font-size", r >= 26 ? "9" : "0");
    textEl.textContent = label.length > 8 ? label.slice(0, 7) + "…" : label;
    g.appendChild(textEl);

    // Subtitle below circle
    const subtitleEl = document.createElementNS(svgNS, "text");
    subtitleEl.setAttribute("class", "graph-node-subtitle");
    subtitleEl.setAttribute("text-anchor", "middle");
    subtitleEl.textContent = label;
    g.appendChild(subtitleEl);

    // Type label
    const typeEl = document.createElementNS(svgNS, "text");
    typeEl.setAttribute("class", "graph-node-type");
    typeEl.setAttribute("text-anchor", "middle");
    typeEl.setAttribute("fill", "var(--muted)");
    typeEl.setAttribute("font-size", "8");
    typeEl.textContent = (node.entity_type || node.label || "").slice(0, 16);
    g.appendChild(typeEl);

    nodeG.appendChild(g);
    nodeEls.set(node.id, { g, circle, textEl, subtitleEl, typeEl, node });
  }

  // Create edge elements
  const edgeEls = new Map();
  const edgePaths = computeEdgePaths(subgraph.edges, state.graphNodePositions);
  edgePaths.forEach((ep, idx) => {
    const g = document.createElementNS(svgNS, "g");
    const path = document.createElementNS(svgNS, "path");
    const isResult = ep.edge.edge_type === "result";
    path.setAttribute("class", `graph-edge-path${isResult ? " is-result" : ""}`);
    path.setAttribute("d", ep.d);
    path.setAttribute("marker-end", isResult ? "url(#arrowhead-result)" : "url(#arrowhead)");
    g.appendChild(path);

    const label = document.createElementNS(svgNS, "text");
    label.setAttribute("class", "graph-edge-label");
    label.setAttribute("text-anchor", "middle");
    label.setAttribute("x", ep.mx);
    label.setAttribute("y", ep.my - 5);
    label.textContent = (ep.edge.edge_type || "edge").slice(0, 16);
    g.appendChild(label);

    edgeG.appendChild(g);
    edgeEls.set(idx, { g, path, label, edge: ep.edge });
  });

  state.graphSvgRefs = { svg, rootG, edgeG, nodeG, svgWrap, edgeEls, nodeEls, width, height };
  if (shouldShowGraphMinimap(subgraph)) {
    buildGraphMinimap(state.graphSvgRefs, subgraph);
  }
  attachGraphInteraction();
}

// Update SVG element positions (called every animation frame)
function updateSvgPositions() {
  const refs = state.graphSvgRefs;
  if (!refs) return;
  const positions = state.graphNodePositions;
  const subgraph = state.graphCurrentSubgraph;
  if (!subgraph) return;

  // Determine result node highlighting
  const resultNodeIds = subgraph._resultNodeIds || null;
  const hasResultHighlight = resultNodeIds && resultNodeIds.size > 0;
  const searchMatches = new Set(graphSearchMatches(subgraph).map((node) => node.id));
  const hasSearchMatches = searchMatches.size > 0;

  // Update nodes
  for (const [id, nel] of refs.nodeEls) {
    const p = positions.get(id);
    if (!p) continue;
    const r = nodeRadius(nel.node);
    nel.circle.setAttribute("cx", p.x);
    nel.circle.setAttribute("cy", p.y);
    nel.textEl.setAttribute("x", p.x);
    nel.textEl.setAttribute("y", p.y);
    nel.subtitleEl.setAttribute("x", p.x);
    nel.subtitleEl.setAttribute("y", p.y + r + 13);
    nel.typeEl.setAttribute("x", p.x);
    nel.typeEl.setAttribute("y", p.y + r + 23);

    // Selection / focus / result styling
    const isSel = id === state.selectedGraphNodeId;
    const isFocus = id === subgraph.focus_node_id;
    const isResult = hasResultHighlight && resultNodeIds.has(id);
    const isDimmed = hasResultHighlight && !isResult && !isSel && !isFocus;
    const isSearchMatch = hasSearchMatches && searchMatches.has(id);
    nel.circle.classList.toggle("is-selected", isSel);
    nel.circle.classList.toggle("is-focus", isFocus);
    nel.circle.classList.toggle("is-pinned", !!p.pinned);
    nel.circle.classList.toggle("is-result", isResult);
    nel.circle.classList.toggle("is-search-match", isSearchMatch);
    nel.circle.classList.toggle("is-search-active", isSearchMatch && isSel);
    nel.circle.classList.toggle("is-dimmed", isDimmed);
    nel.g.classList.toggle("is-dimmed", isDimmed);
    if (isSel) {
      nel.circle.setAttribute("stroke-width", "4");
    } else if (isFocus) {
      nel.circle.setAttribute("stroke", "var(--accent-lime)");
      nel.circle.setAttribute("stroke-width", "3");
    } else if (isResult) {
      nel.circle.setAttribute("stroke", "var(--accent)");
      nel.circle.setAttribute("stroke-width", "3.5");
      nel.circle.setAttribute("filter", "url(#glow)");
    } else {
      const color = getNodeColor(nel.node.entity_type || nel.node.label || "Node");
      nel.circle.setAttribute("stroke", color);
      nel.circle.setAttribute("stroke-width", "2.5");
      nel.circle.setAttribute("filter", "none");
    }
    // Dim opacity for non-result nodes when result highlighting is active
    nel.g.setAttribute("opacity", isDimmed ? "0.35" : "1");
  }

  // Update edges
  const edgePaths = computeEdgePaths(subgraph.edges, positions);
  edgePaths.forEach((ep, idx) => {
    const eel = refs.edgeEls.get(idx);
    if (!eel) return;
    eel.path.setAttribute("d", ep.d);
    eel.label.setAttribute("x", ep.mx);
    eel.label.setAttribute("y", ep.my - 5);
    // Dim edges not connecting result nodes
    if (hasResultHighlight) {
      const edgeTouchesResult = resultNodeIds.has(ep.edge.source) || resultNodeIds.has(ep.edge.target);
      eel.g.setAttribute("opacity", edgeTouchesResult ? "1" : "0.2");
    } else {
      eel.g.setAttribute("opacity", "1");
    }
  });
  updateMinimap();
}

// Animation loop
function startGraphAnimation() {
  stopGraphAnimation();
  const loop = () => {
    if (!state.graphSim) return;
    const running = state.graphSim.tick();
    updateSvgPositions();
    if (running || state.graphDragNode) {
      state.graphAnimFrame = requestAnimationFrame(loop);
    } else {
      state.graphAnimFrame = null;
    }
  };
  state.graphAnimFrame = requestAnimationFrame(loop);
}

function stopGraphAnimation() {
  if (state.graphAnimFrame) {
    cancelAnimationFrame(state.graphAnimFrame);
    state.graphAnimFrame = null;
  }
}

function ensureAnimationRunning() {
  if (!state.graphAnimFrame && state.graphSim) {
    startGraphAnimation();
  }
}

// Convert screen coordinates to SVG space
function screenToSvg(clientX, clientY) {
  const refs = state.graphSvgRefs;
  if (!refs) return { x: clientX, y: clientY };
  const pt = refs.svg.createSVGPoint();
  pt.x = clientX;
  pt.y = clientY;
  const ctm = refs.rootG.getScreenCTM();
  if (!ctm) return { x: clientX, y: clientY };
  const svgPt = pt.matrixTransform(ctm.inverse());
  return { x: svgPt.x, y: svgPt.y };
}

// Attach pointer events for drag, click, pan/zoom, context menu
function attachGraphInteraction() {
  const refs = state.graphSvgRefs;
  if (!refs) return;
  const svgWrap = refs.svgWrap;
  let isPanning = false;
  let panStart = { x: 0, y: 0 };
  let dragOffset = { x: 0, y: 0 };
  let clickStart = null;
  let lastClickTime = 0;
  let lastClickNodeId = null;

  svgWrap.addEventListener("mousedown", (e) => {
    hideGraphContextMenu();
    const nodeGroup = e.target.closest("[data-node-id]");
    if (nodeGroup) {
      e.preventDefault();
      const nodeId = nodeGroup.dataset.nodeId;
      state.graphDragNode = nodeId;
      const svgPt = screenToSvg(e.clientX, e.clientY);
      const pos = state.graphNodePositions.get(nodeId);
      if (pos) {
        dragOffset = { x: svgPt.x - pos.x, y: svgPt.y - pos.y };
        state.graphSim?.pinNode(nodeId);
        state.graphSim?.reheat(0.3);
        ensureAnimationRunning();
      }
      clickStart = { x: e.clientX, y: e.clientY, time: Date.now() };
      return;
    }
    // Pan
    isPanning = true;
    panStart = { x: e.clientX - state.graphPanOffset.x, y: e.clientY - state.graphPanOffset.y };
    svgWrap.classList.add("is-panning");
  });

  svgWrap.addEventListener("mousemove", (e) => {
    if (state.graphDragNode) {
      const svgPt = screenToSvg(e.clientX, e.clientY);
      state.graphSim?.setNodePosition(state.graphDragNode, svgPt.x - dragOffset.x, svgPt.y - dragOffset.y);
      return;
    }
    if (isPanning) {
      state.graphPanOffset = { x: e.clientX - panStart.x, y: e.clientY - panStart.y };
      updateGraphTransform();
    }
  });

  const endInteraction = (e) => {
    if (state.graphDragNode) {
      const moved = clickStart ? Math.hypot(e.clientX - clickStart.x, e.clientY - clickStart.y) : 999;
      const elapsed = clickStart ? Date.now() - clickStart.time : 999;
      const nodeId = state.graphDragNode;

      if (moved < 5 && elapsed < 300) {
        // It was a click, not a drag — check for double-click
        const now = Date.now();
        if (lastClickNodeId === nodeId && (now - lastClickTime) < 400) {
          // Double-click detected — expand neighbors
          lastClickTime = 0;
          lastClickNodeId = null;
          expandNodeNeighbors(nodeId);
        } else {
          // Single click — select node
          lastClickTime = now;
          lastClickNodeId = nodeId;
          state.selectedGraphNodeId = nodeId;
          updateSvgPositions();
          updateGraphDetailPanel();
        }
      }
      const pos = state.graphNodePositions.get(nodeId);
      if (pos && !pos.pinned) {
        state.graphSim?.unpinNode(nodeId);
      }
      state.graphDragNode = null;
      clickStart = null;
      return;
    }
    if (isPanning) {
      isPanning = false;
      svgWrap.classList.remove("is-panning");
    }
  };

  svgWrap.addEventListener("mouseup", endInteraction);
  svgWrap.addEventListener("mouseleave", endInteraction);

  // Context menu on right-click
  svgWrap.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    const nodeGroup = e.target.closest("[data-node-id]");
    if (nodeGroup) {
      showGraphContextMenu(nodeGroup.dataset.nodeId, e.clientX, e.clientY);
    } else {
      showGraphCanvasContextMenu(e.clientX, e.clientY);
    }
  });

  // Scroll to zoom
  svgWrap.addEventListener("wheel", (e) => {
    e.preventDefault();
    const delta = e.deltaY > 0 ? -0.1 : 0.1;
    state.graphVisualScale = Math.max(0.3, Math.min(3, state.graphVisualScale + delta));
    state.uiPrefs.graphVisualScale = state.graphVisualScale;
    saveUiPrefs();
    updateGraphTransform();
    updateZoomLabel();
  }, { passive: false });

  // Zoom toolbar buttons
  refs.svgWrap.parentElement.querySelectorAll("[data-graph-zoom]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const action = btn.dataset.graphZoom;
      if (action === "in") state.graphVisualScale = Math.min(2.4, state.graphVisualScale + 0.2);
      else if (action === "out") state.graphVisualScale = Math.max(0.6, state.graphVisualScale - 0.2);
      else { state.graphVisualScale = 1; state.graphPanOffset = { x: 0, y: 0 }; }
      state.uiPrefs.graphVisualScale = state.graphVisualScale;
      saveUiPrefs();
      updateGraphTransform();
      updateZoomLabel();
    });
  });
}

function updateGraphTransform() {
  const refs = state.graphSvgRefs;
  if (!refs) return;
  const w = refs.width;
  const h = refs.height;
  refs.rootG.setAttribute("transform",
    `translate(${w / 2 + state.graphPanOffset.x} ${h / 2 + state.graphPanOffset.y}) scale(${state.graphVisualScale}) translate(${-w / 2} ${-h / 2})`
  );
  updateMinimap();
}

function updateZoomLabel() {
  const label = els.graphPane.querySelector(".graph-zoom-label");
  if (label) label.textContent = `${Math.round(state.graphVisualScale * 100)}%`;
}

function updateMinimap() {
  const refs = state.graphSvgRefs;
  const subgraph = state.graphCurrentSubgraph;
  const minimap = refs?.minimap;
  if (!refs || !subgraph || !minimap) return;
  const bounds = graphBounds(state.graphNodePositions, subgraph);
  const padding = 12;
  const scaleX = (180 - padding * 2) / bounds.width;
  const scaleY = (120 - padding * 2) / bounds.height;
  const scale = Math.min(scaleX, scaleY);
  const toMiniX = (x) => padding + (x - bounds.minX) * scale;
  const toMiniY = (y) => padding + (y - bounds.minY) * scale;

  minimap.nodeLayer.innerHTML = (subgraph.nodes || []).map((node) => {
    const pos = state.graphNodePositions.get(node.id);
    if (!pos) return "";
    const classes = [
      "graph-minimap-node",
      node.id === state.selectedGraphNodeId ? "is-selected" : "",
      node.id === subgraph.focus_node_id ? "is-focus" : "",
    ].filter(Boolean).join(" ");
    return `<circle class="${classes}" cx="${toMiniX(pos.x).toFixed(2)}" cy="${toMiniY(pos.y).toFixed(2)}" r="2.6"></circle>`;
  }).join("");

  minimap.edgeLayer.innerHTML = (subgraph.edges || []).slice(0, 120).map((edge) => {
    const src = state.graphNodePositions.get(edge.source);
    const tgt = state.graphNodePositions.get(edge.target);
    if (!src || !tgt) return "";
    return `<line class="graph-minimap-edge" x1="${toMiniX(src.x).toFixed(2)}" y1="${toMiniY(src.y).toFixed(2)}" x2="${toMiniX(tgt.x).toFixed(2)}" y2="${toMiniY(tgt.y).toFixed(2)}"></line>`;
  }).join("");

  const visibleWidth = refs.width / state.graphVisualScale;
  const visibleHeight = refs.height / state.graphVisualScale;
  const left = refs.width / 2 - state.graphPanOffset.x / state.graphVisualScale - visibleWidth / 2;
  const top = refs.height / 2 - state.graphPanOffset.y / state.graphVisualScale - visibleHeight / 2;
  minimap.viewport.setAttribute("x", String(toMiniX(left)));
  minimap.viewport.setAttribute("y", String(toMiniY(top)));
  minimap.viewport.setAttribute("width", String(Math.max(16, visibleWidth * scale)));
  minimap.viewport.setAttribute("height", String(Math.max(12, visibleHeight * scale)));
}

// Context menu: node
function showGraphContextMenu(nodeId, clientX, clientY) {
  const pos = state.graphNodePositions.get(nodeId);
  const isPinned = pos?.pinned || false;
  const menu = els.graphContextMenu;
  menu.innerHTML = `
    <button data-ctx="focus">Focus here</button>
    <button data-ctx="expand">Expand neighbors</button>
    <button data-ctx="edit">Edit properties</button>
    <button data-ctx="pin">${isPinned ? "Unpin node" : "Pin node"}</button>
    <button data-ctx="copy-id">Copy ID</button>
    <button data-ctx="hide">Hide node</button>
  `;
  positionContextMenu(menu, clientX, clientY);
  state.graphContextMenu = { nodeId };

  menu.querySelectorAll("button").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const action = btn.dataset.ctx;
      hideGraphContextMenu();
      if (action === "focus") {
        state.selectedGraphNodeId = nodeId;
        await refreshGraphSubgraph();
      } else if (action === "expand") {
        await expandNodeNeighbors(nodeId);
      } else if (action === "edit") {
        showEditNodeModal(nodeId);
      } else if (action === "pin") {
        if (isPinned) state.graphSim?.unpinNode(nodeId);
        else state.graphSim?.pinNode(nodeId);
        updateSvgPositions();
      } else if (action === "copy-id") {
        navigator.clipboard.writeText(nodeId).then(() => showToast("Node ID copied"));
      } else if (action === "hide") {
        state.graphHiddenNodes.add(nodeId);
        renderGraphPane();
      }
    });
  });
}

// Context menu: canvas (no node)
function showGraphCanvasContextMenu(clientX, clientY) {
  const menu = els.graphContextMenu;
  menu.innerHTML = `
    <button data-ctx="add-node">Add Node</button>
    <button data-ctx="add-edge">Add Edge</button>
  `;
  positionContextMenu(menu, clientX, clientY);
  state.graphContextMenu = {};

  menu.querySelectorAll("button").forEach((btn) => {
    btn.addEventListener("click", () => {
      hideGraphContextMenu();
      if (btn.dataset.ctx === "add-node") showAddNodeModal();
      else if (btn.dataset.ctx === "add-edge") showAddEdgeModal();
    });
  });
}

function positionContextMenu(menu, clientX, clientY) {
  menu.style.display = "block";
  menu.style.left = `${clientX}px`;
  menu.style.top = `${clientY}px`;
  requestAnimationFrame(() => {
    const rect = menu.getBoundingClientRect();
    if (rect.right > window.innerWidth) menu.style.left = `${clientX - rect.width}px`;
    if (rect.bottom > window.innerHeight) menu.style.top = `${clientY - rect.height}px`;
  });
}

function hideGraphContextMenu() {
  els.graphContextMenu.style.display = "none";
  state.graphContextMenu = null;
}

async function expandNodeNeighbors(nodeId) {
  try {
    const params = new URLSearchParams({ focus_node_id: nodeId, depth: "1", limit: String(state.uiPrefs.graphLimit) });
    const expansion = await api(`/api/graph/subgraph?${params.toString()}`);
    if (!expansion?.nodes?.length) return;

    // Check what's already visible (current subgraph + previously expanded)
    const sub = state.graphCurrentSubgraph;
    if (!sub) return;
    const existingIds = new Set(sub.nodes.map((n) => n.id));
    const newNodes = expansion.nodes.filter((n) => !existingIds.has(n.id));
    const existingEdgeKeys = new Set(sub.edges.map((e) => `${e.source}->${e.target}:${e.edge_type}`));
    const newEdges = expansion.edges.filter((e) => !existingEdgeKeys.has(`${e.source}->${e.target}:${e.edge_type}`));
    if (newNodes.length === 0 && newEdges.length === 0) {
      showToast("No new neighbors to expand");
      return;
    }

    // Store in persistent expanded state so they survive renderGraphPane rebuilds
    state.graphExpandedNodes.push(...newNodes);
    state.graphExpandedEdges.push(...newEdges);

    // Pre-seed positions for new nodes near the expanded node
    const pos = state.graphNodePositions.get(nodeId);
    const cx = pos?.x || 460;
    const cy = pos?.y || 310;
    newNodes.forEach((n, i) => {
      const angle = (2 * Math.PI * i) / Math.max(newNodes.length, 1);
      state.graphNodePositions.set(n.id, { x: cx + Math.cos(angle) * 80, y: cy + Math.sin(angle) * 80, vx: 0, vy: 0, fx: null, fy: null, pinned: false });
    });

    // Rebuild SVG and simulation
    renderGraphPane();
    showToast(`Expanded: +${newNodes.length} nodes, +${newEdges.length} edges`);
  } catch (err) {
    showToast(`Expand error: ${err.message}`);
  }
}

// Fit all nodes into view
function graphFitToView() {
  const refs = state.graphSvgRefs;
  if (!refs) return;
  const positions = state.graphNodePositions;
  if (!positions.size) return;
  let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
  for (const p of positions.values()) {
    minX = Math.min(minX, p.x);
    maxX = Math.max(maxX, p.x);
    minY = Math.min(minY, p.y);
    maxY = Math.max(maxY, p.y);
  }
  const padX = 80;
  const padY = 80;
  const contentW = (maxX - minX) + padX * 2;
  const contentH = (maxY - minY) + padY * 2;
  const scaleX = refs.width / contentW;
  const scaleY = refs.height / contentH;
  state.graphVisualScale = Math.max(0.3, Math.min(2, Math.min(scaleX, scaleY)));
  const centerContentX = (minX + maxX) / 2;
  const centerContentY = (minY + maxY) / 2;
  state.graphPanOffset = {
    x: (refs.width / 2 - centerContentX) * state.graphVisualScale,
    y: (refs.height / 2 - centerContentY) * state.graphVisualScale,
  };
  // Simpler approach: reset pan, adjust scale
  state.graphPanOffset = { x: 0, y: 0 };
  state.uiPrefs.graphVisualScale = state.graphVisualScale;
  saveUiPrefs();
  updateGraphTransform();
  updateZoomLabel();
}

// Update legend strip
function updateGraphLegend(subgraph) {
  const types = [...new Set(subgraph.nodes.map((n) => n.entity_type || n.label).filter(Boolean))].sort();
  els.graphLegend.innerHTML = types.map((type) => {
    const color = getNodeColor(type);
    return `<span class="graph-legend-chip"><span class="graph-legend-dot" style="background:${color}"></span>${escapeHtml(type)}</span>`;
  }).join("");
}

// Update detail panel
function updateGraphDetailPanel() {
  const sub = state.graphCurrentSubgraph;
  const panel = els.graphDetailPanel;
  const content = els.graphDetailContent;
  if (!sub || !state.selectedGraphNodeId) {
    panel.classList.add("is-empty");
    content.innerHTML = "";
    return;
  }
  const node = sub.nodes.find((n) => n.id === state.selectedGraphNodeId);
  if (!node) {
    panel.classList.add("is-empty");
    content.innerHTML = "";
    return;
  }
  panel.classList.remove("is-empty");
  panel.classList.toggle("is-collapsed", state.graphDetailCollapsed);
  const relations = graphRelations(sub, node).slice(0, 12);
  const matches = graphSearchMatches(sub);
  const searchSummary = graphSearchQuery()
    ? `<p class="timeline-summary">Search active: ${matches.length} match${matches.length === 1 ? "" : "es"} in the visible graph.</p>`
    : "";
  content.innerHTML = `${searchSummary}${renderNodeDetailCard(node, relations)}`;
  // Wire focus buttons in detail panel
  content.querySelectorAll("[data-focus-node]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      state.selectedGraphNodeId = btn.dataset.focusNode;
      await refreshGraphSubgraph();
    });
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
      els.queryInput.value = item.snippet;
      syncEditorHighlight();
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
      els.queryInput.value = item.snippet;
      syncEditorHighlight();
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
      els.queryInput.value = query;
      syncEditorHighlight();
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
    els.dbPathBadge.textContent = "db: none";
    els.projectContextBadge.textContent = "No project";
    els.projectHeadline.textContent = "Open a project to start exploring and build a chain of findings.";
    els.nextStepBadge.textContent = "Next: open project";
    els.sessionBadge.textContent = "session: launcher";
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
  els.nextStepBadge.textContent = !result
    ? "Next: run query"
    : findingsCount === 0
      ? "Next: capture finding"
      : result.graph_hint
        ? "Next: inspect graph"
        : "Next: review results";
  els.dbPathBadge.textContent = `db: ${displayName}`;
  els.sessionBadge.textContent = state.workspace.sessionRestored
    ? `session: restored · ${state.workspace.timelineCount} runs · ${findingsCount} findings`
    : `session: new · ${state.workspace.timelineCount} runs · ${findingsCount} findings`;
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
      els.queryInput.value = finding.query_text;
      syncEditorHighlight();
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
      els.queryInput.value = query;
      syncEditorHighlight();
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
      els.queryInput.value = item.query;
      syncEditorHighlight();
      schedulePersistActiveTabQuery(item.query);
      showToast(`Loaded: ${item.title}`);
    });
  });

  els.suggestedQueriesPane.querySelectorAll("[data-suggested-run]").forEach((button) => {
    button.addEventListener("click", async () => {
      const item = suggestions[Number(button.dataset.suggestedRun)];
      if (!item) return;
      els.queryInput.value = item.query;
      syncEditorHighlight();
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
    els.queryInput.value = "";
    syncEditorHighlight();
    return;
  }
  els.queryInput.value = tab.query_text || "";
  syncEditorHighlight();
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
  stopGraphAnimation();

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
  const queryText = sourceTab.query_text || els.queryInput.value || "";
  if (queryText) {
    const session = await api(`/api/tabs/${encodeURIComponent(targetTab.id)}/query`, {
      method: "POST",
      body: JSON.stringify({ query_text: queryText }),
    });
    state.workspace.session = session;
    els.queryInput.value = queryText;
    syncEditorHighlight();
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
  els.queryInput.value = entry.query;
  syncEditorHighlight();
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
  const query = els.queryInput.value.trim();
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

  const query = els.queryInput.value.trim();
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

  setStatus("syncing", "accent");
  renderTable([], []);
  renderProjectMenu();
  renderLauncher();
  renderSidebar();
  renderWorkspaceTabs();
  renderDetailPane();
  renderProjectList();
  syncEditorHighlight();

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
      els.queryInput.value = query.query;
      syncEditorHighlight();
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
els.tabGraph.addEventListener("click", () => setWorkspaceTab("graph"));
els.tabTimeline.addEventListener("click", () => setWorkspaceTab("timeline"));
els.tabDetail.addEventListener("click", () => setWorkspaceTab("detail"));
els.tabFindings.addEventListener("click", () => setWorkspaceTab("findings"));
els.referenceButton.addEventListener("click", async () => {
  await openReferenceCenter();
});
els.onboardingLoadButton.addEventListener("click", () => {
  const query = els.onboardingLoadButton.dataset.starterQuery || starterQuery();
  els.queryInput.value = query;
  syncEditorHighlight();
  schedulePersistActiveTabQuery(query);
  showToast("Starter query loaded");
});
els.onboardingRunButton.addEventListener("click", async () => {
  const query = els.onboardingRunButton.dataset.starterQuery || starterQuery();
  els.queryInput.value = query;
  syncEditorHighlight();
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
els.queryInput.addEventListener("keydown", (event) => {
  if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
    runQuery();
    return;
  }
  if (state.autocomplete.visible) {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      state.autocomplete.activeIndex = Math.min(state.autocomplete.activeIndex + 1, state.autocomplete.items.length - 1);
      renderAutocomplete();
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      state.autocomplete.activeIndex = Math.max(state.autocomplete.activeIndex - 1, 0);
      renderAutocomplete();
      return;
    }
    if (event.key === "Enter" || event.key === "Tab") {
      event.preventDefault();
      insertAutocomplete(state.autocomplete.activeIndex);
      return;
    }
    if (event.key === "Escape") {
      state.autocomplete.visible = false;
      renderAutocomplete();
      return;
    }
  }
  if ((event.metaKey || event.ctrlKey) && event.key === " ") {
    event.preventDefault();
    updateAutocomplete();
    return;
  }
  // T1-4: Tab key for indentation
  if (event.key === "Tab") {
    event.preventDefault();
    const textarea = event.target;
    const start = textarea.selectionStart;
    const end = textarea.selectionEnd;
    if (event.shiftKey) {
      // Unindent: remove up to 2 leading spaces on current line
      const before = textarea.value.substring(0, start);
      const lineStart = before.lastIndexOf("\n") + 1;
      const line = textarea.value.substring(lineStart);
      if (line.startsWith("  ")) {
        textarea.value = textarea.value.substring(0, lineStart) + textarea.value.substring(lineStart + 2);
        textarea.selectionStart = textarea.selectionEnd = Math.max(lineStart, start - 2);
      }
    } else {
      // Insert 2 spaces
      textarea.value = textarea.value.substring(0, start) + "  " + textarea.value.substring(end);
      textarea.selectionStart = textarea.selectionEnd = start + 2;
    }
    syncEditorHighlight();
    schedulePersistActiveTabQuery(textarea.value);
    return;
  }
});
els.queryInput.addEventListener("input", (event) => {
  schedulePersistActiveTabQuery(event.target.value || "");
  refreshGraphDirtyState();
  renderGraphPane();
  syncEditorHighlight();
  updateAutocomplete();
});
els.queryInput.addEventListener("scroll", () => {
  syncEditorHighlight();
});
els.queryInput.addEventListener("blur", () => {
  // Delay to allow click on autocomplete items
  setTimeout(() => {
    state.autocomplete.visible = false;
    renderAutocomplete();
  }, 200);
});
els.runMode.addEventListener("change", (event) => {
  state.uiPrefs.runMode = event.target.value;
  saveUiPrefs();
});

// T3-2: Theme toggle
els.themeToggle.addEventListener("click", () => {
  applyTheme(state.uiPrefs.theme === "dark" ? "light" : "dark");
});

// T2-1: Export button
els.exportButton.addEventListener("click", () => {
  const isVisible = els.exportDropdown.style.display !== "none";
  els.exportDropdown.style.display = isVisible ? "none" : "block";
});
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
els.graphFitButton.addEventListener("click", graphFitToView);
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
els.graphDetailToggle.addEventListener("click", () => {
  state.graphDetailCollapsed = !state.graphDetailCollapsed;
  els.graphDetailPanel.classList.toggle("is-collapsed", state.graphDetailCollapsed);
  els.graphDetailToggle.textContent = state.graphDetailCollapsed ? "Expand" : "Collapse";
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

bootstrap();
