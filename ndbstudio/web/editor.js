// ── NDBStudio NQL Editor — CodeMirror 6 via CDN ──────────────────────
// All CM6 modules are loaded from esm.sh as standard ES modules.
// This file is loaded as a deferred script and initialises the editor
// into the #cmEditor mount point defined in index.html.

const CM_CDN = "https://esm.sh";

// ── NQL keywords and functions (shared with autocomplete) ─────────────
const NQL_KEYWORDS = [
  "find", "from", "where", "group", "by", "having", "order", "limit",
  "offset", "at", "timestamp", "export", "with", "add", "update", "set",
  "delete", "sketch", "commit", "create", "drop", "index", "explain",
  "asc", "desc", "and", "or", "not", "as", "on", "type", "in", "like",
  "hash", "btree", "fulltext", "csv", "json", "arrow", "parquet",
  "true", "false", "null", "profile", "run",
];

const NQL_FUNCTIONS = [
  "count", "sum", "avg", "min", "max",
  "pagerank", "betweenness", "clustering", "degree", "shortestpath",
  "community", "community_fast", "leiden",
  "has_embedding", "embedding_similarity", "knn_nodes", "similar_to",
  "pattern_has_embeddings", "pattern_embedding",
  "path_has_embeddings", "path_embedding",
  "path_embedding_similarity", "path_knn_references", "path_anomaly_score",
];

const NQL_KEYWORD_SET = new Set(NQL_KEYWORDS);
const NQL_FUNCTION_SET = new Set(NQL_FUNCTIONS);

// ── CodeMirror editor instance (module-level singleton) ───────────────
let _cmView = null;

// Public getter used by the rest of the app to read the query text
function getEditorValue() {
  return _cmView ? _cmView.state.doc.toString() : "";
}

// Public setter used by tabs to restore query text
function setEditorValue(text) {
  if (!_cmView) return;
  const { EditorState } = _cmView.state.constructor;
  _cmView.dispatch({
    changes: { from: 0, to: _cmView.state.doc.length, insert: text ?? "" },
  });
}

// ── NQL StreamLanguage definition ─────────────────────────────────────
function buildNqlLanguage(StreamLanguage) {
  return StreamLanguage.define({
    name: "nql",
    token(stream) {
      // Block comment
      if (stream.match("/*")) {
        while (!stream.match("*/", true) && !stream.eol()) stream.next();
        return "comment";
      }
      // Line comment
      if (stream.match("--")) {
        stream.skipToEnd();
        return "comment";
      }
      // Strings
      if (stream.match('"')) {
        while (!stream.match('"', true) && !stream.eol()) {
          if (stream.peek() === "\\") stream.next();
          stream.next();
        }
        return "string";
      }
      if (stream.match("'")) {
        while (!stream.match("'", true) && !stream.eol()) {
          if (stream.peek() === "\\") stream.next();
          stream.next();
        }
        return "string";
      }
      // Numbers
      if (stream.match(/^[0-9]+\.?[0-9]*/)) return "number";
      // Identifiers / keywords / functions
      if (stream.match(/^[A-Za-z_][A-Za-z0-9_]*/)) {
        const word = stream.current().toLowerCase();
        if (NQL_FUNCTION_SET.has(word)) return "name.function";
        if (NQL_KEYWORD_SET.has(word)) return "keyword";
        return "variableName";
      }
      // Graph edge operators: ->, <-, -[, ]-
      if (stream.match(/^->|^<-|^-\[|^]-/)) return "operator";
      // Comparison operators
      if (stream.match(/^!=|^<=|^>=/)) return "operator";
      if (stream.match(/^[<>=!]/)) return "operator";
      // Punctuation
      stream.next();
      return null;
    },
    languageData: {
      commentTokens: { line: "--", block: { open: "/*", close: "*/" } },
    },
  });
}

