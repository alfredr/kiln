import wasmInit, { trace, match_selector, init_runtime, dispatch_event } from './pkg/kiln_doc_wasm.js';

// ── Scenarios ──

const SCENARIOS = {
  headings: `# Introduction {#sec:intro}

## Background {#sec:bg}

Some text here.

## Methods {#sec:methods}

# Results {#sec:results}

## Analysis {#sec:analysis}`,

  figures: `{@var(fig, 0, format="1")}
{@init(figure, series="fig")}
{@init(figure, label="Figure {n}")}
{@on([series="fig"], @inc(fig), @store(number, fig))}

![Architecture](arch.png){#fig:arch}

![Pipeline](pipeline.png){#fig:pipeline}

See @fig:arch and @fig:pipeline.`,

  'def-use': `{@var(sec, 0, format="1")}
{@var(sub, 0, format="1")}
{@on(enter, section > heading, @inc(sec), @reset(sub, 0), @store(number, sec))}
{@on(enter, section > section > heading, @inc(sub), @store(number, sub))}
{@content(heading, "{number}. ")}

# First {#sec:first}

## Sub A {#sec:a}

## Sub B {#sec:b}

# Second {#sec:second}`,

  inits: `{@init(heading, numbering=true)}
{@init(.wide, width="100%")}
{@init(figure, label="Figure {n}")}

# Intro {#sec:intro .wide}

![Normal](a.png){#fig:normal}

![Special](b.png){#fig:special label="Diagram {n}"}`,

  crossrefs: `# Introduction {#sec:intro}

## Background {#sec:bg}

See @sec:intro for context.

See @sec:bg for details.

# Conclusion {#sec:conclusion}

Refer back to @sec:bg.`,

  click: `{@var(count, 0, format="1")}
{@on(click, heading, @inc(count), @store(number, count))}
{@content(heading, "{number}. ")}

# Apple {#fruit:apple}

# Banana {#fruit:banana}

# Cherry {#fruit:cherry}`,
};

// ── State ──

let traceData = null;
let currentIdx = -1;
let currentAst = null;      // AST snapshot for current step
let inspectedId = null;     // node id selected by click (cleared on step)
let currentNodeProps = {};  // node_id -> properties overlay (accumulated mid-pass)
let liveMode = false;       // true when interactive runtime is active

// ── DOM refs ──

const editor = document.getElementById('editor');
const scenarioSelect = document.getElementById('scenario');
const parseBtn = document.getElementById('parse-btn');
const liveBtn = document.getElementById('live-btn');
const stageLabel = document.getElementById('stage-label');
const treeContainer = document.getElementById('tree-container');
const treeSvg = document.getElementById('tree-svg');
const docPreview = document.getElementById('doc-preview');
const timeline = document.getElementById('timeline');
const eventDetail = document.getElementById('event-detail');
const statePanel = document.getElementById('state-panel');
const stateContent = document.getElementById('state-content');
const eventCounter = document.getElementById('event-counter');
const prevBtn = document.getElementById('prev-btn');
const nextBtn = document.getElementById('next-btn');
const selectorInput = document.getElementById('selector-input');
const selectorChips = document.getElementById('selector-chips');

// ── Init ──

async function main() {
  await wasmInit();

  scenarioSelect.addEventListener('change', loadScenario);
  parseBtn.addEventListener('click', parseInput);
  liveBtn.addEventListener('click', startLive);
  prevBtn.addEventListener('click', () => step(-1));
  nextBtn.addEventListener('click', () => step(1));

  // Delegated click on tree SVG nodes
  treeSvg.addEventListener('click', (e) => {
    const g = e.target.closest('g.node');
    if (!g) return;
    const id = g.dataset.id;
    if (!id) return;
    if (liveMode) { liveDispatch('click', id); }
    else { inspectNode(id); }
  });

  // Delegated click on doc preview nodes
  docPreview.addEventListener('click', (e) => {
    const el = e.target.closest('.doc-node[data-id]');
    if (!el) return;
    e.stopPropagation();
    const id = el.dataset.id;
    if (liveMode) { liveDispatch('click', id); }
    else { inspectNode(id); }
  });

  // Selector bar: hover on chips
  selectorChips.addEventListener('mouseover', (e) => {
    const chip = e.target.closest('.selector-chip');
    if (chip) highlightMatches(chip.dataset.selector);
  });
  selectorChips.addEventListener('mouseout', () => clearMatchHighlights());

  // Selector bar: custom input
  let selectorTimer = null;
  selectorInput.addEventListener('input', () => {
    clearTimeout(selectorTimer);
    selectorTimer = setTimeout(() => highlightMatches(selectorInput.value), 150);
  });
  selectorInput.addEventListener('blur', () => clearMatchHighlights());

  document.addEventListener('keydown', (e) => {
    if (e.target === editor || e.target === selectorInput) {
      if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
        e.preventDefault();
        parseInput();
      }
      return;
    }
    if (e.key === 'ArrowUp' || e.key === 'ArrowLeft') { e.preventDefault(); step(-1); }
    if (e.key === 'ArrowDown' || e.key === 'ArrowRight') { e.preventDefault(); step(1); }
  });

  loadScenario();
}

function loadScenario() {
  editor.value = SCENARIOS[scenarioSelect.value] || '';
  parseInput();
}

