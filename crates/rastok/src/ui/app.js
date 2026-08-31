"use strict";

const controller = sessionStorage.getItem("rastok-controller") || crypto.randomUUID();
sessionStorage.setItem("rastok-controller", controller);
history.replaceState(null, "", location.pathname);

let controls = false;
let currentSnapshot = null;
let historyLimit = 100;
let selectedTopologyItem = null;

async function post(path, body = null) {
  const headers = { "X-Rastok-Controller": controller };
  if (body !== null) headers["Content-Type"] = "application/json";
  const response = await fetch(path, {
    method: "POST",
    headers,
    body: body === null ? undefined : body,
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
  text("execution-state", state);
  document.querySelector("#execution-state").className = `pill state-${state}`;
  text("mode", snapshot.mode === "simulate" ? "SIMULATION" : (snapshot.mode || "—").toUpperCase());
  text("logical-time", formatNanoseconds(execution.logical_time_ns));
  text("awaiting-calls", String(execution.awaiting_outbound_calls || 0));

  const simulation = snapshot.mode === "simulate";
  for (const element of document.querySelectorAll(".simulation-only")) {
    element.hidden = !simulation;
  }
  const scenario = snapshot.scenario?.current;
  const stepLabel = simulation && scenario && !scenario.complete && scenario.quiescent
    ? "Step to next scenario action"
    : "Step one atomic transaction";
  document.querySelector("#step").title = stepLabel;
  document.querySelector("#step").setAttribute("aria-label", stepLabel);
  renderFault(snapshot.fault);
  renderScenario(snapshot.scenario, simulation);
  renderTopology(snapshot.topology);
  renderHistory(snapshot.history);
  renderInstances(snapshot.instances);
  renderExchanges(snapshot.exchanges);
  renderCost(snapshot.cost_report);
  updateControls();
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
  text("fault-title", fault.commit?.includes("draft") ? "Input edit refused" : "Execution fault");
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

  const graph = materializeGraph(topology);
  const layout = layoutGraph(graph.nodes, graph.edges, topology.owners || []);
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

  const scopeLayer = svgElement("g", { class: "graph-scope-layer" });
  const orderedOwners = [...(topology.owners || [])]
    .sort((left, right) => (layout.scopes.get(left.id)?.depth || 0) - (layout.scopes.get(right.id)?.depth || 0));
  for (const owner of orderedOwners) {
    const position = layout.scopes.get(owner.id);
    if (!position) continue;
    const active = activity?.kind === "transaction" && activity.node === owner.path;
    const selectable = Boolean(owner.definition);
    const selected = selectedTopologyItem === owner.id;
    const group = svgElement("g", {
      class: `graph-scope${position.depth === 0 ? " root" : " nested"}${active ? " active" : ""}${selectable ? " selectable" : ""}${selected ? " selected" : ""}`,
    });
    if (selectable) {
      group.setAttribute("role", "button");
      group.setAttribute("tabindex", "0");
      group.setAttribute("aria-pressed", String(selected));
      group.setAttribute("aria-label", `Inspect ${owner.path || "App"} Graph definition`);
      group.addEventListener("click", () => selectTopologyItem(owner.id));
      group.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          selectTopologyItem(owner.id);
        }
      });
    }
    const title = svgElement("title");
    title.textContent = `${owner.label || "App"} · ${itemDescriptor(owner)}`;
    const body = svgElement("rect", {
      class: "graph-scope-body",
      x: position.x,
      y: position.y,
      width: position.width,
      height: position.height,
      rx: position.depth === 0 ? "16" : "12",
    });
    const header = svgElement("line", {
      class: "graph-scope-header-line",
      x1: position.x,
      y1: position.y + 34,
      x2: position.x + position.width,
      y2: position.y + 34,
    });
    const name = svgElement("text", {
      class: "graph-scope-name",
      x: position.x + 14,
      y: position.y + 22,
    });
    name.textContent = truncate(owner.label || "App", 28);
    const type = svgElement("text", {
      class: "graph-scope-type",
      x: position.x + position.width - 14,
      y: position.y + 22,
      "text-anchor": "end",
    });
    type.textContent = truncate(itemDescriptor(owner), 34);
    group.append(title, body, header, name, type);
    if (position.depth > 0) {
      for (const port of owner.ports || []) {
        const anchor = scopePortAnchor(owner, position, port);
        const disk = svgElement("circle", {
          class: `graph-port graph-port-${port.direction}`,
          cx: anchor.x,
          cy: anchor.y,
          r: "5.5",
        });
        const label = svgElement("text", {
          class: "graph-scope-port-label",
          x: anchor.x + (port.direction === "input" ? 9 : -9),
          y: anchor.y + 3.5,
          "text-anchor": port.direction === "input" ? "start" : "end",
        });
        label.textContent = port.name;
        group.append(disk, label);
      }
    }
    scopeLayer.append(group);
  }
  svg.append(scopeLayer);

  const edgeLayer = svgElement("g", { class: "graph-edge-layer" });
  const activeRecords = activity?.records || null;
  for (const edge of graph.edges) {
    const source = graph.byId.get(edge.source.id);
    const destination = graph.byId.get(edge.target.id);
    const sourcePosition = layout.positions.get(edge.source.id);
    const destinationPosition = layout.positions.get(edge.target.id);
    if (!source || !destination || !sourcePosition || !destinationPosition) continue;
    const active = activeRecords
      && edge.last_envelope_record !== null
      && edge.last_envelope_record !== undefined
      && edge.last_envelope_record >= activeRecords.from
      && edge.last_envelope_record <= activeRecords.through;
    const group = svgElement("g", {
      class: `graph-edge${active ? " active" : ""}`,
      "data-route": edge.route,
    });
    const sourcePort = portAnchor(source, sourcePosition, edge.source.port, "output");
    const targetPort = portAnchor(destination, destinationPosition, edge.target.port, "input");
    const startX = sourcePort.x + 7;
    const startY = sourcePort.y;
    const endX = targetPort.x - 7;
    const endY = targetPort.y;
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

    const sourcePortLabel = svgElement("text", {
      class: "edge-port source-port",
      x: startX + 8,
      y: startY - 9,
    });
    sourcePortLabel.textContent = edge.source.port || "";
    const targetPortLabel = svgElement("text", {
      class: "edge-port target-port",
      x: endX - 8,
      y: endY - 9,
      "text-anchor": "end",
    });
    targetPortLabel.textContent = edge.target.port || "";
    group.append(sourcePortLabel, targetPortLabel);
    edgeLayer.append(group);
  }
  svg.append(edgeLayer);

  const nodeLayer = svgElement("g", { class: "graph-node-layer" });
  for (const node of graph.nodes) {
    const position = layout.positions.get(node.id);
    if (!position) continue;
    const active = activity?.kind === "transaction" && activity.node === node.path;
    const selectable = Boolean(node.definition);
    const selected = selectedTopologyItem === node.id;
    const boundary = node.kind === "boundary" || node.kind === "discard";
    const group = svgElement("g", {
      class: `graph-node graph-node-${node.kind}${active ? " active" : ""}${selectable ? " selectable" : ""}${selected ? " selected" : ""}`,
      transform: `translate(${position.x} ${position.y})`,
    });
    if (selectable) {
      group.setAttribute("role", "button");
      group.setAttribute("tabindex", "0");
      group.setAttribute("aria-pressed", String(selected));
      group.setAttribute("aria-label", `Inspect ${node.path} definition`);
      group.addEventListener("click", () => selectTopologyItem(node.id));
      group.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          selectTopologyItem(node.id);
        }
      });
    }
    const title = svgElement("title");
    title.textContent = node.path
      ? `${node.path} · ${itemDescriptor(node)}`
      : `${node.label} · ${node.kind}`;
    group.append(title);
    if (boundary) {
      const direction = node.direction || (node.kind === "discard" ? "discard" : "input");
      const diskX = direction === "input" ? position.width - 10 : 10;
      const disk = svgElement("circle", {
        class: `graph-port graph-port-${direction}`,
        cx: diskX,
        cy: position.height / 2,
        r: "6",
      });
      const label = svgElement("text", {
        class: "graph-boundary-label",
        x: direction === "input" ? diskX + 12 : diskX - 12,
        y: position.height / 2 + 4,
        "text-anchor": direction === "input" ? "start" : "end",
      });
      label.textContent = truncate(node.label || node.ports?.[0]?.name || "boundary", 16);
      group.append(disk, label);
      nodeLayer.append(group);
      continue;
    }
    const body = svgElement("rect", {
      class: "graph-node-body",
      width: position.width,
      height: position.height,
      rx: "10",
    });
    const name = svgElement("text", {
      class: "graph-node-name",
      x: position.width / 2,
      y: "22",
      "text-anchor": "middle",
    });
    name.textContent = truncate(node.label || node.path || node.id, 22);
    const type = svgElement("text", {
      class: "graph-node-type",
      x: position.width / 2,
      y: "41",
      "text-anchor": "middle",
    });
    type.textContent = truncate(itemDescriptor(node), 27);
    group.append(body, name, type);
    for (const port of node.ports || []) {
      const anchor = portAnchor(node, { x: 0, y: 0, ...position }, port.name, port.direction);
      const disk = svgElement("circle", {
        class: `graph-port graph-port-${port.direction}`,
        cx: anchor.x - position.x,
        cy: anchor.y - position.y,
        r: "5.5",
      });
      const portTitle = svgElement("title");
      portTitle.textContent = `${port.direction} port ${port.name}`;
      disk.append(portTitle);
      group.append(disk);
    }
    nodeLayer.append(group);
  }
  svg.append(nodeLayer);
  canvas.append(svg);

  const selectableItems = [
    ...(topology.owners || []).map((owner) => ({ ...owner, kind: "graph" })),
    ...graph.nodes.filter((node) => node.definition),
  ];
  let selected = selectableItems.find((item) => item.id === selectedTopologyItem) || null;
  if (selectedTopologyItem && !selected) {
    selectedTopologyItem = null;
    selected = null;
  }
  const workbench = document.createElement("div");
  workbench.className = "graph-workbench";
  const inspectors = document.createElement("div");
  inspectors.className = "inspection-stack";
  inspectors.append(envelopePanel(currentSnapshot), definitionPanel(selected));
  workbench.append(canvas, inspectors);

  const legend = document.createElement("div");
  legend.className = "graph-legend";
  for (const [kind, label] of [["operator", "Operator"], ["connector", "Connector"], ["scope", "Graph scope"], ["input", "Input"], ["output", "Output"]]) {
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
  target.replaceChildren(activityPanel, workbench, legend);
}

