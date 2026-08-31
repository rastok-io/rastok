"use strict";

const controller = sessionStorage.getItem("rastok-controller") || crypto.randomUUID();
sessionStorage.setItem("rastok-controller", controller);
history.replaceState(null, "", location.pathname);

let controls = false;
let currentSnapshot = null;
let historyLimit = 100;

async function post(path) {
  const response = await fetch(path, {
    method: "POST",
    headers: { "X-Rastok-Controller": controller },
  });
  if (!response.ok) throw new Error(await response.text());
  return response;
}

async function claim(take = false) {
  try {
    await post(take ? "/api/control/take" : "/api/control/claim");
    controls = true;
    text("role", "controller");
  } catch (_) {
    controls = false;
    text("role", "observer");
  }
  updateControls();
}

async function refresh() {
  const response = await fetch("/api/snapshot", { cache: "no-store" });
  if (!response.ok) throw new Error(await response.text());
  currentSnapshot = await response.json();
  renderSnapshot(currentSnapshot);
}

function renderSnapshot(snapshot) {
  const execution = snapshot.execution || {};
  const state = execution.state || "paused";
  text("title", `Execution ${state.replace("-", " ")}`);
  text("execution-state", state);
  document.querySelector("#execution-state").className = `pill state-${state}`;
  text("mode", snapshot.mode || "—");
  text("logical-time", formatNanoseconds(execution.logical_time_ns));
  text("awaiting-calls", String(execution.awaiting_outbound_calls || 0));

  const simulation = snapshot.mode === "simulate";
  for (const element of document.querySelectorAll(".simulation-only")) {
    element.hidden = !simulation;
  }
  const scenario = snapshot.scenario?.current;
  text(
    "step",
    simulation && scenario && !scenario.complete && scenario.quiescent
      ? "Step: next action"
      : "Step: next transaction",
  );
  renderGuidance(state, execution.awaiting_outbound_calls || 0, simulation);
  renderFault(snapshot.fault);
  renderScenario(snapshot.scenario, simulation);
  renderTopology(snapshot.topology);
  renderHistory(snapshot.history);
  renderInstances(snapshot.instances);
  renderExchanges(snapshot.exchanges);
  renderCost(snapshot.cost_report);
  updateControls();
}

function renderGuidance(state, awaiting, simulation) {
  const messages = {
    paused: simulation
      ? "Paused at a semantic boundary. Step advances one scenario action or one VM transaction, never both."
      : "Paused at a semantic boundary. Resume continuously or advance one atomic transaction.",
    running: "Running until completion, an assertion boundary, a fault, or a safe pause request.",
    "pause-requested": awaiting > 0
      ? `Safe pause requested; waiting for ${awaiting} open Call outcome${awaiting === 1 ? "" : "s"} without consuming it.`
      : "Safe pause requested; finishing the current transaction boundary.",
    faulted: "The failing transaction committed nothing. Inspect the fault before retrying or closing.",
    completed: "Execution is complete. The retained in-memory history remains inspectable until Close UI.",
  };
  text("guidance", messages[state] || "Execution state unavailable.");
}

function updateControls() {
  const legal = new Set(currentSnapshot?.execution?.legal_commands || []);
  for (const button of document.querySelectorAll("button[data-command]")) {
    button.disabled = !controls || !legal.has(button.dataset.command);
  }
  document.querySelector("#select-scenario").disabled =
    !controls || !legal.has("select-scenario") || !document.querySelector("#scenario-select").value;
  document.querySelector("#take-control").hidden = controls;
}

function renderFault(fault) {
  const panel = document.querySelector("#fault-panel");
  panel.hidden = !fault;
  if (!fault) return;
  text("fault-message", fault.message || "The transaction faulted.");
  text("fault-help", fault.help || "Inspect the retained state before choosing a legal action.");
  text("fault-commit", fault.commit || "Nothing from the faulting transaction was committed.");
}

function renderScenario(scenario, enabled) {
  if (!enabled || !scenario) return;
  const current = scenario.current || {};
  const next = current.next_action ? `next: ${current.next_action}` : "no remaining action";
  text(
    "scenario-progress",
    `${current.simulation || "?"}.${current.scenario || "?"} · ${current.next_action_index || 0}/${current.action_count || 0}`,
  );
  text("scenario-status", `${current.quiescent ? "at action boundary" : "VM work ready"}; ${next}`);
  const select = document.querySelector("#scenario-select");
  const selected = `${current.simulation || ""}\u0000${current.scenario || ""}`;
  const options = [];
  for (const simulation of scenario.available || []) {
    for (const item of simulation.scenarios || []) {
      options.push({
        value: `${simulation.name}\u0000${item.name}`,
        label: item.label
          ? `${simulation.name}.${item.name} — ${item.label}`
          : `${simulation.name}.${item.name}`,
      });
    }
  }
  select.replaceChildren(...options.map((option) => {
    const element = document.createElement("option");
    element.value = option.value;
    element.textContent = option.label;
    element.selected = option.value === selected;
    return element;
  }));
  renderGeneric("scenario", {
    current_action: current.last_action || null,
    next_action: current.next_action || null,
    failures: current.failures || [],
    golden_failures: current.golden_failures || [],
  }, "No scenario selected.");
}