function parseInput() {
  exitLive();
  try {
    const raw = trace(editor.value);
    const parsed = JSON.parse(raw);
    if (parsed.error) {
      traceData = null;
      treeSvg.innerHTML = `<text x="20" y="30" fill="${CSS_ACCENT}" font-size="13">${esc(parsed.error)}</text>`;
      docPreview.innerHTML = '';
      return;
    }
    traceData = parsed;
    buildTimeline();
    jumpTo(0);
  } catch (e) {
    traceData = null;
    treeSvg.innerHTML = `<text x="20" y="30" fill="${CSS_ACCENT}" font-size="13">${esc(String(e))}</text>`;
    docPreview.innerHTML = '';
  }
}

// ── Live mode ──

function startLive() {
  liveMode = true;
  traceData = null;
  currentIdx = -1;
  inspectedId = null;
  currentNodeProps = {};

  try {
    const raw = init_runtime(editor.value);
    const doc = JSON.parse(raw);
    if (doc.error) {
      treeSvg.innerHTML = `<text x="20" y="30" fill="${CSS_ACCENT}" font-size="13">${esc(doc.error)}</text>`;
      return;
    }
    currentAst = doc;
    stageLabel.textContent = '(live)';
    renderTree(doc, null);
    renderDocPreview(doc, null);
    timeline.innerHTML = '<div class="pg-empty" style="padding:0.5rem">Live mode — click nodes to dispatch events</div>';
    eventDetail.innerHTML = '';
    stateContent.innerHTML = '';
    statePanel.classList.add('state-empty');
    eventCounter.textContent = 'live';
  } catch (e) {
    treeSvg.innerHTML = `<text x="20" y="30" fill="${CSS_ACCENT}" font-size="13">${esc(String(e))}</text>`;
  }
}

function liveDispatch(event, nodeId) {
  try {
    const raw = dispatch_event(event, nodeId);
    const result = JSON.parse(raw);
    if (result.error) return;

    currentAst = result.ast;
    inspectedId = nodeId;
    renderTree(result.ast, nodeId);
    renderDocPreview(result.ast, nodeId);

    // Show dispatch result in the inspector
    const node = findNodeById(result.ast.content, nodeId);
    if (node) showNodeDetail(node);

    // Show changed info in state panel
    if (result.changed.length) {
      statePanel.classList.remove('state-empty');
      stateContent.innerHTML = `<div class="pg-group"><div class="pg-header">dispatch</div>`
        + `<div class="pg-row"><span class="pg-key">event</span><span class="pg-val">${esc(event)}</span></div>`
        + `<div class="pg-row"><span class="pg-key">node</span><span class="pg-val">${esc(nodeId)}</span></div>`
        + `<div class="pg-row"><span class="pg-key">changed</span><span class="pg-val pg-changed">${esc(result.changed.join(', '))}</span></div>`
        + `</div>`;
    }
  } catch (e) {
    // ignore errors from dispatch
  }
}

function exitLive() {
  liveMode = false;
  stageLabel.textContent = '';
}

// ── Timeline ──

function buildTimeline() {
  timeline.innerHTML = '';
  if (!traceData) return;
  traceData.events.forEach((ev, i) => {
    const row = document.createElement('div');
    row.className = 'timeline-event';
    row.dataset.idx = i;
    row.addEventListener('click', () => jumpTo(i));

    const { type, summary } = summarizeEvent(ev);
    const isPass = type === 'PassStart' || type === 'PassEnd';

    row.innerHTML = `
      <span class="ev-idx">${i}</span>
      <span class="ev-type${isPass ? ' ev-pass' : ''}">${type}</span>
      <span class="ev-summary">${esc(summary)}</span>
    `;
    timeline.appendChild(row);
  });
}

function summarizeEvent(ev) {
  if (typeof ev === 'string') return { type: ev, summary: '' };
  const type = Object.keys(ev)[0];
  const d = ev[type];

  switch (type) {
    case 'PassStart': return { type, summary: d.name };
    case 'PassEnd': return { type, summary: d.name };
    case 'NodeEnter': return { type: d.pass, summary: `enter ${d.kind} ${d.node_id}` };
    case 'NodeExit': return { type: d.pass, summary: `exit ${d.node_id}` };
    case 'Hoist': return { type, summary: `${d.node_id} (${d.decls}d, ${d.inits}i)` };
    case 'InitApplied': return { type: 'Init=', summary: `${d.node_id}.${d.property} = ${valStr(d.value)}` };
    case 'InitSkipped': return { type: 'Init-', summary: `${d.node_id}.${d.property} (exists)` };
    case 'DeclareVar': return { type: '@var', summary: `${d.name} = ${valStr(d.value)}` };
    case 'RegisterTrigger': return { type: '@on', summary: `${d.event} ${d.selector}` };
    case 'DefRegister': return { type: '@def', summary: `${d.name}(${d.params.join(', ')})` };
    case 'UseExpand': return { type: '@use', summary: `${d.name} ${JSON.stringify(d.bindings)}` };
    case 'TriggerMatch': return { type: 'Match', summary: `${d.node_id} <- ${d.event} ${shortSelector(d.selector)}` };
    case 'ActionExec': return { type: 'Action', summary: `${shortAction(d.action)} on ${d.node_id}` };
    case 'PrependText': return { type: 'Prepend', summary: `${d.node_id} <- "${truncate(d.text, 30)}"` };
    case 'AppendText': return { type: 'Append', summary: `${d.node_id} <- "${truncate(d.text, 30)}"` };
    case 'ResolveRef': return { type: 'XRef', summary: `${d.target} -> "${d.resolved_text}"` };
    default: return { type, summary: JSON.stringify(d).slice(0, 60) };
  }
}