function selectTopologyItem(id) {
  selectedTopologyItem = selectedTopologyItem === id ? null : id;
  renderTopology(currentSnapshot?.topology);
}

function envelopePanel(snapshot) {
  const panel = document.createElement("aside");
  panel.className = "envelope-inspector";
  panel.setAttribute("aria-label", "Current envelope boundary");
  const draft = snapshot?.scenario?.current?.draft;
  const deliveries = snapshot?.history?.deliveries || [];
  const delivery = deliveries.at(-1) || null;
  const heading = document.createElement("div");
  heading.className = "envelope-heading";
  const identity = document.createElement("div");
  const position = document.createElement("p");
  position.className = "envelope-position";
  const title = document.createElement("h3");
  identity.append(position, title);
  heading.append(identity);
  panel.append(heading);

  if (draft?.position === "before-app-input") {
    position.textContent = "Before admission";
    title.textContent = `App.${draft.port}`;
    const badges = document.createElement("div");
    badges.className = "envelope-badges";
    badges.append(typeBadge(draft.type?.label || "unknown"));
    if (draft.edited) {
      const edited = document.createElement("span");
      edited.className = "edited-badge";
      edited.textContent = "edited draft";
      badges.append(edited);
    }
    panel.append(badges);
    const zones = envelopeZones(draft.meta, null);
    const dataZone = zones.querySelector("[data-zone='data']");
    if (draft.editable) {
      let editorValue = cloneJson(draft.data);
      const tree = typedNode(draft.type, editorValue, "DATA", (next) => { editorValue = next; });
      tree.classList.add("typed-tree");
      dataZone.append(tree);
      const actions = document.createElement("div");
      actions.className = "envelope-actions";
      const save = document.createElement("button");
      save.type = "button";
      save.textContent = "Apply DATA draft";
      save.disabled = !controls || !(snapshot.execution?.legal_commands || []).includes("edit-next-injection");
      save.addEventListener("click", async () => {
        try {
          await post("/api/command/edit-next-injection", JSON.stringify(editorValue));
          setCommandStatus("DATA draft accepted for validation.");
        } catch (error) {
          setCommandStatus(`DATA edit refused: ${error.message}`);
        }
      });
      actions.append(save);
      dataZone.append(actions);
    } else {
      appendValue(dataZone, draft.data);
    }
    panel.append(zones);
    return panel;
  }

  if (delivery) {
    position.textContent = "Latest envelope boundary";
    title.textContent = `${delivery.node || "App"}.${delivery.port || "?"}`;
    const badges = document.createElement("div");
    badges.className = "envelope-badges";
    for (const label of [delivery.envelope_id, delivery.origin_id && `origin ${delivery.origin_id}`]) {
      if (!label) continue;
      const badge = document.createElement("span");
      badge.textContent = label;
      badges.append(badge);
    }
    panel.append(badges, envelopeZones(delivery.meta, delivery.data));
    return panel;
  }

  position.textContent = "Envelope";
  title.textContent = "No boundary selected";
  return panel;
}