function renderTopology(topology) {
  const target = document.querySelector("#graph");
  if (!topology || !(topology.nodes || []).length) {
    empty(target, "Waiting for a validated topology.");
    return;
  }
  const activity = topology.activity || null;
  const activityPanel = document.createElement("div");
  activityPanel.className = `graph-activity${activity ? ` activity-${activity.kind}` : ""}`;
  const activityLabel = document.createElement("span");
  activityLabel.className = "graph-activity-label";
  activityLabel.textContent = activity?.kind === "transaction" ? "Last Step" : "Latest boundary";
  const activityText = document.createElement("strong");
  activityText.textContent = activity?.detail || "No Step has run yet.";
  activityPanel.append(activityLabel, activityText);

  const ownerStrip = document.createElement("div");
  ownerStrip.className = "graph-owners";
  for (const owner of topology.owners || []) {
    const chip = document.createElement("div");
    const active = activity?.kind === "transaction" && activity.node === owner.path;
    chip.className = `graph-owner${active ? " active" : ""}`;
    const mark = document.createElement("span");
    mark.className = "graph-owner-mark";
    mark.textContent = "×";
    const label = document.createElement("strong");
    label.textContent = owner.label || "App";
    const count = document.createElement("span");
    count.textContent = `${owner.transaction_count || "0"} tx`;
    chip.append(mark, label, count);
    if (active) {
      const ran = document.createElement("b");
      ran.textContent = "JUST RAN";
      chip.append(ran);
    }
    ownerStrip.append(chip);
  }

  const graph = materializeGraph(topology);
  const layout = layoutGraph(graph.nodes, graph.edges);
  const canvas = document.createElement("div");
  canvas.className = "graph-canvas";
  const svg = svgElement("svg", {
    viewBox: `0 0 ${layout.width} ${layout.height}`,
    width: layout.width,
    height: layout.height,
    role: "img",
    "aria-label": "Validated Rastok graph with cumulative envelope traffic",
  });
  svg.classList.add("graph-svg");
  const definitions = svgElement("defs");
  const marker = svgElement("marker", {
    id: "graph-arrow",
    viewBox: "0 0 10 10",
    refX: "9",
    refY: "5",
    markerWidth: "8",
    markerHeight: "8",
    orient: "auto-start-reverse",
  });
  marker.append(svgElement("path", { d: "M 0 0 L 10 5 L 0 10 z" }));
  const activeMarker = svgElement("marker", {
    id: "graph-arrow-active",
    viewBox: "0 0 10 10",
    refX: "9",
    refY: "5",
    markerWidth: "8",
    markerHeight: "8",
    orient: "auto-start-reverse",
  });
  activeMarker.append(svgElement("path", { d: "M 0 0 L 10 5 L 0 10 z" }));
  definitions.append(marker, activeMarker);
  svg.append(definitions);

  const edgeLayer = svgElement("g", { class: "graph-edge-layer" });
  const activeRecords = activity?.records || null;
  for (const edge of graph.edges) {
    const source = layout.positions.get(edge.source.id);
    const destination = layout.positions.get(edge.target.id);
    if (!source || !destination) continue;
    const active = activeRecords
      && edge.last_envelope_record !== null
      && edge.last_envelope_record !== undefined
      && edge.last_envelope_record >= activeRecords.from
      && edge.last_envelope_record <= activeRecords.through;
    const group = svgElement("g", {
      class: `graph-edge${active ? " active" : ""}`,
      "data-route": edge.route,
    });
    const startX = source.x + source.width;
    const startY = source.y + source.height / 2;
    const endX = destination.x;
    const endY = destination.y + destination.height / 2;
    const bend = Math.max(56, (endX - startX) * 0.48);
    const path = svgElement("path", {
      d: `M ${startX} ${startY} C ${startX + bend} ${startY}, ${endX - bend} ${endY}, ${endX} ${endY}`,
      "marker-end": active ? "url(#graph-arrow-active)" : "url(#graph-arrow)",
    });
    const title = svgElement("title");
    title.textContent = `${edge.route} · ${edge.envelope_count || "0"} envelope(s)`;
    path.append(title);
    group.append(path);

    const midpointX = (startX + endX) / 2;
    const midpointY = (startY + endY) / 2;
    const badge = svgElement("g", {
      class: "edge-counter",
      transform: `translate(${midpointX - 27} ${midpointY - 13})`,
    });
    badge.append(svgElement("rect", { width: "54", height: "26", rx: "13" }));
    const counter = svgElement("text", { x: "27", y: "17", "text-anchor": "middle" });
    counter.textContent = `✉ ${edge.envelope_count || "0"}`;
    badge.append(counter);
    group.append(badge);

    const sourcePort = svgElement("text", {
      class: "edge-port source-port",
      x: startX + 8,
      y: startY - 9,
    });
    sourcePort.textContent = edge.source.port || "";
    const targetPort = svgElement("text", {
      class: "edge-port target-port",
      x: endX - 8,
      y: endY - 9,
      "text-anchor": "end",
    });
    targetPort.textContent = edge.target.port || "";
    group.append(sourcePort, targetPort);
    edgeLayer.append(group);
  }
  svg.append(edgeLayer);

  const nodeLayer = svgElement("g", { class: "graph-node-layer" });
  for (const node of graph.nodes) {
    const position = layout.positions.get(node.id);
    if (!position) continue;
    const active = activity?.kind === "transaction" && activity.node === node.path;
    const group = svgElement("g", {
      class: `graph-node graph-node-${node.kind}${active ? " active" : ""}`,
      transform: `translate(${position.x} ${position.y})`,
    });
    const title = svgElement("title");
    title.textContent = node.path
      ? `${node.path} · ${node.type}`
      : `${node.label} · ${node.kind}`;
    const body = svgElement("rect", {
      class: "graph-node-body",
      width: position.width,
      height: position.height,
      rx: node.kind === "operator" || node.kind === "connector" ? "12" : "8",
    });
    group.append(title, body);
    const icon = svgElement("text", { class: "graph-node-icon", x: "18", y: "29" });
    icon.textContent = node.kind === "operator" ? "○" : node.kind === "connector" ? "□" : "×";
    const name = svgElement("text", { class: "graph-node-name", x: "43", y: "29" });
    name.textContent = truncate(node.path || node.label || node.id, 24);
    const type = svgElement("text", { class: "graph-node-type", x: "18", y: "56" });
    type.textContent = truncate(node.type || node.kind, 29);
    group.append(icon, name, type);
    if (node.firing_count !== undefined) {
      const transactions = svgElement("text", {
        class: "graph-node-count",
        x: position.width - 14,
        y: position.height - 14,
        "text-anchor": "end",
      });
      transactions.textContent = `${node.firing_count} tx`;
      group.append(transactions);
    }
    if (active) {
      const ran = svgElement("g", {
        class: "node-ran",
        transform: `translate(${position.width - 83} -10)`,
      });
      ran.append(svgElement("rect", { width: "76", height: "22", rx: "11" }));
      const label = svgElement("text", { x: "38", y: "15", "text-anchor": "middle" });
      label.textContent = "JUST RAN";
      ran.append(label);
      group.append(ran);
    }
    nodeLayer.append(group);
  }
  svg.append(nodeLayer);
  canvas.append(svg);

  const legend = document.createElement("div");
  legend.className = "graph-legend";
  for (const [kind, label] of [["operator", "Operator"], ["connector", "Connector"], ["boundary", "Graph boundary"]]) {
    const item = document.createElement("span");
    item.className = `legend-${kind}`;
    item.textContent = label;
    legend.append(item);
  }
  const evidence = document.createElement("span");
  evidence.className = "legend-evidence";
  evidence.textContent = "✉ cumulative envelopes from causal records";
  legend.append(evidence);

  target.className = "topology";
  target.replaceChildren(activityPanel, ownerStrip, canvas, legend);
}