// ── Navigation ──

function jumpTo(idx) {
  if (!traceData || idx < 0 || idx >= traceData.events.length) return;
  currentIdx = idx;

  // Update timeline selection
  timeline.querySelectorAll('.timeline-event').forEach((el, i) => {
    el.classList.toggle('active', i === idx);
  });
  const activeRow = timeline.querySelector('.timeline-event.active');
  if (activeRow) activeRow.scrollIntoView({ block: 'nearest' });

  eventCounter.textContent = `${idx + 1} / ${traceData.events.length}`;

  inspectedId = null; // clear inspect on step

  const ev = traceData.events[idx];
  const activeId = evNodeId(ev);

  // Find the most recent PassEnd AST, or use initial_ast
  const { ast, passName } = findCurrentAst(idx);
  currentAst = ast;

  stageLabel.textContent = `- ${passName}`;

  const rs = buildRunningState(idx);
  currentNodeProps = rs.nodeProps;
  currentRunningState = rs;

  renderTree(ast, activeId);
  renderDocPreview(ast, activeId);
  showEventDetail(ev);
  renderRunningState(rs);
  updateSelectorBar();
}

function step(delta) {
  if (!traceData) return;
  const next = currentIdx + delta;
  if (next >= 0 && next < traceData.events.length) jumpTo(next);
}

function evNodeId(ev) {
  if (typeof ev === 'string') return null;
  const type = Object.keys(ev)[0];
  return ev[type]?.node_id || null;
}

function findCurrentAst(idx) {
  let ast = traceData.initial_ast;
  let passName = 'Parsed';

  // Track which pass we're inside
  let currentPass = null;
  for (let i = 0; i <= idx; i++) {
    const e = traceData.events[i];
    const t = Object.keys(e)[0];
    if (t === 'PassStart') {
      currentPass = e.PassStart.name;
    } else if (t === 'PassEnd') {
      ast = e.PassEnd.ast;
      passName = e.PassEnd.name;
      currentPass = null;
    }
  }

  if (currentPass) passName = currentPass;
  return { ast, passName };
}

// ── Event detail ──

function showEventDetail(ev) {
  const type = Object.keys(ev)[0];
  const d = ev[type];
  const row = (k, v, cls) => `<div class="pg-row"><span class="pg-key">${esc(k)}</span><span class="pg-val${cls ? ' ' + cls : ''}">${esc(String(v))}</span></div>`;
  const header = (label) => `<div class="pg-header">${esc(label)}</div>`;

  let html = `<div class="pg-group">${header(type)}`;

  switch (type) {
    case 'PassStart':
    case 'PassEnd':
      html += row('pass', d.name);
      break;
    case 'NodeEnter':
      html += row('pass', d.pass);
      html += row('node', d.node_id);
      html += row('kind', d.kind);
      break;
    case 'NodeExit':
      html += row('pass', d.pass);
      html += row('node', d.node_id);
      break;
    case 'Hoist':
      html += row('node', d.node_id);
      html += row('declarations', d.decls);
      html += row('inits', d.inits);
      break;
    case 'InitApplied':
      html += row('node', d.node_id);
      html += row('property', d.property);
      html += row('value', valStr(d.value));
      html += row('from', d.source);
      break;
    case 'InitSkipped':
      html += row('node', d.node_id);
      html += row('property', d.property);
      break;
    case 'DeclareVar':
      html += row('name', d.name);
      html += row('value', valStr(d.value));
      break;
    case 'RegisterTrigger':
      html += row('event', d.event);
      html += row('selector', d.selector);
      html += row('actions', d.actions.join(', '));
      break;
    case 'DefRegister':
      html += row('name', d.name);
      html += row('params', d.params.join(', '));
      break;
    case 'UseExpand':
      html += row('name', d.name);
      for (const [k, v] of Object.entries(d.bindings)) html += row(k, v);
      break;
    case 'TriggerMatch':
      html += row('node', d.node_id);
      html += row('event', d.event);
      html += row('selector', d.selector);
      break;
    case 'ActionExec': {
      html += row('node', d.node_id);
      html += row('action', d.action);
      for (let i = currentIdx - 1; i >= 0; i--) {
        const prev = traceData.events[i];
        const pt = Object.keys(prev)[0];
        if (pt === 'TriggerMatch' && prev.TriggerMatch.node_id === d.node_id) {
          html += row('trigger', `${prev.TriggerMatch.event} ${prev.TriggerMatch.selector}`);
          break;
        }
        if (pt === 'NodeEnter' || pt === 'NodeExit') break;
      }
      html += `</div>`; // close event group
      const before = d.state_before || {};
      const after = d.state_after || {};
      const allKeys = new Set([...Object.keys(before), ...Object.keys(after)]);
      let hasDiffs = false;
      let diffHtml = '';
      for (const k of allKeys) {
        const bv = valStr(before[k]);
        const av = valStr(after[k]);
        if (bv !== av) {
          hasDiffs = true;
          diffHtml += `<div class="pg-row"><span class="pg-key">${esc(k)}</span>`;
          diffHtml += `<span class="pg-val"><span class="var-old">${esc(bv)}</span>`;
          diffHtml += ` <span class="var-arrow">&rarr;</span> `;
          diffHtml += `<span class="var-changed">${esc(av)}</span></span></div>`;
        }
      }
      if (hasDiffs) html += `<div class="pg-group">${header('state changes')}${diffHtml}</div>`;
      // node_properties
      const np = d?.node_properties;
      if (np) {
        const npKeys = Object.keys(np).filter(k => !k.startsWith('__'));
        if (npKeys.length) {
          html += `<div class="pg-group">${header('node properties')}`;
          for (const k of npKeys) html += row(k, valStr(np[k]));
          html += `</div>`;
        }
      }
      eventDetail.innerHTML = html;
      return; // early return — already closed all groups
    }
    case 'PrependText':
    case 'AppendText':
      html += row('node', d.node_id);
      html += row('text', d.text);
      break;
    case 'ResolveRef':
      html += row('node', d.node_id);
      html += row('target', d.target);
      html += row('resolved', d.resolved_text);
      break;
  }

  html += `</div>`; // close event group

  // Show node properties for any event that carries them
  const np = d?.node_properties;
  if (np) {
    const npKeys = Object.keys(np).filter(k => !k.startsWith('__'));
    if (npKeys.length) {
      html += `<div class="pg-group"><div class="pg-header">node properties</div>`;
      for (const k of npKeys) html += row(k, valStr(np[k]));
      html += `</div>`;
    }
  }

  eventDetail.innerHTML = html;
}