function envelopeZones(meta, data) {
  const zones = document.createElement("div");
  zones.className = "envelope-zones";
  for (const [name, value] of [["META", meta], ["DATA", data]]) {
    const zone = document.createElement("div");
    zone.className = "envelope-zone";
    zone.dataset.zone = name.toLowerCase();
    const title = document.createElement("h4");
    title.textContent = name;
    zone.append(title);
    if (value !== null && value !== undefined) appendValue(zone, value);
    zones.append(zone);
  }
  return zones;
}

function typedNode(schema, value, name, setValue) {
  const kind = schema?.kind || "variant";
  if (kind === "struct") {
    const fieldset = typedFieldset(name, schema.label);
    const children = document.createElement("div");
    children.className = "typed-items";
    for (const field of schema.fields || []) {
      children.append(typedNode(field.type, value?.[field.name], field.name, (next) => {
        value[field.name] = next;
        setValue(value);
      }));
    }
    fieldset.append(children);
    return fieldset;
  }
  if (kind === "array") {
    const fieldset = typedFieldset(name, schema.label);
    const items = document.createElement("div");
    items.className = "typed-items";
    const renderItems = () => {
      items.replaceChildren(...value.map((item, index) => {
        const row = document.createElement("div");
        row.className = "typed-item";
        row.append(typedNode(schema.element, item, `[${index}]`, (next) => {
          value[index] = next;
          setValue(value);
        }));
        if (schema.length === null || schema.length === undefined) {
          const remove = tinyButton("−", `Remove item ${index}`);
          remove.addEventListener("click", () => {
            value.splice(index, 1);
            setValue(value);
            renderItems();
          });
          row.append(remove);
        }
        return row;
      }));
    };
    renderItems();
    fieldset.append(items);
    if (schema.length === null || schema.length === undefined) {
      const add = tinyButton("+", "Add item");
      add.addEventListener("click", () => {
        value.push(defaultValue(schema.element));
        setValue(value);
        renderItems();
      });
      fieldset.append(add);
    }
    return fieldset;
  }
  if (kind === "map") {
    const fieldset = typedFieldset(name, schema.label);
    const items = document.createElement("div");
    items.className = "typed-items";
    const renderItems = () => {
      items.replaceChildren(...value.map((entry, index) => {
        const row = document.createElement("div");
        row.className = "typed-item";
        const pair = typedFieldset(`entry ${index}`, "key → value");
        pair.append(
          typedNode(schema.key, entry.key, "key", (next) => { entry.key = next; setValue(value); }),
          typedNode(schema.value, entry.value, "value", (next) => { entry.value = next; setValue(value); }),
        );
        const remove = tinyButton("−", `Remove entry ${index}`);
        remove.addEventListener("click", () => {
          value.splice(index, 1);
          setValue(value);
          renderItems();
        });
        row.append(pair, remove);
        return row;
      }));
    };
    renderItems();
    const add = tinyButton("+", "Add map entry");
    add.addEventListener("click", () => {
      value.push({ key: defaultValue(schema.key), value: defaultValue(schema.value) });
      setValue(value);
      renderItems();
    });
    fieldset.append(items, add);
    return fieldset;
  }
  if (kind === "structural-union") {
    const fieldset = typedFieldset(name, schema.label);
    const select = document.createElement("select");
    for (const [index, branch] of (schema.branches || []).entries()) {
      const option = document.createElement("option");
      option.value = String(index);
      option.textContent = branch.label;
      option.selected = Number(value?.branch || 0) === index;
      select.append(option);
    }
    const body = document.createElement("div");
    const renderBranch = () => {
      const index = Number(select.value);
      body.replaceChildren(typedNode(schema.branches[index], value.value, "value", (next) => {
        value.value = next;
        setValue(value);
      }));
    };
    select.addEventListener("change", () => {
      value = { branch: Number(select.value), value: defaultValue(schema.branches[Number(select.value)]) };
      setValue(value);
      renderBranch();
    });
    renderBranch();
    fieldset.append(select, body);
    return fieldset;
  }
  if (kind === "tagged-union") {
    const fieldset = typedFieldset(name, schema.label);
    const select = document.createElement("select");
    for (const item of schema.cases || []) {
      const option = document.createElement("option");
      option.value = item.name;
      option.textContent = item.name;
      option.selected = value?.case === item.name;
      select.append(option);
    }
    const body = document.createElement("div");
    const renderCase = () => {
      const item = (schema.cases || []).find((candidate) => candidate.name === select.value);
      body.replaceChildren(typedNode(item.type, value.value, item.name, (next) => {
        value.value = next;
        setValue(value);
      }));
    };
    select.addEventListener("change", () => {
      const item = (schema.cases || []).find((candidate) => candidate.name === select.value);
      value = { case: item.name, value: defaultValue(item.type) };
      setValue(value);
      renderCase();
    });
    renderCase();
    fieldset.append(select, body);
    return fieldset;
  }
  if (kind === "temporal" && value !== null && typeof value === "object") {
    const fieldset = typedFieldset(name, schema.label);
    for (const field of ["months", "days", "nanoseconds"]) {
      fieldset.append(typedLeaf({ kind: "integer", label: "int64" }, value[field], field, (next) => {
        value[field] = next;
        setValue(value);
      }));
    }
    return fieldset;
  }
  return typedLeaf(schema, value, name, setValue);
}