// ── Autocomplete source using CM6 API ────────────────────────────────
function buildNqlAutocomplete(context, CompletionContext) {
  const schema = state?.workspace?.schema;

  // After ":" → label completions
  const labelCtx = context.matchBefore(/:([A-Za-z0-9_]*)$/);
  if (labelCtx) {
    const prefix = labelCtx.text.slice(1).toLowerCase();
    const labels = schema
      ? [
          ...(schema.node_types || []).map((t) => t.name),
          ...(schema.edge_types || []).map((t) => t.name),
          "*",
        ]
      : [];
    return {
      from: labelCtx.from + 1,
      options: fuzzyMatchList(labels, prefix).slice(0, 15).map((label) => ({
        label,
        type: "type",
        detail: "Label",
      })),
    };
  }

  // After "alias." → property completions
  const propCtx = context.matchBefore(/([A-Za-z_][A-Za-z0-9_]*)\.([A-Za-z0-9_]*)$/);
  if (propCtx) {
    const parts = propCtx.text.split(".");
    const alias = parts[0];
    const prefix = (parts[1] || "").toLowerCase();
    const allProps = new Set();
    if (schema) {
      // Try to infer alias binding from full document
      const fullText = _cmView ? _cmView.state.doc.toString() : "";
      const bindings = inferAliasBindings(fullText);
      const binding = bindings.get(alias);
      if (binding?.kind === "node" && binding.type) {
        const nodeType = (schema.node_types || []).find((t) => t.name === binding.type);
        for (const p of nodeType?.properties || []) allProps.add(p);
      } else if (binding?.kind === "edge" && binding.type) {
        const edgeType = (schema.edge_types || []).find((t) => t.name === binding.type);
        for (const p of edgeType?.properties || []) allProps.add(p);
      } else {
        for (const nt of schema.node_types || []) for (const p of nt.properties || []) allProps.add(p);
        for (const et of schema.edge_types || []) for (const p of et.properties || []) allProps.add(p);
      }
    }
    return {
      from: propCtx.from + alias.length + 1,
      options: fuzzyMatchList([...allProps], prefix).slice(0, 15).map((prop) => ({
        label: prop,
        type: "property",
        detail: "Property",
      })),
    };
  }

  // General keyword/function completion
  const wordCtx = context.matchBefore(/[A-Za-z_][A-Za-z0-9_]*$/);
  if (!wordCtx) return null;
  if (wordCtx.from === wordCtx.to && !context.explicit) return null;
  const prefix = wordCtx.text.toLowerCase();
  const allWords = [...new Set([...NQL_KEYWORDS, ...NQL_FUNCTIONS])];
  return {
    from: wordCtx.from,
    options: fuzzyMatchList(allWords, prefix).slice(0, 14).map((word) => ({
      label: word,
      type: NQL_FUNCTION_SET.has(word) ? "function" : "keyword",
      detail: NQL_FUNCTION_SET.has(word) ? "Function" : "Keyword",
    })),
  };
}

// ── Fuzzy helpers (re-used from original editor.js logic) ─────────────
function fuzzyScore(candidate, prefix) {
  const c = String(candidate || "").toLowerCase();
  const p = String(prefix || "").toLowerCase();
  if (!p) return 1;
  if (c.startsWith(p)) return 100 - c.length;
  let cursor = 0;
  for (const char of p) {
    cursor = c.indexOf(char, cursor);
    if (cursor === -1) return -1;
    cursor += 1;
  }
  return 40 - c.length;
}

function fuzzyMatchList(values, prefix) {
  return values
    .map((v) => ({ v, score: fuzzyScore(v, prefix) }))
    .filter((item) => item.score >= 0)
    .sort((a, b) => b.score - a.score || String(a.v).localeCompare(String(b.v)))
    .map((item) => item.v);
}