// ── Running state ──

function buildRunningState(upToIdx) {
  let passName = 'Parsed';
  let hoist = { totalDecls: 0, totalInits: 0, nodes: [] };
  let cascade = [];
  let execute = { vars: {}, prevVars: {}, triggers: [], defs: [], lastAction: null };
  let refs = [];
  let nodeProps = {};  // node_id -> accumulated properties

  for (let i = 0; i <= upToIdx; i++) {
    const ev = traceData.events[i];
    const type = Object.keys(ev)[0];
    if (type === 'PassStart') { passName = ev.PassStart.name; continue; }
    if (type === 'PassEnd') { passName = ev.PassEnd.name + ' (done)'; nodeProps = {}; continue; }

    // Extract node_properties from any event that carries it
    const d = ev[type];
    if (d?.node_properties && d?.node_id) {
      nodeProps[d.node_id] = { ...d.node_properties };
    }

    switch (type) {
      case 'Hoist':
        hoist.totalDecls += ev.Hoist.decls;
        hoist.totalInits += ev.Hoist.inits;
        hoist.nodes.push({ id: ev.Hoist.node_id, decls: ev.Hoist.decls, inits: ev.Hoist.inits });
        break;
      case 'InitApplied':
        cascade.push(ev.InitApplied);
        break;
      case 'DeclareVar':
        execute.prevVars = { ...execute.vars };
        execute.vars = { ...execute.vars, [ev.DeclareVar.name]: ev.DeclareVar.value };
        execute.lastAction = null;
        break;
      case 'RegisterTrigger':
        execute.triggers.push(ev.RegisterTrigger);
        break;
      case 'DefRegister':
        execute.defs.push(ev.DefRegister);
        break;
      case 'ActionExec':
        execute.prevVars = ev.ActionExec.state_before || {};
        execute.vars = { ...ev.ActionExec.state_after };
        execute.lastAction = ev.ActionExec;
        break;
      case 'ResolveRef':
        refs.push(ev.ResolveRef);
        break;
    }
  }
  return { passName, hoist, cascade, execute, refs, nodeProps };
}