function typedLeaf(schema, value, name, setValue) {
  const row = document.createElement("label");
  row.className = "typed-leaf";
  const label = document.createElement("span");
  label.className = "typed-leaf-label";
  label.append(document.createTextNode(name), typeBadge(schema?.label || schema?.kind || "unknown"));
  let input;
  if (schema?.kind === "bool") {
    input = document.createElement("input");
    input.type = "checkbox";
    input.checked = Boolean(value);
    input.addEventListener("change", () => setValue(input.checked));
  } else if (schema?.kind === "enum") {
    input = document.createElement("select");
    for (const symbol of schema.symbols || []) {
      const option = document.createElement("option");
      option.value = symbol;
      option.textContent = symbol;
      option.selected = symbol === value;
      input.append(option);
    }
    input.addEventListener("change", () => setValue(input.value));
  } else {
    input = document.createElement("input");
    input.type = "text";
    input.value = schema?.kind === "null" ? "null" : String(value ?? "");
    input.disabled = schema?.kind === "null" || schema?.kind === "variant";
    input.addEventListener("input", () => setValue(input.value));
  }
  row.append(label, input);
  return row;
}

function typedFieldset(name, type) {
  const fieldset = document.createElement("fieldset");
  fieldset.className = "typed-fieldset";
  const legend = document.createElement("legend");
  legend.append(document.createTextNode(name), typeBadge(type));
  fieldset.append(legend);
  return fieldset;
}