const SVG_NAMESPACE = "http://www.w3.org/2000/svg";

function svgElement(name, attributes = {}) {
  const element = document.createElementNS(SVG_NAMESPACE, name);
  for (const [key, value] of Object.entries(attributes)) {
    element.setAttribute(key, String(value));
  }
  return element;
}

function materializeGraph(topology) {
  const nodes = new Map((topology.nodes || []).map((node) => [node.id, { ...node }]));
  for (const edge of topology.edges || []) {
    for (const endpoint of [edge.source, edge.target]) {
      if (!endpoint || nodes.has(endpoint.id)) continue;
      nodes.set(endpoint.id, {
        id: endpoint.id,
        kind: endpoint.kind === "discard" ? "discard" : "boundary",
        label: endpoint.label || endpoint.port || "Boundary",
        path: null,
        type: endpoint.kind === "discard" ? "sink" : endpoint.kind,
      });
    }
  }
  return { nodes: [...nodes.values()], edges: topology.edges || [] };
}

function layoutGraph(nodes, edges) {
  const byId = new Map(nodes.map((node, index) => [node.id, { node, index }]));
  const incoming = new Map(nodes.map((node) => [node.id, 0]));
  const outgoing = new Map(nodes.map((node) => [node.id, []]));
  for (const edge of edges) {
    if (!byId.has(edge.source.id) || !byId.has(edge.target.id)) continue;
    outgoing.get(edge.source.id).push(edge.target.id);
    incoming.set(edge.target.id, (incoming.get(edge.target.id) || 0) + 1);
  }
  const rank = new Map(nodes.map((node) => [node.id, 0]));
  const queue = nodes
    .filter((node) => incoming.get(node.id) === 0)
    .map((node) => node.id);
  let cursor = 0;
  while (cursor < queue.length) {
    const source = queue[cursor++];
    for (const target of outgoing.get(source) || []) {
      rank.set(target, Math.max(rank.get(target) || 0, (rank.get(source) || 0) + 1));
      incoming.set(target, (incoming.get(target) || 0) - 1);
      if (incoming.get(target) === 0) queue.push(target);
    }
  }
  const groups = new Map();
  for (const node of nodes) {
    const level = rank.get(node.id) || 0;
    if (!groups.has(level)) groups.set(level, []);
    groups.get(level).push(node);
  }
  for (const group of groups.values()) {
    group.sort((left, right) => byId.get(left.id).index - byId.get(right.id).index);
  }
  const nodeWidth = 196;
  const nodeHeight = 78;
  const columnGap = 142;
  const rowGap = 56;
  const marginX = 38;
  const marginY = 44;
  const levels = Math.max(0, ...groups.keys()) + 1;
  const maximumRows = Math.max(1, ...[...groups.values()].map((group) => group.length));
  const height = marginY * 2 + maximumRows * nodeHeight + (maximumRows - 1) * rowGap;
  const width = marginX * 2 + levels * nodeWidth + (levels - 1) * columnGap;
  const positions = new Map();
  for (const [level, group] of groups) {
    const groupHeight = group.length * nodeHeight + Math.max(0, group.length - 1) * rowGap;
    const startY = (height - groupHeight) / 2;
    group.forEach((node, index) => {
      positions.set(node.id, {
        x: marginX + level * (nodeWidth + columnGap),
        y: startY + index * (nodeHeight + rowGap),
        width: nodeWidth,
        height: nodeHeight,
      });
    });
  }
  return { width, height, positions };
}