function renderRunningState(rs) {
  statePanel.classList.remove('state-empty');
  const pass = rs.passName;
  let html = '';

  if (pass === 'Hoist' || pass === 'Hoist (done)') {
    html += `<div class="state-label">Hoist \u2014 ${rs.hoist.totalDecls}d ${rs.hoist.totalInits}i</div>`;
    for (const n of rs.hoist.nodes) {
      html += `<div class="state-var"><span class="var-name">${esc(n.id)}</span>`;
      html += `<span class="var-value">${n.decls}d ${n.inits}i</span></div>`;
    }
  } else if (pass === 'Cascade' || pass === 'Cascade (done)') {
    html += `<div class="state-label">Cascade \u2014 ${rs.cascade.length} properties</div>`;
    for (const c of rs.cascade) {
      html += `<div class="state-var">`;
      html += `<span class="var-name">${esc(c.node_id)}</span>`;
      html += `<span class="var-value">.${esc(c.property)} = ${esc(valStr(c.value))}</span>`;
      html += `</div>`;
    }
  } else if (pass === 'Execute' || pass === 'Execute (done)') {
    // Variables
    const varKeys = Object.keys(rs.execute.vars);
    if (varKeys.length) {
      html += `<div class="state-label">Variables</div>`;
      for (const k of varKeys) {
        const cur = valStr(rs.execute.vars[k]);
        const prev = valStr(rs.execute.prevVars[k]);
        const changed = prev !== cur;
        const isNew = rs.execute.prevVars[k] === undefined;
        html += `<div class="state-var${isNew ? ' var-new' : ''}">`;
        html += `<span class="var-name">${esc(k)}</span>`;
        if (changed && !isNew) {
          html += `<span class="var-old">${esc(prev)}</span>`;
          html += `<span class="var-arrow">&rarr;</span>`;
          html += `<span class="var-changed">${esc(cur)}</span>`;
        } else {
          html += `<span class="${changed ? 'var-changed' : 'var-value'}">${esc(cur)}</span>`;
        }
        html += `</div>`;
      }
    }
    // Triggers
    if (rs.execute.triggers.length) {
      html += `<div class="state-label">Triggers</div>`;
      for (const t of rs.execute.triggers) {
        html += `<div class="state-var">`;
        html += `<span class="var-name">@on</span>`;
        html += `<span class="var-value">${esc(t.event)} ${esc(t.selector)} \u2192 ${t.actions.map(a => esc(a)).join(', ')}</span>`;
        html += `</div>`;
      }
    }
    // Defs
    if (rs.execute.defs.length) {
      html += `<div class="state-label">Defs</div>`;
      for (const d of rs.execute.defs) {
        html += `<div class="state-var">`;
        html += `<span class="var-name">@def</span>`;
        html += `<span class="var-value">${esc(d.name)}(${d.params.map(p => esc(p)).join(', ')})</span>`;
        html += `</div>`;
      }
    }
    // Last action indicator
    if (rs.execute.lastAction) {
      const a = rs.execute.lastAction;
      html += `<div class="state-label">Last Action</div>`;
      html += `<div class="state-var var-new">`;
      html += `<span class="var-name">\u25b8</span>`;
      html += `<span class="var-changed">${esc(a.action)} on ${esc(a.node_id)}</span>`;
      html += `</div>`;
    }
  } else if (pass === 'CrossRefs' || pass === 'CrossRefs (done)') {
    html += `<div class="state-label">CrossRefs \u2014 ${rs.refs.length} resolved</div>`;
    for (const r of rs.refs) {
      html += `<div class="state-var">`;
      html += `<span class="var-name">@${esc(r.target)}</span>`;
      html += `<span class="var-value">\u2192 "${esc(r.resolved_text)}"</span>`;
      html += `</div>`;
    }
  } else {
    // Parsed or unknown — show nothing
    statePanel.classList.add('state-empty');
    return;
  }

  if (!html) {
    statePanel.classList.add('state-empty');
    return;
  }
  stateContent.innerHTML = html;
}

// ── Node inspection (click) ──

function inspectNode(id) {
  if (!currentAst || !id) return;
  inspectedId = id;

  const node = findNodeById(currentAst.content, id);
  if (!node) return;

  showNodeDetail(node);

  // Re-render tree and doc to apply inspected highlight consistently
  const activeId = evNodeId(traceData.events[currentIdx]);
  renderTree(currentAst, activeId);
  renderDocPreview(currentAst, activeId);

  const el = docPreview.querySelector('.doc-inspected');
  if (el) el.scrollIntoView({ block: 'nearest', behavior: 'auto' });
}

function findNodeById(node, id) {
  if (node.attrs?.id === id) return node;
  for (const child of (node.children || [])) {
    const found = findNodeById(child, id);
    if (found) return found;
  }
  return null;
}

