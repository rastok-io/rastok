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
      ? "Paused at a semantic boundary. Scenario actions and VM transactions are separate controls."
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
  const nodes = document.createElement("div");
  nodes.className = "topology-nodes";
  for (const node of topology.nodes) {
    const card = document.createElement("article");
    card.className = "node-card";
    const type = document.createElement("span");
    type.className = "node-type";
    type.textContent = `${node.kind} · ${node.type}`;
    const name = document.createElement("strong");
    name.textContent = node.path || "app";
    card.append(name, type);
    nodes.append(card);
  }
  const edges = document.createElement("ol");
  edges.className = "edge-list";
  for (const edge of topology.edges || []) {
    const item = document.createElement("li");
    const route = document.createElement("code");
    route.textContent = edge.route;
    const scope = document.createElement("span");
    scope.textContent = `scope ${edge.scope || "app"}`;
    item.append(route, scope);
    edges.append(item);
  }
  target.className = "topology";
  target.replaceChildren(nodes, edges);
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