function inferAliasBindings(queryText) {
  const bindings = new Map();
  const text = String(queryText || "");
  const nodeRegex = /\(([A-Za-z_][A-Za-z0-9_]*)(?::([A-Za-z_][A-Za-z0-9_*]*))?/g;
  const edgeRegex = /\[([A-Za-z_][A-Za-z0-9_]*)(?::([A-Za-z_][A-Za-z0-9_*]*))?/g;
  for (const m of text.matchAll(nodeRegex)) {
    if (m[1]) bindings.set(m[1], { kind: "node", type: m[2] || null });
  }
  for (const m of text.matchAll(edgeRegex)) {
    if (m[1]) bindings.set(m[1], { kind: "edge", type: m[2] || null });
  }
  return bindings;
}

// ── Dark/light theme helper ───────────────────────────────────────────
let _currentThemeExt = null;

function buildThemeExtension(oneDark, isDark) {
  return isDark ? oneDark : [];
}

// ── Init: dynamically import CM6, mount editor ────────────────────────
async function initCodeMirrorEditor() {
  const mountEl = document.getElementById("cmEditor");
  if (!mountEl) return;

  let codemirrorModules;
  try {
    codemirrorModules = await Promise.all([
      import(`${CM_CDN}/@codemirror/view@6`),
      import(`${CM_CDN}/@codemirror/state@6`),
      import(`${CM_CDN}/@codemirror/language@6`),
      import(`${CM_CDN}/@codemirror/autocomplete@6`),
      import(`${CM_CDN}/@codemirror/commands@6`),
      import(`${CM_CDN}/@codemirror/theme-one-dark@6`),
    ]);
  } catch (err) {
    console.warn("CodeMirror CDN load failed, falling back to textarea:", err);
    mountEl.innerHTML =
      `<textarea id="queryInput" spellcheck="false" style="width:100%;height:100%;">find n from (n) limit 25</textarea>`;
    return;
  }

  const [
    { EditorView, keymap, placeholder: cmPlaceholder, lineNumbers, highlightActiveLine, drawSelection },
    { EditorState, Compartment },
    { StreamLanguage, syntaxHighlighting, defaultHighlightStyle },
    { autocompletion, completionKeymap },
    { defaultKeymap, historyKeymap, history },
    { oneDark },
  ] = codemirrorModules;

  const nqlLang = buildNqlLanguage(StreamLanguage);
  const themeCompartment = new Compartment();
  const isDark = (state?.uiPrefs?.theme ?? "light") === "dark";

  const initialDoc = (() => {
    // Try to read stored tab value
    if (typeof activeTab === "function") {
      const tab = activeTab();
      if (tab?.query_text) return tab.query_text;
    }
    return "find n from (n) limit 25";
  })();

  const editorState = EditorState.create({
    doc: initialDoc,
    extensions: [
      lineNumbers(),
      highlightActiveLine(),
      drawSelection(),
      history(),
      syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
      nqlLang,
      autocompletion({
        override: [(context) => buildNqlAutocomplete(context)],
        defaultKeymap: true,
      }),
      keymap.of([...defaultKeymap, ...historyKeymap, ...completionKeymap]),
      themeCompartment.of(buildThemeExtension(oneDark, isDark)),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) {
          const text = update.state.doc.toString();
          if (typeof schedulePersistActiveTabQuery === "function") {
            schedulePersistActiveTabQuery(text);
          }
          if (typeof refreshGraphDirtyState === "function") {
            refreshGraphDirtyState();
          }
        }
      }),
      EditorView.theme({
        "&": {
          fontSize: "13px",
          height: "100%",
          maxHeight: "100%",
        },
        ".cm-scroller": { overflow: "auto", fontFamily: "var(--font-mono)", lineHeight: "1.6" },
        ".cm-content": { padding: "10px 14px" },
        ".cm-gutters": { background: "var(--surface-2)", borderRight: "1px solid var(--border)", color: "var(--muted)" },
        ".cm-activeLineGutter": { background: "var(--surface-3)" },
        ".cm-activeLine": { background: "var(--surface-3)" },
        ".cm-focused": { outline: "none" },
        ".cm-tooltip.cm-tooltip-autocomplete": {
          background: "var(--surface-2)",
          border: "1px solid var(--border)",
          borderRadius: "8px",
          fontSize: "12px",
          boxShadow: "0 8px 24px rgba(0,0,0,0.18)",
        },
        ".cm-completionLabel": { color: "var(--fg)" },
        ".cm-completionDetail": { color: "var(--muted)", fontStyle: "normal" },
        ".cm-completionMatchedText": { textDecoration: "none", fontWeight: "700", color: "var(--accent)" },
        "&.cm-focused .cm-selectionBackground, .cm-selectionBackground": { background: "var(--accent-muted)" },
      }),
      EditorView.domEventHandlers({
        keydown(event) {
          // Cmd/Ctrl+Enter → Run
          if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
            event.preventDefault();
            if (typeof runQuery === "function") runQuery();
          }
        },
      }),
    ],
  });

  _cmView = new EditorView({ state: editorState, parent: mountEl });

  // Store compartment ref for theme updates
  _cmView._themeCompartment = themeCompartment;
  _cmView._oneDark = oneDark;
  _cmView._EditorView = EditorView;

  console.info("[NDBStudio] CodeMirror 6 editor mounted.");
}

// ── Theme sync: called when user toggles dark/light ───────────────────
function syncEditorTheme(isDark) {
  if (!_cmView || !_cmView._themeCompartment) return;
  const { _themeCompartment, _oneDark, _EditorView } = _cmView;
  _cmView.dispatch({
    effects: _themeCompartment.reconfigure(buildThemeExtension(_oneDark, isDark)),
  });
}

// ── Stubs to keep backward compatibility with rest of codebase ────────
// These replace the old syncEditorHighlight / updateAutocomplete / insertAutocomplete
// that were called by main.js event listeners for the textarea.
function syncEditorHighlight() { /* no-op: CM6 handles its own rendering */ }
function updateAutocomplete()  { /* no-op: CM6 autocompletion is native   */ }
function renderAutocomplete()  { /* no-op: CM6 autocompletion is native   */ }
function insertAutocomplete()  { /* no-op: CM6 autocompletion is native   */ }

// ── Boot ──────────────────────────────────────────────────────────────
// initCodeMirrorEditor() is called from bootstrap() in api.js after the
// DOM and session state are ready.