function showNodeDetail(node) {
  const kind = node.kind;
  const kindName = typeof kind === 'string' ? kind : Object.keys(kind)[0];
  const kindData = typeof kind === 'string' ? null : kind[kindName];
  const row = (k, v, cls) => `<div class="pg-row"><span class="pg-key">${esc(k)}</span><span class="pg-val${cls ? ' ' + cls : ''}">${esc(String(v))}</span></div>`;
  const header = (label, count) => `<div class="pg-header">${esc(label)}${count != null ? ` (${count})` : ''}</div>`;

  let html = '';

  // ── node.kind ──
  html += `<div class="pg-group">`;
  html += header('kind');
  html += row('type', kindName);
  if (kindData && typeof kindData === 'object') {
    for (const [k, v] of Object.entries(kindData))
      html += row(k, typeof v === 'string' ? v : JSON.stringify(v));
  } else if (kindData !== null && kindData !== undefined) {
    html += row('value', String(kindData));
  }
  html += `</div>`;

  // ── node.attrs (identity) ──
  html += `<div class="pg-group">`;
  html += header('attrs');
  html += row('id', node.attrs.id, node.attrs.auto_id ? 'pg-auto' : '');
  if (node.attrs.auto_id) html += row('auto_id', 'true', 'pg-auto');
  const classes = node.attrs.classes || [];
  html += row('classes', classes.length ? classes.map(c => '.' + c).join(' ') : '(none)', classes.length ? '' : 'pg-auto');
  html += `</div>`;

  // ── node.attrs.properties ──
  const baseProps = node.attrs.properties || {};
  const overlay = currentNodeProps[node.attrs.id] || {};
  const props = { ...baseProps, ...overlay };
  const propKeys = Object.keys(props).filter(k => !k.startsWith('__'));
  html += `<div class="pg-group">`;
  html += header('attrs.properties', propKeys.length);
  if (propKeys.length) {
    for (const k of propKeys) {
      const isOverlay = overlay[k] !== undefined && valStr(overlay[k]) !== valStr(baseProps[k]);
      html += row(k, valStr(props[k]), isOverlay ? 'pg-changed' : '');
    }
  } else {
    html += `<div class="pg-empty">empty</div>`;
  }
  html += `</div>`;

  // ── node.attrs.declarations ──
  const decls = node.attrs.declarations || [];
  html += `<div class="pg-group">`;
  html += header('attrs.declarations', decls.length);
  if (decls.length) {
    for (const d of decls) {
      const dk = typeof d === 'string' ? d : Object.keys(d)[0];
      const dd = typeof d === 'string' ? null : d[dk];
      if (dk === 'VarDecl') {
        html += row('@var ' + dd.name, valStr(dd.value));
      } else if (dk === 'OnDirective') {
        const ev = typeof dd.event === 'string' ? dd.event : Object.keys(dd.event)[0];
        const sel = dd.selector || '';
        html += row('@on ' + ev, sel);
      } else if (dk === 'Def') {
        html += row('@def ' + (dd.name || ''), `${(dd.body || []).length} decls, ${(dd.inits || []).length} inits`);
      } else if (dk === 'Use') {
        html += row('@use ' + (dd.name || ''), dd.overrides?.length ? `${dd.overrides.length} overrides` : '');
      } else {
        html += row(dk, JSON.stringify(dd));
      }
    }
  } else {
    html += `<div class="pg-empty">empty</div>`;
  }
  html += `</div>`;

  // ── node.attrs.inits ──
  const inits = node.attrs.inits || [];
  html += `<div class="pg-group">`;
  html += header('attrs.inits', inits.length);
  if (inits.length) {
    for (const init of inits) {
      const sel = typeof init.selector === 'string' ? init.selector
        : init.selector?.This ? 'this'
        : init.selector?.Expr ? fmtSelector(init.selector.Expr)
        : JSON.stringify(init.selector);
      html += row(sel + ' → ' + init.property, valStr(init.value));
    }
  } else {
    html += `<div class="pg-empty">empty</div>`;
  }
  html += `</div>`;

  // ── node.attrs.frontmatter ──
  if (node.attrs.frontmatter) {
    html += `<div class="pg-group">`;
    html += header('attrs.frontmatter');
    html += row('yaml', JSON.stringify(node.attrs.frontmatter));
    html += `</div>`;
  }

  // ── node.children ──
  const ch = node.children || [];
  html += `<div class="pg-group">`;
  html += header('children', ch.length);
  if (ch.length) {
    const kindCounts = {};
    for (const c of ch) {
      const ck = typeof c.kind === 'string' ? c.kind : Object.keys(c.kind)[0];
      kindCounts[ck] = (kindCounts[ck] || 0) + 1;
    }
    for (const [k, n] of Object.entries(kindCounts)) {
      html += row(k, n);
    }
  } else {
    html += `<div class="pg-empty">none</div>`;
  }
  html += `</div>`;

  eventDetail.innerHTML = html;
}

// ── Document preview ──

function renderDocPreview(ast, activeId) {
  const root = ast.content;
  if (!root) {
    docPreview.innerHTML = '<div class="doc-empty">No content</div>';
    return;
  }

  docPreview.innerHTML = renderDocNode(root, activeId);

  // Scroll active into view
  if (activeId) {
    const el = docPreview.querySelector('.doc-active');
    if (el) el.scrollIntoView({ block: 'nearest', behavior: 'auto' });
  }
}