function truncate(value, maximum) {
  const text = String(value || "");
  return text.length <= maximum ? text : `${text.slice(0, maximum - 1)}…`;
}

function renderHistory(history) {
  const target = document.querySelector("#history");
  const records = history?.records || [];
  const deliveries = history?.deliveries || [];
  text("history-summary", `${records.length} causal record${records.length === 1 ? "" : "s"}; ${deliveries.length} observed deliver${deliveries.length === 1 ? "y" : "ies"}.`);
  if (!records.length && !deliveries.length) {
    empty(target, "No committed records.");
    document.querySelector("#show-older").hidden = true;
    return;
  }
  const start = Math.max(0, records.length - historyLimit);
  const fragment = document.createDocumentFragment();
  fragment.append(historyTable("Causal records", records.slice(start), ["record_index", "event", "owner", "connector", "origin_id"]));
  fragment.append(historyTable("Observed deliveries", deliveries.slice(Math.max(0, deliveries.length - historyLimit)), ["index", "node", "port", "envelope_id", "payload"]));
  target.className = "history-stack";
  target.replaceChildren(fragment);
  const older = document.querySelector("#show-older");
  older.hidden = start === 0;
  older.textContent = `Show ${Math.min(100, start)} older records`;
}

function historyTable(label, rows, columns) {
  const details = document.createElement("details");
  details.open = true;
  const summary = document.createElement("summary");
  summary.textContent = `${label} (${rows.length} shown)`;
  const table = document.createElement("table");
  const head = document.createElement("thead");
  const header = document.createElement("tr");
  for (const column of columns) {
    const cell = document.createElement("th");
    cell.scope = "col";
    cell.textContent = column.replaceAll("_", " ");
    header.append(cell);
  }
  head.append(header);
  const body = document.createElement("tbody");
  for (const row of rows) {
    const line = document.createElement("tr");
    for (const column of columns) {
      const cell = document.createElement("td");
      appendValue(cell, row[column]);
      line.append(cell);
    }
    body.append(line);
  }
  table.append(head, body);
  details.append(summary, table);
  return details;
}