function typeBadge(label) {
  const badge = document.createElement("span");
  badge.className = "type-badge";
  badge.textContent = label || "unknown";
  return badge;
}

function tinyButton(label, accessible) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "tiny-button";
  button.textContent = label;
  button.setAttribute("aria-label", accessible);
  button.title = accessible;
  return button;
}

function defaultValue(schema) {
  switch (schema?.kind) {
    case "null": return null;
    case "bool": return false;
    case "integer": case "float": case "decimal": case "number": return "0";
    case "temporal": return String(schema.family || "").startsWith("Interval")
      ? { months: "0", days: "0", nanoseconds: "0" }
      : "0";
    case "text": case "media-type": case "bytes": case "uuid": return "";
    case "enum": return schema.symbols?.[0] || "";
    case "struct": return Object.fromEntries((schema.fields || []).map((field) => [field.name, defaultValue(field.type)]));
    case "array": return Array.from({ length: schema.length || 0 }, () => defaultValue(schema.element));
    case "map": return [];
    case "structural-union": return { branch: 0, value: defaultValue(schema.branches?.[0]) };
    case "tagged-union": return { case: schema.cases?.[0]?.name || "", value: defaultValue(schema.cases?.[0]?.type) };
    default: return null;
  }
}

function cloneJson(value) {
  return JSON.parse(JSON.stringify(value));
}