function renderDocNode(node, activeId) {
  const kind = node.kind;
  const id = node.attrs?.id || '';
  const activeCls = (activeId && id === activeId) ? ' doc-active' : '';
  const inspCls = (inspectedId && id === inspectedId) ? ' doc-inspected' : '';
  const dataAttr = id ? ` data-id="${esc(id)}"` : '';

  // Leaf text
  if (typeof kind === 'object' && kind.Text !== undefined) {
    return `<span class="doc-node doc-text${activeCls}${inspCls}"${dataAttr}>${esc(kind.Text)}</span>`;
  }
  if (typeof kind === 'object' && kind.Code !== undefined) {
    return `<span class="doc-node doc-code${activeCls}${inspCls}"${dataAttr}>${esc(kind.Code)}</span>`;
  }

  const kindName = typeof kind === 'string' ? kind : Object.keys(kind)[0];
  const kindData = typeof kind === 'string' ? null : kind[kindName];
  const children = (node.children || []).map(c => renderDocNode(c, activeId)).join('');

  switch (kindName) {
    case 'Section': {
      const level = kindData?.level || 0;
      return `<div class="doc-node doc-section${activeCls}${inspCls}"${dataAttr}>${children}</div>`;
    }

    case 'Heading': {
      const level = kindData?.level || 1;
      const prefix = '#'.repeat(level) + ' ';
      const hClass = level <= 3 ? ` doc-heading-${level}` : ' doc-heading-3';
      return `<div class="doc-node doc-heading${hClass}${activeCls}${inspCls}"${dataAttr}><span class="doc-muted">${prefix}</span>${children}</div>`;
    }

    case 'Paragraph':
      return `<div class="doc-node doc-para${activeCls}${inspCls}"${dataAttr}>${children}</div>`;

    case 'Figure': {
      const figId = id && !node.attrs?.auto_id ? id : '';
      return `<div class="doc-node doc-figure${activeCls}${inspCls}"${dataAttr}>[figure${figId ? ': ' + esc(figId) : ''}] ${children}</div>`;
    }

    case 'Image': {
      const url = kindData?.url || '';
      return `<span class="doc-node doc-muted${activeCls}${inspCls}"${dataAttr}>[img: ${esc(truncate(url, 20))}]</span>`;
    }

    case 'Caption':
      return `<span class="doc-node${activeCls}${inspCls}"${dataAttr}>${children}</span>`;

    case 'CrossRef': {
      const target = kindData?.target || '?';
      const resolved = node.attrs?.properties?.text;
      const resolvedText = resolved ? valStr(resolved) : null;
      if (resolvedText && resolvedText !== '-') {
        return `<span class="doc-node doc-ref-resolved${activeCls}${inspCls}"${dataAttr}>${esc(resolvedText.replace(/^"|"$/g, ''))}</span>`;
      }
      return `<span class="doc-node doc-ref${activeCls}${inspCls}"${dataAttr}>@${esc(target)}</span>`;
    }

    case 'CodeBlock': {
      const lang = kindData?.lang || '';
      const src = kindData?.source || '';
      return `<div class="doc-node doc-codeblock${activeCls}${inspCls}"${dataAttr}>${lang ? `<span class="doc-muted">[${esc(lang)}]</span>\n` : ''}${esc(src)}</div>`;
    }

    case 'BlockQuote':
      return `<div class="doc-node${activeCls}${inspCls}" style="border-left:2px solid var(--muted);padding-left:0.5rem;margin:0.2rem 0"${dataAttr}>${children}</div>`;

    case 'Emphasis':
      return `<em class="doc-node${activeCls}${inspCls}"${dataAttr}>${children}</em>`;

    case 'Strong':
      return `<strong class="doc-node${activeCls}${inspCls}"${dataAttr}>${children}</strong>`;

    case 'Link': {
      const url = kindData?.url || '';
      return `<span class="doc-node doc-ref${activeCls}${inspCls}"${dataAttr}>[${children || esc(truncate(url, 20))}]</span>`;
    }

    case 'SoftBreak':
    case 'HardBreak':
      return ' ';

    case 'List': {
      const ordered = kindData?.ordered;
      return `<div class="doc-node${activeCls}${inspCls}" style="padding-left:1rem;margin:0.15rem 0"${dataAttr}>${
        (node.children || []).map((c, i) => {
          const bullet = ordered ? `${i + 1}. ` : '- ';
          return `<div><span class="doc-muted">${bullet}</span>${renderDocNode(c, activeId)}</div>`;
        }).join('')
      }</div>`;
    }

    case 'ListItem':
      return `<span class="doc-node${activeCls}${inspCls}"${dataAttr}>${children}</span>`;

    case 'Custom':
      return ''; // directives are hoisted, skip

    default:
      if (children) return `<span class="doc-node${activeCls}${inspCls}"${dataAttr}>${children}</span>`;
      return '';
  }
}

// ── Tree rendering ──

const NODE_W = 110;
const NODE_H = 34;
const H_GAP = 12;
const V_GAP = 44;
const CSS_ACCENT = '#e94560';

function renderTree(ast, activeId) {
  const root = ast.content;
  if (!root) {
    treeSvg.innerHTML = '<text x="20" y="30" fill="#8899aa" font-size="12">No content</text>';
    return;
  }

  const layoutRoot = buildLayout(root);
  assignDepth(layoutRoot, 0);
  assignX(layoutRoot);

  const bounds = treeBounds(layoutRoot);
  const padX = 30, padY = 20;
  const svgW = bounds.maxX + NODE_W + padX * 2;
  const svgH = bounds.maxY + NODE_H + padY * 2;

  treeSvg.setAttribute('viewBox', `0 0 ${svgW} ${svgH}`);
  treeSvg.style.width = svgW + 'px';
  treeSvg.style.height = svgH + 'px';

  let html = drawLinks(layoutRoot, padX, padY);
  html += drawNodes(layoutRoot, padX, padY, activeId);
  treeSvg.innerHTML = html;

  // Scroll active node into view in tree
  if (activeId) {
    const el = treeSvg.querySelector('.node.active');
    if (el) el.scrollIntoView({ block: 'nearest', behavior: 'auto' });
  }
}

function buildLayout(node) {
  const label = kindLabel(node.kind);
  const detail = kindDetail(node);
  const id = node.attrs?.id || '';
  const children = (node.children || []).map(c => buildLayout(c));
  return { label, detail, id, children, x: 0, y: 0 };
}

function assignDepth(node, d) {
  node.y = d * (NODE_H + V_GAP);
  node.children.forEach(c => assignDepth(c, d + 1));
}

function assignX(root) {
  let leafIndex = 0;
  (function walk(node) {
    if (node.children.length === 0) {
      node.x = leafIndex++ * (NODE_W + H_GAP);
      return;
    }
    node.children.forEach(walk);
    const first = node.children[0];
    const last = node.children[node.children.length - 1];
    node.x = (first.x + last.x) / 2;
  })(root);
}

function treeBounds(node) {
  let maxX = node.x, maxY = node.y;
  node.children.forEach(c => {
    const cb = treeBounds(c);
    maxX = Math.max(maxX, cb.maxX);
    maxY = Math.max(maxY, cb.maxY);
  });
  return { maxX, maxY };
}

