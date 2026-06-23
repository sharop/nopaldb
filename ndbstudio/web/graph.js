// ── interactive Graph Visualization ──────────────────────────────────

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

    // Use consistent direction for normal calculation based on node ID string comparison
    const isReversed = edge.source > edge.target;
    const absDx = isReversed ? -dx : dx;
    const absDy = isReversed ? -dy : dy;

    let d;
    let mx, my;
    if (total > 1) {
      const offset = ((idx - (total - 1) / 2) * 30);
      const nx = -absDy / dist;
      const ny = absDx / dist;
      const cx = (x1 + x2) / 2 + nx * offset;
      const cy = (y1 + y2) / 2 + ny * offset;
      d = `M ${x1} ${y1} Q ${cx} ${cy} ${x2} ${y2}`;
      mx = (x1 + x2) / 2 + nx * offset * 0.5;
      my = (y1 + y2) / 2 + ny * offset * 0.5;
    } else {
      d = `M ${x1} ${y1} L ${x2} ${y2}`;
      mx = (x1 + x2) / 2;
      my = (y1 + y2) / 2;
    }

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
    g.dataset.edgeIdx = idx;
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
    if (running || state.graphMoveNode) {
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

function teardownGraph() {
  stopGraphAnimation();
  if (state.graphSim) {
    state.graphSim = null;
  }
  state.graphSvgRefs = null;
  state.graphMoveNode = null;
  
  if (els.graphPane) {
    // Explicitly remove all child nodes to help GC
    while (els.graphPane.firstChild) {
      els.graphPane.removeChild(els.graphPane.firstChild);
    }
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
      state.graphMoveNode = nodeId;
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
    
    const edgeGroup = e.target.closest("[data-edge-idx]");
    if (edgeGroup) {
      e.preventDefault();
      state.selectedGraphNodeId = null;
      state.selectedGraphEdgeIdx = parseInt(edgeGroup.dataset.edgeIdx, 10);
      updateSvgPositions();
      updateGraphInspectorPanel();
      return;
    }
    // Pan
    isPanning = true;
    panStart = { x: e.clientX - state.graphPanOffset.x, y: e.clientY - state.graphPanOffset.y };
    svgWrap.classList.add("is-panning");
  });

  svgWrap.addEventListener("mousemove", (e) => {
    if (state.graphMoveNode) {
      const svgPt = screenToSvg(e.clientX, e.clientY);
      state.graphSim?.setNodePosition(state.graphMoveNode, svgPt.x - dragOffset.x, svgPt.y - dragOffset.y);
      return;
    }
    if (isPanning) {
      state.graphPanOffset = { x: e.clientX - panStart.x, y: e.clientY - panStart.y };
      updateGraphTransform();
    }
  });

  const endInteraction = (e) => {
    if (state.graphMoveNode) {
      const moved = clickStart ? Math.hypot(e.clientX - clickStart.x, e.clientY - clickStart.y) : 999;
      const elapsed = clickStart ? Date.now() - clickStart.time : 999;
      const nodeId = state.graphMoveNode;

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
          updateGraphInspectorPanel();
        }
      }
      const pos = state.graphNodePositions.get(nodeId);
      if (pos && !pos.pinned) {
        state.graphSim?.unpinNode(nodeId);
      }
      state.graphMoveNode = null;
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
  let html = "";

  if (subgraph._resultNodeIds && subgraph._resultNodeIds.size > 0) {
    const resultCount = subgraph._resultNodeIds.size;
    const contextCount = subgraph.nodes.length - resultCount;
    html += `<span class="graph-legend-chip is-result-count">
      <span class="graph-legend-dot" style="background:var(--accent); box-shadow: 0 0 4px var(--accent);"></span>
      ${resultCount} Result${resultCount !== 1 ? 's' : ''}
    </span>`;

    if (contextCount > 0) {
      html += `<span class="graph-legend-chip is-context-count" style="opacity: 0.7">
        <span class="graph-legend-dot" style="background:var(--text-muted)"></span>
        ${contextCount} Context Neighbor${contextCount !== 1 ? 's' : ''}
      </span>`;
    }

    html += `<span class="graph-legend-divider" style="color:var(--text-muted); margin:0 8px;">|</span>`;
  }

  const types = [...new Set(subgraph.nodes.map((n) => n.entity_type || n.label).filter(Boolean))].sort();
  html += types.map((type) => {
    const color = getNodeColor(type);
    return `<span class="graph-legend-chip"><span class="graph-legend-dot" style="background:${color}"></span>${escapeHtml(type)}</span>`;
  }).join("");

  els.graphLegend.innerHTML = html;
}

// Update detail panel
function updateGraphInspectorPanel() {
  const sub = state.graphCurrentSubgraph;
  const panel = els.graphInspectorPanel;
  const content = els.graphInspectorContent;
  
  if (!sub || (!state.selectedGraphNodeId && state.selectedGraphEdgeIdx == null)) {
    panel.classList.add("is-empty");
    content.innerHTML = "";
    return;
  }
  
  if (state.selectedGraphNodeId) {
    const node = sub.nodes.find((n) => n.id === state.selectedGraphNodeId);
    if (!node) {
      panel.classList.add("is-empty");
      content.innerHTML = "";
      return;
    }
    panel.classList.remove("is-empty");
    panel.classList.toggle("is-collapsed", state.graphInspectorCollapsed);
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
        state.selectedGraphEdgeIdx = null;
        await refreshGraphSubgraph();
      });
    });
  } else if (state.selectedGraphEdgeIdx != null) {
    const edge = sub.edges[state.selectedGraphEdgeIdx];
    if (!edge) {
      panel.classList.add("is-empty");
      content.innerHTML = "";
      return;
    }
    panel.classList.remove("is-empty");
    panel.classList.toggle("is-collapsed", state.graphInspectorCollapsed);
    content.innerHTML = renderEdgeDetailCard(edge);
  }
}

window.exportGraphSvg = function() {
  if (!state.graphSvgRefs || !state.graphSvgRefs.svg) return null;
  const clone = state.graphSvgRefs.svg.cloneNode(true);
  
  const styleEl = document.createElementNS("http://www.w3.org/2000/svg", "style");
  styleEl.textContent = `
    .graph-svg { background: #0b0c10; font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; }
    .graph-node-circle { fill: #1f2833; stroke: #c0cfd2; stroke-width: 2px; }
    .graph-node-circle.is-result { stroke: #66fcf1; stroke-width: 3.5px; }
    .graph-node-label { fill: #c0cfd2; font-weight: 500; font-size: 13px; }
    .graph-node-subtitle { fill: #45a29e; font-size: 11px; }
    .graph-node-type { fill: #c0cfd2; font-size: 10px; opacity: 0.6; }
    .graph-edge-path { fill: none; stroke: #c0cfd2; stroke-width: 1.8px; opacity: 0.8; }
    .graph-edge-path.is-result { stroke: #66fcf1; stroke-width: 2.2px; opacity: 1; }
    .graph-edge-arrow { fill: #c0cfd2; }
    .graph-edge-arrow.is-result { fill: #66fcf1; }
    .graph-edge-label { fill: #c0cfd2; font-size: 10px; opacity: 0.8; }
  `;
  clone.insertBefore(styleEl, clone.firstChild);
  
  const serializer = new XMLSerializer();
  let source = serializer.serializeToString(clone);
  
  if(!source.includes('xmlns="http://www.w3.org/2000/svg"')){
      source = source.replace(/^<svg/, '<svg xmlns="http://www.w3.org/2000/svg"');
  }
  source = '<?xml version="1.0" standalone="no"?>\r\n' + source;
  return source;
};