function definitionPanel(item) {
  const panel = document.createElement("aside");
  panel.className = `node-definition${item ? " populated" : " empty"}`;
  panel.setAttribute("aria-label", "Selected Node definition");
  if (!item?.definition) {
    const title = document.createElement("h3");
    title.textContent = "Node definition";
    panel.append(title);
    return panel;
  }

  const heading = document.createElement("div");
  heading.className = "node-definition-heading";
  const identity = document.createElement("div");
  const eyebrow = document.createElement("p");
  eyebrow.className = "node-definition-kind";
  eyebrow.textContent = item.kind || "node";
  const title = document.createElement("h3");
  title.textContent = item.path || item.label || "App";
  identity.append(eyebrow, title);
  const close = document.createElement("button");
  close.type = "button";
  close.className = "node-definition-close secondary";
  close.textContent = "Close";
  close.setAttribute("aria-label", "Close Node definition");
  close.addEventListener("click", () => selectTopologyItem(item.id));
  heading.append(identity, close);

  const badges = document.createElement("div");
  badges.className = "node-definition-badges";
  for (const label of [item.type, ...(item.aspects || []).map((aspect) => `+ ${aspect}`)]) {
    if (!label) continue;
    const badge = document.createElement("span");
    badge.textContent = label;
    badges.append(badge);
  }
  const location = document.createElement("p");
  location.className = "node-definition-location";
  const lineLabel = item.definition.first_line === item.definition.last_line
    ? `line ${item.definition.first_line}`
    : `lines ${item.definition.first_line}–${item.definition.last_line}`;
  location.textContent = `Source ${Number(item.definition.source_index) + 1} · ${lineLabel}`;
  const pre = document.createElement("pre");
  pre.className = "node-definition-code";
  const code = document.createElement("code");
  code.textContent = item.definition.code;
  pre.append(code);
  panel.append(heading, badges, location, pre);
  return panel;
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
  const rootOwner = (topology.owners || []).find((owner) => owner.parent === null)?.id || "owner:0";
  const nodes = new Map((topology.nodes || []).map((node) => [node.id, {
    ...node,
    owner: node.owner || rootOwner,
    ports: node.ports || [],
  }]));
  for (const edge of topology.edges || []) {
    for (const endpoint of [edge.source, edge.target]) {
      if (!endpoint || nodes.has(endpoint.id)) continue;
      const direction = endpoint.kind === "output"
        ? "output"
        : endpoint.kind === "discard"
          ? "discard"
          : "input";
      nodes.set(endpoint.id, {
        id: endpoint.id,
        kind: endpoint.kind === "discard" ? "discard" : "boundary",
        label: endpoint.port || endpoint.label || "Boundary",
        path: null,
        type: endpoint.kind === "discard" ? "sink" : endpoint.kind,
        direction,
        owner: endpoint.owner || rootOwner,
        ports: endpoint.port ? [{ name: endpoint.port, direction }] : [],
      });
    }
  }
  return { nodes: [...nodes.values()], byId: nodes, edges: topology.edges || [] };
}