function drawLinks(node, ox, oy) {
  let s = '';
  for (const c of node.children) {
    const x1 = ox + node.x + NODE_W / 2;
    const y1 = oy + node.y + NODE_H;
    const x2 = ox + c.x + NODE_W / 2;
    const y2 = oy + c.y;
    const my = (y1 + y2) / 2;
    s += `<path class="link" d="M${x1},${y1} C${x1},${my} ${x2},${my} ${x2},${y2}"/>`;
    s += drawLinks(c, ox, oy);
  }
  return s;
}

function drawNodes(node, ox, oy, activeId) {
  const x = ox + node.x;
  const y = oy + node.y;
  const isActive = activeId && node.id === activeId;
  const isInspected = inspectedId && node.id === inspectedId;
  const cls = 'node' + (isActive ? ' active' : '') + (isInspected ? ' inspected' : '');
  let s = `<g class="${cls}" data-id="${esc(node.id)}" transform="translate(${x},${y})">`;
  s += `<rect width="${NODE_W}" height="${NODE_H}"/>`;
  s += `<text class="node-kind" x="${NODE_W/2}" y="14" text-anchor="middle">${esc(node.label)}</text>`;
  if (node.detail) {
    s += `<text class="node-detail" x="${NODE_W/2}" y="26" text-anchor="middle">${esc(node.detail)}</text>`;
  }
  s += '</g>';
  for (const c of node.children) s += drawNodes(c, ox, oy, activeId);
  return s;
}

// ── Selector bar ──

let currentRunningState = null;

function updateSelectorBar() {
  let html = '';
  const triggers = currentRunningState?.execute?.triggers || [];
  for (const t of triggers) {
    html += `<span class="selector-chip" data-selector="${esc(t.selector)}">`;
    html += `<span class="chip-label">@on</span>${esc(t.selector)}`;
    html += `</span>`;
  }
  selectorChips.innerHTML = html;
}

function fmtSelector(expr) {
  if (!expr) return '?';
  if (expr.Simple) {
    const s = expr.Simple;
    let out = s.kind || '';
    if (s.classes?.length) out += s.classes.map(c => '.' + c).join('');
    if (s.id) out += '#' + s.id;
    if (s.attrs?.length) out += s.attrs.map(a => `[${a.name}${a.op || ''}${a.value ? '"' + a.value + '"' : ''}]`).join('');
    return out || '*';
  }
  if (expr.Combinator) {
    const op = expr.Combinator.op === 'Descendant' ? ' ~> ' : ' > ';
    return fmtSelector(expr.Combinator.left) + op + fmtSelector(expr.Combinator.right);
  }
  if (expr.Or) return expr.Or.map(fmtSelector).join(', ');
  if (expr.And) return expr.And.map(fmtSelector).join(' & ');
  return JSON.stringify(expr);
}

function highlightMatches(selectorStr) {
  clearMatchHighlights();
  if (!selectorStr?.trim() || currentIdx < 0) return;
  try {
    const json = match_selector(currentIdx, selectorStr);
    const ids = JSON.parse(json);
    for (const id of ids) {
      const escaped = CSS.escape(id);
      treeSvg.querySelectorAll(`.node[data-id="${escaped}"]`).forEach(el => el.classList.add('matched'));
      docPreview.querySelectorAll(`.doc-node[data-id="${escaped}"]`).forEach(el => el.classList.add('doc-matched'));
    }
  } catch (e) {
    // ignore parse errors from incomplete input
  }
}

function clearMatchHighlights() {
  treeSvg.querySelectorAll('.node.matched').forEach(el => el.classList.remove('matched'));
  docPreview.querySelectorAll('.doc-matched').forEach(el => el.classList.remove('doc-matched'));
}

// ── Helpers ──

function kindLabel(kind) {
  if (typeof kind === 'string') return kind;
  if (!kind) return '?';
  const keys = Object.keys(kind);
  return keys.length === 1 ? keys[0] : '?';
}

function kindDetail(node) {
  const id = node.attrs?.id || '';
  if (id && !node.attrs?.auto_id) return truncate(id, 14);

  const kind = node.kind;
  if (!kind || typeof kind === 'string') return '';
  const key = Object.keys(kind)[0];
  const val = kind[key];
  if (typeof val === 'string') return truncate(val, 14);
  if (val?.level !== undefined) return `L${val.level}`;
  if (val?.target !== undefined) return truncate(val.target, 14);
  if (val?.url !== undefined) return truncate(val.url, 14);
  if (val?.content_type !== undefined) return val.content_type;
  return '';
}

function valStr(v) {
  if (v === undefined || v === null) return '-';
  if (typeof v === 'object') {
    const keys = Object.keys(v);
    if (keys.length === 1) {
      const k = keys[0];
      const inner = v[k];
      if (k === 'Number') return String(inner);
      if (k === 'Text') return `"${inner}"`;
      if (k === 'Bool') return String(inner);
      if (k === 'List') return `[${inner.map(valStr).join(', ')}]`;
      if (k === 'Val') return valStr(inner);
    }
    return JSON.stringify(v);
  }
  return String(v);
}

function shortAction(s) {
  if (s.length > 50) return s.slice(0, 47) + '...';
  return s;
}

function shortSelector(s) {
  if (s.length > 30) return s.slice(0, 27) + '...';
  return s;
}

function truncate(s, n) {
  return s.length > n ? s.slice(0, n - 1) + '\u2026' : s;
}

function esc(s) {
  return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

// ── Go ──

main();