function renderInstances(instances) {
  const target = document.querySelector("#state");
  if (!(instances || []).length) {
    empty(target, "No active instance.");
    return;
  }
  target.className = "card-stack";
  target.replaceChildren(...instances.map((instance) => {
    const card = document.createElement("article");
    card.className = "data-card";
    const heading = document.createElement("h3");
    heading.textContent = instance.owner || "app";
    const meta = document.createElement("p");
    meta.className = "card-meta";
    meta.textContent = `${instance.phase} · generation ${instance.reference?.generation ?? "?"}`;
    card.append(heading, meta);
    appendValue(card, instance.state);
    return card;
  }));
}

function renderExchanges(exchanges) {
  const target = document.querySelector("#exchanges");
  const calls = exchanges?.calls || [];
  const sessions = exchanges?.sessions || [];
  if (!calls.length && !sessions.length) {
    empty(target, "No open exchange.");
    return;
  }
  target.className = "card-stack";
  const rows = [
    ...calls.map((call) => ({ kind: "Call", ...call })),
    ...sessions.map((session) => ({ kind: "Session", ...session })),
  ];
  target.replaceChildren(...rows.map((exchange) => {
    const card = document.createElement("article");
    card.className = "data-card";
    const heading = document.createElement("h3");
    heading.textContent = `${exchange.kind} · ${exchange.connector}`;
    const meta = document.createElement("p");
    meta.className = "card-meta";
    meta.textContent = `${exchange.direction}; ${exchange.completion_retained ? "terminal outcome retained" : "open"}`;
    card.append(heading, meta);
    return card;
  }));
}

function renderCost(report) {
  if (report?.error) {
    renderGeneric("cost", report, "No measured cost.");
    return;
  }
  renderGeneric("cost", {
    measures: report?.measures || [],
    costs: report?.costs || [],
    unpriced: report?.unpriced || [],
  }, "No measured cost.");
}

function renderGeneric(id, value, message) {
  const target = document.querySelector(`#${id}`);
  if (value === undefined || value === null || (Array.isArray(value) && value.length === 0)) {
    empty(target, message);
    return;
  }
  target.className = "structured-value";
  target.replaceChildren();
  appendValue(target, value);
}

function appendValue(target, value) {
  if (value !== null && typeof value === "object") {
    const details = document.createElement("details");
    details.open = true;
    const summary = document.createElement("summary");
    const count = Array.isArray(value) ? value.length : Object.keys(value).length;
    summary.textContent = Array.isArray(value)
      ? `${count} item${count === 1 ? "" : "s"}`
      : `${count} field${count === 1 ? "" : "s"}`;
    const pre = document.createElement("pre");
    pre.textContent = JSON.stringify(value, null, 2);
    details.append(summary, pre);
    target.append(details);
    return;
  }
  const code = document.createElement("code");
  code.textContent = value === undefined ? "—" : String(value);
  target.append(code);
}

function empty(target, message) {
  target.className = "empty";
  target.textContent = message;
}

function text(id, value) {
  document.querySelector(`#${id}`).textContent = value;
}

function formatNanoseconds(value) {
  return value === undefined || value === null ? "—" : `${value} ns`;
}

for (const button of document.querySelectorAll("button[data-command]")) {
  button.addEventListener("click", async () => {
    try {
      await post(`/api/command/${button.dataset.command}`);
    } catch (error) {
      text("guidance", `UI command refused: ${error.message}`);
    }
  });
}

document.querySelector("#take-control").addEventListener("click", () => claim(true));
document.querySelector("#show-older").addEventListener("click", () => {
  historyLimit += 100;
  renderHistory(currentSnapshot?.history);
});
document.querySelector("#scenario-select").addEventListener("change", updateControls);
document.querySelector("#select-scenario").addEventListener("click", async () => {
  const [simulation, scenario] = document.querySelector("#scenario-select").value.split("\u0000");
  try {
    await post(`/api/command/select-scenario?simulation=${encodeURIComponent(simulation)}&scenario=${encodeURIComponent(scenario)}`);
  } catch (error) {
    text("guidance", `Scenario selection refused: ${error.message}`);
  }
});

const events = new EventSource("/api/events");
events.addEventListener("revision", () => refresh().catch((error) => text("guidance", error.message)));
events.onerror = () => {
  text("role", "reconnecting");
  controls = false;
  updateControls();
};

claim().then(refresh).catch((error) => text("guidance", error.message));