function layoutGraph(nodes, edges, owners) {
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

  const dimensions = new Map(nodes.map((node) => {
    if (node.kind === "boundary" || node.kind === "discard") {
      return [node.id, { width: 104, height: 34 }];
    }
    const inputs = (node.ports || []).filter((port) => port.direction === "input").length;
    const outputs = (node.ports || []).filter((port) => port.direction === "output").length;
    return [node.id, {
      width: 150,
      height: Math.max(50, (Math.max(inputs, outputs) + 1) * 13),
    }];
  }));
  const columnGap = 104;
  const rankWidths = new Map();
  for (const [level, group] of groups) {
    rankWidths.set(level, Math.max(...group.map((node) => dimensions.get(node.id).width)));
  }
  const rankX = new Map();
  let nextX = 72;
  const maximumRank = Math.max(0, ...groups.keys());
  for (let level = 0; level <= maximumRank; level += 1) {
    rankX.set(level, nextX);
    nextX += (rankWidths.get(level) || 150) + columnGap;
  }

  const ownerMap = new Map(owners.map((owner, index) => [owner.id, { ...owner, index }]));
  const rootOwner = owners.find((owner) => owner.parent === null) || owners[0] || {
    id: "owner:0",
    parent: null,
    label: "App",
  };
  if (!ownerMap.has(rootOwner.id)) ownerMap.set(rootOwner.id, rootOwner);
  const children = new Map([...ownerMap.keys()].map((id) => [id, []]));
  for (const owner of ownerMap.values()) {
    if (owner.parent && children.has(owner.parent)) children.get(owner.parent).push(owner.id);
  }
  for (const childIds of children.values()) {
    childIds.sort((left, right) => (ownerMap.get(left)?.index || 0) - (ownerMap.get(right)?.index || 0));
  }
  const directNodes = new Map([...ownerMap.keys()].map((id) => [id, []]));
  for (const node of nodes) {
    const owner = directNodes.has(node.owner) ? node.owner : rootOwner.id;
    directNodes.get(owner).push(node);
  }
  for (const ownerNodes of directNodes.values()) {
    ownerNodes.sort((left, right) => byId.get(left.id).index - byId.get(right.id).index);
  }

  const rowGap = 24;
  const scopeGap = 20;
  const scopeHeader = 34;
  const scopePadding = 20;
  const measured = new Map();
  const directHeight = new Map();
  function measureOwner(ownerId) {
    if (measured.has(ownerId)) return measured.get(ownerId);
    const ownerGroups = new Map();
    for (const node of directNodes.get(ownerId) || []) {
      const level = rank.get(node.id) || 0;
      if (!ownerGroups.has(level)) ownerGroups.set(level, []);
      ownerGroups.get(level).push(node);
    }
    const bandHeight = Math.max(0, ...[...ownerGroups.values()].map((group) =>
      group.reduce((sum, node) => sum + dimensions.get(node.id).height, 0)
        + Math.max(0, group.length - 1) * rowGap));
    directHeight.set(ownerId, bandHeight);
    const childIds = children.get(ownerId) || [];
    const childHeight = childIds.reduce((sum, childId) => sum + measureOwner(childId), 0)
      + Math.max(0, childIds.length - 1) * scopeGap;
    const contentGap = bandHeight > 0 && childHeight > 0 ? scopeGap : 0;
    const height = scopeHeader + scopePadding * 2 + Math.max(36, bandHeight + contentGap + childHeight);
    measured.set(ownerId, height);
    return height;
  }
  measureOwner(rootOwner.id);

  const positions = new Map();
  const scopeVertical = new Map();
  function placeOwner(ownerId, top, depth) {
    const height = measured.get(ownerId);
    scopeVertical.set(ownerId, { y: top, height, depth });
    let cursorY = top + scopeHeader + scopePadding;
    const bandHeight = directHeight.get(ownerId) || 0;
    const ownerGroups = new Map();
    for (const node of directNodes.get(ownerId) || []) {
      const level = rank.get(node.id) || 0;
      if (!ownerGroups.has(level)) ownerGroups.set(level, []);
      ownerGroups.get(level).push(node);
    }
    for (const [level, group] of ownerGroups) {
      const groupHeight = group.reduce((sum, node) => sum + dimensions.get(node.id).height, 0)
        + Math.max(0, group.length - 1) * rowGap;
      let nodeY = cursorY + (bandHeight - groupHeight) / 2;
      for (const node of group) {
        const size = dimensions.get(node.id);
        positions.set(node.id, {
          x: rankX.get(level),
          y: nodeY,
          width: size.width,
          height: size.height,
        });
        nodeY += size.height + rowGap;
      }
    }
    cursorY += bandHeight;
    const childIds = children.get(ownerId) || [];
    if (bandHeight > 0 && childIds.length > 0) cursorY += scopeGap;
    for (const childId of childIds) {
      placeOwner(childId, cursorY, depth + 1);
      cursorY += measured.get(childId) + scopeGap;
    }
  }
  placeOwner(rootOwner.id, 18, 0);

  const scopes = new Map();
  function placeHorizontalScope(ownerId) {
    const components = [];
    for (const node of directNodes.get(ownerId) || []) {
      const position = positions.get(node.id);
      if (position) components.push({ left: position.x, right: position.x + position.width });
    }
    for (const childId of children.get(ownerId) || []) {
      placeHorizontalScope(childId);
      const child = scopes.get(childId);
      components.push({ left: child.x, right: child.x + child.width });
    }
    let left = components.length ? Math.min(...components.map((component) => component.left)) - 26 : 48;
    let right = components.length ? Math.max(...components.map((component) => component.right)) + 26 : left + 220;
    if (right - left < 220) {
      const extension = (220 - (right - left)) / 2;
      left -= extension;
      right += extension;
    }
    const vertical = scopeVertical.get(ownerId);
    scopes.set(ownerId, {
      x: left,
      y: vertical.y,
      width: right - left,
      height: vertical.height,
      depth: vertical.depth,
    });
  }
  placeHorizontalScope(rootOwner.id);
  const rootScope = scopes.get(rootOwner.id);
  const shiftX = rootScope.x < 18 ? 18 - rootScope.x : 0;
  if (shiftX > 0) {
    for (const position of positions.values()) position.x += shiftX;
    for (const scope of scopes.values()) scope.x += shiftX;
  }
  for (const node of nodes.filter((node) => node.kind === "boundary" || node.kind === "discard")) {
    const scope = scopes.get(node.owner || rootOwner.id) || scopes.get(rootOwner.id);
    const position = positions.get(node.id);
    if (!scope || !position) continue;
    const direction = node.direction || (node.kind === "discard" ? "discard" : "input");
    const peers = nodes.filter((candidate) =>
      (candidate.owner || rootOwner.id) === (node.owner || rootOwner.id)
      && (candidate.direction || (candidate.kind === "discard" ? "discard" : "input")) === direction);
    const index = Math.max(0, peers.findIndex((candidate) => candidate.id === node.id));
    position.y = scope.y + 34 + (scope.height - 34) * (index + 1) / (peers.length + 1) - position.height / 2;
    position.x = direction === "input"
      ? scope.x - position.width + 10
      : scope.x + scope.width - 10;
  }
  const width = Math.max(...[...scopes.values()].map((scope) => scope.x + scope.width), nextX) + 18;
  const height = rootScope.y + rootScope.height + 18;
  return { width, height, positions, scopes };
}

function scopePortAnchor(owner, position, port) {
  const peers = (owner.ports || []).filter((candidate) => candidate.direction === port.direction);
  const index = Math.max(0, peers.findIndex((candidate) => candidate.name === port.name));
  return {
    x: port.direction === "input" ? position.x : position.x + position.width,
    y: position.y + 34 + (position.height - 34) * (index + 1) / (peers.length + 1),
  };
}

function portAnchor(node, position, portName, expectedDirection) {
  if (node.kind === "boundary" || node.kind === "discard") {
    const direction = node.direction || expectedDirection;
    return {
      x: direction === "input" ? position.x + position.width - 10 : position.x + 10,
      y: position.y + position.height / 2,
    };
  }
  const candidates = (node.ports || []).filter((port) => port.direction === expectedDirection);
  const index = Math.max(0, candidates.findIndex((port) => port.name === portName));
  const count = Math.max(1, candidates.length);
  return {
    x: expectedDirection === "input" ? position.x : position.x + position.width,
    y: position.y + position.height * (index + 1) / (count + 1),
  };
}

function itemDescriptor(item) {
  return [item.type || item.kind, ...(item.aspects || []).map((aspect) => `+${aspect}`)]
    .filter(Boolean)
    .join(" · ");
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
  fragment.append(historyTable("Observed envelopes", deliveries.slice(Math.max(0, deliveries.length - historyLimit)), ["index", "node", "port", "envelope_id", "meta", "data"]));
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
    cell.textContent = ["meta", "data"].includes(column)
      ? column.toUpperCase()
      : column.replaceAll("_", " ");
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

function setCommandStatus(message) {
  const status = document.querySelector("#command-status");
  status.textContent = message;
  status.hidden = !message;
}

function formatNanoseconds(value) {
  return value === undefined || value === null ? "—" : `${value} ns`;
}

for (const button of document.querySelectorAll("button[data-command]")) {
  button.addEventListener("click", async () => {
    try {
      await post(`/api/command/${button.dataset.command}`);
    } catch (error) {
      setCommandStatus(`UI command refused: ${error.message}`);
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
    setCommandStatus(`Scenario selection refused: ${error.message}`);
  }
});

const events = new EventSource("/api/events");
events.addEventListener("revision", () => refresh().catch((error) => setCommandStatus(error.message)));
events.onerror = () => {
  text("role", "reconnecting");
  controls = false;
  updateControls();
};

claim().then(refresh).catch((error) => setCommandStatus(error.message));
