"use strict";

const graphLayout = globalThis.RastokGraphLayout;
if (!graphLayout) throw new Error("the embedded graph layout capability did not load");

const controller = sessionStorage.getItem("rastok-controller") || crypto.randomUUID();
sessionStorage.setItem("rastok-controller", controller);
history.replaceState(null, "", location.pathname);

let controls = false;
let currentSnapshot = null;
let liveSnapshot = null;
let timelineIndex = null;
let manualHistoricalView = false;
let bufferedPlayback = false;
let replayPaused = false;
let playbackTimer = null;
let viewRequest = 0;
let refreshPromise = null;
let refreshAgain = false;
let historyLimit = 100;
let selectedTopologyItem = null;
let selectedLoopInstance = null;
let selectedMailboxRoute = null;
let mailboxVisibleLimit = 100;
let lastAnimatedActivity = null;
let pendingCommandRevision = null;
let commandOrigin = null;
let firstContact = true;
let selectedEnvelopeId = null;
let selectedRecordIndex = null;
let explainedFailure = null;
let comparisonRequest = 0;
let activeInspectorTab = "envelope";
let scenarioDefinitionActive = false;
let graphResizeObserver = null;
let graphTextSync = null;
let activeSourceView = sessionStorage.getItem("rastok-source-view") || null;
let lastSourcePoint = null;
let selectedSourceDefinition = null;
let sourceSelectionPending = false;
const sourceScroll = new Map();
const sourceValues = new Map();
let sourceTooltipAnchor = null;
let sourceTooltipTimer = null;
let dismissingSourceTooltip = false;
const graphViewport = { scale: 1, x: 0, y: 0 };

const COMMAND_HINTS = Object.freeze({
  resume: "Replay: replay recorded transitions at the selected speed",
  pause: "Pause: stop replay or execution at the next safe transition",
  step: "Next transition: advance one recorded transition",
  "retry-step": "Retry: retry the reserved transaction",
  "run-to-completion": "Finish: run to completion",
  "next-scenario-action": "Next action: run one authored action to its effect",
  "run-until-assertion": "Run to expect: continue to the next expect action",
  "reset-scenario": "Reset: start a clean scenario session",
  close: "Close UI",
});

async function post(path) {
  const headers = { "X-Rastok-Controller": controller };
  const response = await fetch(path, {
    method: "POST",
    headers,
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

function releaseControl() {
  if (!controls) return;
  controls = false;
  fetch("/api/control/release", {
    method: "POST",
    headers: { "X-Rastok-Controller": controller },
    keepalive: true,
  }).catch(() => {});
}

function refresh() {
  refreshAgain = true;
  if (!refreshPromise) {
    refreshPromise = (async () => {
      do {
        refreshAgain = false;
        await refreshSnapshot();
      } while (refreshAgain);
    })().finally(() => { refreshPromise = null; });
  }
  return refreshPromise;
}

async function refreshSnapshot() {
  const [snapshotResponse, timelineResponse] = await Promise.all([
    fetch("/api/snapshot", { cache: "no-store" }),
    fetch("/api/timeline", { cache: "no-store" }),
  ]);
  if (!snapshotResponse.ok) throw new Error(await snapshotResponse.text());
  if (!timelineResponse.ok) throw new Error(await timelineResponse.text());
  liveSnapshot = await snapshotResponse.json();
  timelineIndex = await timelineResponse.json();
  if (pendingCommandRevision !== null && liveSnapshot.revision > pendingCommandRevision) {
    pendingCommandRevision = null;
  }
  if (commandOrigin && pendingCommandRevision === null
      && (commandOrigin.command !== "next-scenario-action"
        || liveSnapshot.scenario?.current?.quiescent
        || liveSnapshot.execution?.state !== "running")) {
    const transitions = Number(liveSnapshot.timeline.boundary) - Number(commandOrigin?.boundary || 0);
    const action = Number(liveSnapshot.scenario?.current?.next_action_index || 0);
    const advancedAction = action > Number(commandOrigin?.action || 0);
    setCommandStatus(commandOrigin?.command === "next-scenario-action"
      ? `${transitions} transition${transitions === 1 ? "" : "s"} · authored action reached its effect`
      : commandOrigin?.command === "step" && commandOrigin.quiescent && advancedAction
        ? `no execution work ready · advanced to A${action}` : "");
    commandOrigin = null;
  }
  if (!currentSnapshot) {
    if (liveSnapshot.mode === "replay") {
      replayPaused = true;
      await showBoundary(liveSnapshot.timeline.session, 0);
      setCommandStatus("Recorded run · read-only replay · no Driver I/O");
      return;
    }
    currentSnapshot = liveSnapshot;
    renderSnapshot(currentSnapshot);
    return;
  }
  if (manualHistoricalView) {
    renderLiveExecutionState();
    renderTimeline();
    updateControls();
    return;
  }
  if (bufferedPlayback && !sameTimelinePoint(currentSnapshot, liveSnapshot)) {
    scheduleBufferedBoundary();
    renderExecutionState(currentSnapshot);
    renderTimeline();
    updateControls();
    return;
  }
  currentSnapshot = liveSnapshot;
  renderSnapshot(currentSnapshot);
}

function renderSnapshot(snapshot) {
  currentSnapshot = snapshot;
  const execution = snapshot.execution || {};
  renderExecutionState(snapshot);
  text("mode", snapshot.mode === "simulate" ? "SIMULATION" : (snapshot.mode || "—").toUpperCase());
  text("logical-time", formatLogicalTime(execution.logical_time_ns));
  document.querySelector("#logical-time").title = execution.logical_time_ns === undefined
    ? ""
    : `${execution.logical_time_ns} ns${snapshot.mode === "replay" ? " · checkpoint observation, not a record commit timestamp" : ""}`;
  text("awaiting-calls", String(execution.awaiting_outbound_calls || 0));

  const simulation = snapshot.mode === "simulate";
  for (const element of document.querySelectorAll(".simulation-only")) {
    element.hidden = !simulation;
  }
  renderFault(snapshot.fault);
  renderScenario(snapshot.scenario, simulation);
  const activity = snapshot.topology?.activity || null;
  if (!activity) lastAnimatedActivity = null;
  const activityKey = activity
    ? activity.ordinal === null
      ? `external:${activity.records?.from ?? "?"}:${activity.records?.through ?? "?"}`
      : `session:${snapshot.timeline?.session ?? 0}:boundary:${activity.ordinal}`
    : null;
  const animateBoundary = Boolean(activity) && activityKey !== lastAnimatedActivity;
  renderSource(snapshot);
  renderTopology(snapshot.topology, animateBoundary);
  if (animateBoundary) lastAnimatedActivity = activityKey;
  renderRelatedPanels(snapshot);
  renderTimeline();
  updateControls();
}

function selectSourceView(view, focus = false) {
  hideSourceTooltip();
  const pane = document.querySelector("#source-code");
  if (activeSourceView && activeSourceView !== "graph") sourceScroll.set(activeSourceView, [pane.scrollLeft, pane.scrollTop]);
  activeSourceView = view;
  sessionStorage.setItem("rastok-source-view", view);
  renderSource(currentSnapshot);
  if (view === "graph") renderTopology(currentSnapshot?.topology, false);
  if (focus) document.querySelector(`[data-source-view="${CSS.escape(view)}"]`)?.focus();
}

function renderSource(snapshot) {
  const sources = snapshot?.sources || [];
  const position = snapshot?.current_position;
  const point = `${snapshot?.timeline?.session}:${snapshot?.timeline?.boundary}`;
  const advanced = point !== lastSourcePoint;
  lastSourcePoint = point;
  if (advanced && activeSourceView !== "graph" && position?.status === "available") activeSourceView = String(position.source_index);
  if (!activeSourceView || (activeSourceView !== "graph" && !sources.some(source => String(source.index) === activeSourceView))) {
    activeSourceView = sources.length ? String(sources[0].index) : "graph";
  }
  const tabs = document.querySelector("#source-tabs");
  const focusedTab = tabs.contains(document.activeElement) ? document.activeElement.dataset.sourceView : null;
  tabs.replaceChildren();
  const views = [...sources.map(source => ({ id: String(source.index), name: source.display_name })), { id: "graph", name: "Graph" }];
  for (const view of views) {
    const tab = document.createElement("button");
    tab.type = "button";
    tab.id = `source-tab-${view.id}`;
    tab.dataset.sourceView = view.id;
    tab.setAttribute("role", "tab");
    tab.setAttribute("aria-controls", view.id === "graph" ? "graph" : "source-pane");
    tab.setAttribute("aria-selected", String(activeSourceView === view.id));
    tab.tabIndex = activeSourceView === view.id ? 0 : -1;
    tab.textContent = view.name;
    tab.addEventListener("click", () => selectSourceView(view.id));
    tab.addEventListener("keydown", event => {
      if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
      event.preventDefault();
      const index = views.findIndex(candidate => candidate.id === view.id);
      const next = event.key === "Home" ? 0 : event.key === "End" ? views.length - 1
        : (index + (event.key === "ArrowRight" ? 1 : -1) + views.length) % views.length;
      selectSourceView(views[next].id, true);
    });
    tabs.append(tab);
  }
  document.querySelector("#graph").hidden = activeSourceView !== "graph";
  const panel = document.querySelector("#source-pane");
  panel.hidden = activeSourceView === "graph";
  panel.setAttribute("aria-labelledby", `source-tab-${activeSourceView}`);
  if (focusedTab) document.querySelector(`[data-source-view="${CSS.escape(focusedTab)}"]`)?.focus({ preventScroll: true });
  if (activeSourceView === "graph") return;
  const source = sources.find(source => String(source.index) === activeSourceView);
  if (!source) return;
  const pane = document.querySelector("#source-code");
  const previousScroll = pane.dataset.sourceIndex === activeSourceView ? [pane.scrollLeft, pane.scrollTop] : sourceScroll.get(activeSourceView) || [0, 0];
  pane.dataset.sourceIndex = activeSourceView;
  const focusKey = pane.contains(document.activeElement) ? document.activeElement.dataset.sourceToken : null;
  hideSourceTooltip();
  pane.replaceChildren();
  sourceValues.clear();
  const tokens = (snapshot.source_tokens || []).filter(token => token.source_index === source.index);
  const byLine = new Map();
  for (const token of tokens) {
    if (!byLine.has(token.line)) byLine.set(token.line, []);
    byLine.get(token.line).push(token);
  }
  source.text.split("\n").slice(0, source.line_count).forEach((text, index) => {
    const line = document.createElement("span");
    const atSource = position?.status === "available" && position.source_index === source.index;
    const current = atSource && index + 1 >= position.first_line && index + 1 <= position.last_line;
    const fired = atSource && (position.fired_lines || []).includes(index + 1);
    const selected = selectedSourceDefinition?.source_index === source.index
      && index + 1 >= selectedSourceDefinition.first_line && index + 1 <= selectedSourceDefinition.last_line;
    line.className = `source-line${current ? " current-range" : ""}${fired ? " fired" : ""}${selected ? " selected-definition" : ""}`;
    if (atSource && index + 1 === position.first_line) {
      line.classList.add("current-position");
      line.setAttribute("aria-current", "step");
      line.dataset.nodePath = position.node_path ?? "";
    }
    line.dataset.line = String(index + 1);
    const chars = Array.from(text);
    let offset = 0;
    for (const token of byLine.get(index + 1) || []) {
      line.append(document.createTextNode(chars.slice(offset, token.column - 1).join("")));
      const span = document.createElement("span");
      span.className = `syntax-${token.kind}`;
      span.textContent = chars.slice(token.column - 1, token.end_column - 1).join("");
      if (token.value) {
        const key = `${token.source_index}:${token.line}:${token.column}`;
        span.dataset.sourceToken = key;
        span.tabIndex = 0;
        span.setAttribute("aria-label", `${token.name}: ${token.value.kind}`);
        span.addEventListener("mouseenter", () => showSourceTooltip(span));
        span.addEventListener("focus", () => { if (!dismissingSourceTooltip) showSourceTooltip(span); });
        span.addEventListener("mouseleave", scheduleSourceTooltipClose);
        span.addEventListener("blur", scheduleSourceTooltipClose);
        sourceValues.set(key, token);
      }
      line.append(span);
      offset = token.end_column - 1;
    }
    line.append(document.createTextNode(chars.slice(offset).join("")));
    pane.append(line);
  });
  pane.scrollLeft = previousScroll[0];
  pane.scrollTop = previousScroll[1];
  const policy = source.redacted ? " · Lab source withheld by data policy" : source.complete ? "" : " · recorded excerpts";
  text("source-status", (position?.status === "available"
    ? `Transition ${snapshot.timeline.boundary} · ${position.node_path || "App"}${position.port ? `.${position.port}` : ""} · line ${position.first_line}`
    : position?.reason || "Read-only source") + policy);
  if (advanced) scrollSourceLine(position?.source_index === source.index ? position.first_line : null);
  if (sourceSelectionPending && selectedSourceDefinition?.source_index === source.index) {
    scrollSourceLine(selectedSourceDefinition.first_line);
    sourceSelectionPending = false;
  }
  if (focusKey) pane.querySelector(`[data-source-token="${CSS.escape(focusKey)}"]`)?.focus({ preventScroll: true });
}

function scrollSourceLine(number) {
  if (!number) return;
  const pane = document.querySelector("#source-code");
  const line = pane.querySelector(`[data-line="${number}"]`);
  if (!line || document.querySelector("#source-pane").hidden) return;
  const box = pane.getBoundingClientRect();
  const target = line.getBoundingClientRect();
  if (target.top < box.top || target.bottom > box.bottom) {
    pane.scrollTo({ top: pane.scrollTop + target.top - box.top - pane.clientHeight / 3,
      behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
  }
}

function hideSourceTooltip() {
  clearTimeout(sourceTooltipTimer);
  document.querySelector("#source-tooltip").hidden = true;
  sourceTooltipAnchor?.removeAttribute("aria-describedby");
  sourceTooltipAnchor = null;
}

function scheduleSourceTooltipClose() {
  clearTimeout(sourceTooltipTimer);
  sourceTooltipTimer = setTimeout(() => {
    const tooltip = document.querySelector("#source-tooltip");
    if (!tooltip.matches(":hover") && !tooltip.contains(document.activeElement)
        && sourceTooltipAnchor !== document.activeElement && !sourceTooltipAnchor?.matches(":hover")) hideSourceTooltip();
  }, 150);
}

function showSourceTooltip(anchor) {
  const token = sourceValues.get(anchor.dataset.sourceToken);
  if (!token) return;
  hideSourceTooltip();
  sourceTooltipAnchor = anchor;
  anchor.setAttribute("aria-describedby", "source-tooltip");
  const tooltip = document.querySelector("#source-tooltip");
  const title = document.createElement("strong");
  title.textContent = `${token.name} · ${token.value.kind}`;
  const value = document.createElement("pre");
  value.textContent = token.value.rendering;
  const link = document.createElement("a");
  const target = token.value.inspector;
  link.href = `#panel-${target.tab}`;
  link.textContent = `Inspect in ${target.tab.charAt(0).toUpperCase() + target.tab.slice(1)}`;
  link.addEventListener("click", event => {
    event.preventDefault();
    const item = [...(currentSnapshot?.topology?.nodes || []), ...(currentSnapshot?.topology?.owners || [])]
      .find(item => item.path === target.node_path);
    if (item) selectedTopologyItem = item.id;
    if (target.tab === "mailbox") {
      const edge = currentSnapshot?.topology?.edges.find(edge => edge.mailbox.some(entry => entry.envelope_id === target.envelope_id));
      if (edge) inspectMailbox(edge.route);
    } else if (target.envelope_id) selectEnvelope(target.envelope_id);
    renderRelatedPanels(currentSnapshot);
    activateInspectorTab(target.tab, true);
    if (token.value.kind === "pending deadline") document.querySelector("#future").open = true;
    hideSourceTooltip();
  });
  // The next Tab from the identifier reaches this link without traversing the file.
  anchor.addEventListener("keydown", sourceTooltipKeydown);
  tooltip.replaceChildren(title, value, link);
  tooltip.hidden = false;
  const rect = anchor.getBoundingClientRect();
  tooltip.style.setProperty("left", `${Math.max(8, Math.min(rect.left, innerWidth - tooltip.offsetWidth - 8))}px`);
  tooltip.style.setProperty("top", `${Math.max(8, Math.min(rect.bottom + 6, innerHeight - tooltip.offsetHeight - 8))}px`);
}

function sourceTooltipKeydown(event) {
  if (event.key === "Tab" && !event.shiftKey && !document.querySelector("#source-tooltip").hidden) {
    event.preventDefault();
    document.querySelector("#source-tooltip a")?.focus();
  }
}

document.querySelector("#source-tooltip").addEventListener("mouseleave", scheduleSourceTooltipClose);
document.querySelector("#source-tooltip").addEventListener("focusout", scheduleSourceTooltipClose);
document.addEventListener("keydown", event => {
  if (event.key !== "Escape" || document.querySelector("#source-tooltip").hidden) return;
  event.preventDefault();
  event.stopImmediatePropagation();
  const anchor = sourceTooltipAnchor;
  hideSourceTooltip();
  dismissingSourceTooltip = true;
  anchor?.focus({ preventScroll: true });
  dismissingSourceTooltip = false;
}, true);

function renderLiveExecutionState() {
  renderExecutionState(currentSnapshot);
}

function renderExecutionState(snapshot) {
  const state = snapshot?.execution?.state || "paused";
  const liveState = liveSnapshot?.execution?.state || state;
  const atLive = sameTimelinePoint(snapshot, liveSnapshot);
  const pill = document.querySelector("#execution-state");
  const recording = snapshot?.mode === "replay";
  pill.title = recording ? "Read-only recording; no execution engine is running" : `Engine ${liveState}; ${atLive ? "viewing the live head" : "viewing a recorded transition"}`;
  if (!atLive && (bufferedPlayback || replayPaused)) {
    text("execution-state", replayPaused ? "replay paused" : "replaying");
    pill.className = `execution-state state-${replayPaused ? "paused" : "running"}`;
    return;
  }
  text("execution-state", recording && atLive ? (state === "completed" ? "recorded end" : "incomplete recording") : atLive ? state : "history");
  pill.className = `execution-state state-${atLive ? liveState : "history"}`;
}

function sameTimelinePoint(left, right) {
  return Number(left?.timeline?.session || 0) === Number(right?.timeline?.session || 0)
    && Number(left?.timeline?.boundary || 0) === Number(right?.timeline?.boundary || 0);
}

function timelineSession(session) {
  return (timelineIndex?.sessions || []).find((candidate) => Number(candidate.session) === Number(session));
}

function sessionName(session) {
  return Number(session) === 0 ? "Main" : `Session ${session}`;
}

function timelineFrames(session) {
  return timelineSession(session)?.frames || [];
}

async function showBoundary(session, boundary, manual = true) {
  const request = ++viewRequest;
  if (manual) setCommandStatus("");
  manualHistoricalView = manual;
  const response = await fetch(`/api/snapshot?session=${encodeURIComponent(session)}&boundary=${encodeURIComponent(boundary)}`, { cache: "no-store" });
  if (!response.ok) {
    const message = await response.text();
    if (request !== viewRequest) return false;
    throw new Error(message);
  }
  const snapshot = await response.json();
  if (request !== viewRequest) return false;
  currentSnapshot = snapshot;
  renderSnapshot(currentSnapshot);
  return true;
}

function scheduleBufferedBoundary() {
  if (playbackTimer !== null) return;
  const speed = Number(document.querySelector("#playback-speed").value);
  const session = Number(currentSnapshot?.timeline?.session || 0);
  const currentBoundary = Number(currentSnapshot?.timeline?.boundary || 0);
  const next = timelineFrames(session).find((frame) => Number(frame.boundary) > currentBoundary);
  if (!next) {
    const liveSession = Number(liveSnapshot?.timeline?.session || 0);
    const liveRunning = liveSnapshot?.execution?.state === "running";
    if (session !== liveSession || !liveRunning) {
      bufferedPlayback = false;
      manualHistoricalView = session !== liveSession;
    }
    renderExecutionState(currentSnapshot);
    renderTimeline();
    updateControls();
    return;
  }
  playbackTimer = setTimeout(async () => {
    playbackTimer = null;
    try {
      const presented = await showBoundary(session, next.boundary, false);
      if (presented && bufferedPlayback) scheduleBufferedBoundary();
    } catch (error) {
      bufferedPlayback = false;
      setCommandStatus(`Buffered playback stopped: ${error.message}`);
    }
  }, Math.round(1000 / speed));
}

function clearPlaybackTimer() {
  // Cancel the presentation intent as well as its timer: an older HTTP
  // response must not undo Pause, Reset or a newer history selection.
  viewRequest += 1;
  if (playbackTimer === null) return;
  clearTimeout(playbackTimer);
  playbackTimer = null;
}

function startBufferedPlayback() {
  clearPlaybackTimer();
  setCommandStatus("");
  manualHistoricalView = false;
  replayPaused = false;
  bufferedPlayback = true;
  renderExecutionState(currentSnapshot);
  renderTimeline();
  updateControls();
  scheduleBufferedBoundary();
}

function pauseBufferedPlayback() {
  clearPlaybackTimer();
  bufferedPlayback = false;
  replayPaused = true;
  manualHistoricalView = true;
  renderExecutionState(currentSnapshot);
  renderTimeline();
  updateControls();
  const session = Number(currentSnapshot?.timeline?.session || 0);
  const boundary = Number(currentSnapshot?.timeline?.boundary || 0);
  setCommandStatus(`Replay paused on ${sessionName(session)} · transition ${boundary}; the engine head was not changed.`);
}

function renderTimeline() {
  const session = Number(currentSnapshot?.timeline?.session || 0);
  const boundary = Number(currentSnapshot?.timeline?.boundary || 0);
  const liveSession = Number(liveSnapshot?.timeline?.session ?? session);
  const liveBoundary = Number(liveSnapshot?.timeline?.boundary ?? boundary);
  const frames = timelineFrames(session);
  const sessionSelector = document.querySelector("#timeline-session");
  const sessions = timelineIndex?.sessions || [];
  sessionSelector.replaceChildren(...sessions.map((candidate) => {
    const option = document.createElement("option");
    const candidateFrames = candidate.frames || [];
    const lastBoundary = Number(candidateFrames.at(-1)?.boundary || 0);
    option.value = String(candidate.session);
    option.textContent = `${sessionName(candidate.session)} · 0–${lastBoundary}`;
    option.selected = Number(candidate.session) === session;
    return option;
  }));
  sessionSelector.disabled = sessions.length < 2;
  const slider = document.querySelector("#timeline");
  slider.min = String(frames[0]?.boundary ?? 0);
  slider.max = String(frames.at(-1)?.boundary ?? boundary);
  slider.value = String(boundary);
  slider.disabled = frames.length < 2;
  const atLive = session === liveSession && boundary === liveBoundary;
  document.body.classList.toggle("historical-view", !atLive);
  const recording = liveSnapshot?.mode === "replay";
  const view = atLive
    ? (recording ? "RECORDED" : "LIVE")
    : bufferedPlayback
      ? "REPLAY"
      : replayPaused
        ? "REPLAY PAUSED"
        : "HISTORY";
  text("timeline-position", `${view} · ${sessionName(session)} · transition ${boundary}`);
  const engine = recording ? "recording · " : bufferedPlayback
    ? (liveSnapshot?.execution?.state === "completed" ? "engine completed · replay in progress · " : "engine finishing, then replay · ")
    : atLive ? "" : `engine ${liveSnapshot?.execution?.state || "paused"} · `;
  text("timeline-head", `${engine}head ${liveBoundary}`);
  const returnLive = document.querySelector("#return-live");
  returnLive.hidden = atLive;
  const returnLabel = `${recording ? "Return to recorded end" : "Return to live"} ${sessionName(liveSession)} · transition ${liveBoundary}`;
  returnLive.querySelector("span").textContent = returnLabel;
  returnLive.title = returnLabel;
  returnLive.setAttribute("aria-label", returnLabel);
}

function renderRelatedPanels(snapshot) {
  const selection = selectedTopologyContext(snapshot.topology);
  const envelope = document.querySelector("#envelope");
  envelope.className = "";
  envelope.replaceChildren(envelopePanel(snapshot));
  renderMailbox(snapshot.topology);
  const definition = document.querySelector("#definition");
  definition.className = "";
  definition.replaceChildren(scenarioDefinitionActive
    ? scenarioDefinitionPanel(snapshot.scenario?.current)
    : definitionPanel(selectedTopologyItemView(snapshot.topology)));
  renderHistory(snapshot.history, selection);
  renderInjections(snapshot, selection);
  renderInstances(snapshot.instances, selection);
  renderLoopRuns(snapshot.loop_runs, selection);
  renderRetryOperations(snapshot.retry_operations, selection);
  renderExchanges(snapshot.exchanges, selection);
  renderFuture(snapshot);
  renderSearchResults();
  renderAssertion(snapshot);
  renderComparison();
}

function identifier(value, label = "", action = null) {
  const id = String(value);
  const group = document.createElement("span");
  group.className = "identifier";
  const short = document.createElement(action ? "button" : "code");
  if (action) {
    short.type = "button";
    short.addEventListener("click", event => { event.stopPropagation(); action(); });
  }
  short.textContent = `${label ? `${label} ` : ""}${id.slice(0, 8)}`;
  short.title = id;
  const copy = document.createElement("button");
  copy.type = "button";
  copy.className = "copy-id";
  copy.textContent = "⧉";
  copy.title = `Copy ${label || "identifier"}`;
  copy.setAttribute("aria-label", copy.title);
  copy.addEventListener("click", event => {
    event.stopPropagation();
    navigator.clipboard.writeText(id).then(() => setCommandStatus("Identifier copied"))
      .catch(() => setCommandStatus("Copy unavailable; the complete identifier is in the tooltip."));
  });
  group.append(short, copy);
  return group;
}

function selectEnvelope(id) {
  selectedEnvelopeId = id;
  selectedRecordIndex = null;
  selectedTopologyItem = null;
  renderTopology(currentSnapshot?.topology, false);
  renderRelatedPanels(currentSnapshot);
  activateInspectorTab("envelope");
}

function attachTooltip(element, content) {
  const description = typeof content === "string" ? content : content.textContent;
  // Keep text labels' accessible text separate from the tooltip content.
  if (element.localName !== "text") {
    const title = svgElement("title");
    title.textContent = description;
    element.append(title);
  }
  element.setAttribute("tabindex", "0");
  element.setAttribute("aria-label", description);
  const show = () => {
    const popup = document.querySelector("#graph-tooltip") || document.body.appendChild(document.createElement("div"));
    popup.id = "graph-tooltip";
    popup.setAttribute("role", "tooltip");
    popup.replaceChildren(typeof content === "string" ? document.createTextNode(content) : content.cloneNode(true));
    const rect = element.getBoundingClientRect();
    popup.hidden = false;
    popup.style.left = `${Math.max(8, Math.min(rect.left, innerWidth - 340))}px`;
    popup.style.top = `${Math.max(8, Math.min(rect.bottom + 4, innerHeight - popup.offsetHeight - 8))}px`;
    element.setAttribute("aria-describedby", "graph-tooltip");
  };
  const hide = () => { const popup = document.querySelector("#graph-tooltip"); if (popup) popup.hidden = true; element.removeAttribute("aria-describedby"); };
  element.addEventListener("pointerenter", show);
  element.addEventListener("pointerleave", hide);
  element.addEventListener("focus", show);
  element.addEventListener("blur", hide);
}

function portDescription(topology, item, port) {
  const routes = (topology.routes || []).filter(rule => rule.owner === (item.path || "") && rule.port === port.name);
  const destinations = routes.flatMap(rule => rule.arms.map(arm => `${arm.cases.length ? `[${arm.cases.join(" | ")}] ` : ""}${arm.destinations.join(" ; ")}`));
  return `${item.path || "App"}.${port.name} · ${port.direction} · ${port.type || "type unavailable"}\n${destinations.length ? destinations.join("\n") : port.direction === "input" ? "Admission port" : item.parent === null ? "Public App output" : "No physical downstream route"}`;
}

function extensionDescription(item) {
  const loop = item.loop;
  return (item.extensions || []).map(extension => extension === "App"
    ? "App · public deployment scope · no manager"
    : extension === "Loop" && loop
      ? `Loop · entry ${loop.entry} · terminals ${loop.terminals.join(", ")}\nCommands: Loop.next · Loop.again · Loop.done\nAttributes: Loop.fuel · Loop.delay · Loop.measure · Loop.entry · Loop.terminal · Loop.expired\nFuel: ${loop.fuel ?? "derived from measure"} · extended terminal: ${loop.expired_port || "exhaustion proved impossible"}`
      : extension === "Retry" && item.retry
        ? `Retry · same prepared operation · ${item.retry.attempts} attempts maximum\nOn: ${item.retry.cases.join(" | ") || "no recurrence"}\nEntry: ${item.retry.entry} · terminal: ${item.retry.terminal}\nOnly the final attempt is delivered · no command or contributed port`
        : extension).join("\n");
}

function renderFuture(snapshot) {
  const items = document.querySelector("#future-items");
  const pending = snapshot.future || [];
  items.replaceChildren();
  for (const obligation of pending) {
    const row = document.createElement("button");
    row.type = "button";
    row.textContent = `${obligation.kind} · ${obligation.owner || "App"}${obligation.port ? `.${obligation.port}` : ""}${obligation.due_at_ns == null ? "" : ` · due ${formatLogicalTime(obligation.due_at_ns)}`}`;
    row.title = `${row.textContent} · recorded obligation, not an execution-order prediction`;
    if (obligation.envelope_id) row.addEventListener("click", () => selectEnvelope(obligation.envelope_id));
    items.append(row);
  }
  if (!pending.length) items.textContent = "No pending obligation at this transition.";
  document.querySelector("#future summary").textContent = `Pending · ${pending.length}`;
}

function renderMailbox(topology) {
  const target = document.querySelector("#mailbox");
  const edge = (topology?.edges || []).find((candidate) => candidate.route === selectedMailboxRoute);
  if (!edge) {
    empty(target, "Select a mailbox in the graph.");
    return;
  }
  const destination = (topology.nodes || []).find((node) => node.id === edge.target?.id);
  const name = destination
    ? `${destination.path || destination.label}.${edge.target.port}`
    : edge.route;
  const deliveries = edge.mailbox || [];
  const heading = document.createElement("header");
  heading.className = "mailbox-heading";
  const title = document.createElement("h3");
  title.textContent = `Mailbox · ${name}`;
  const summary = document.createElement("p");
  summary.textContent = `${deliveries.length} message${deliveries.length === 1 ? "" : "s"} waiting · ${edge.route}`;
  heading.append(title, summary);

  const list = document.createElement("div");
  list.className = "mailbox-deliveries";
  for (const [index, delivery] of deliveries.slice(0, mailboxVisibleLimit).entries()) {
    const details = document.createElement("details");
    details.className = "mailbox-delivery";
    details.open = index < 3;
    const row = document.createElement("summary");
    const id = String(delivery.envelope_id || "unknown");
    row.textContent = `#${index + 1} · ${id.slice(0, 8)}${delivery.claimed ? " · claimed" : ""}`;
    row.title = id;
    const meta = document.createElement("p");
    meta.className = "mailbox-delivery-meta";
    meta.textContent = `port ${delivery.port || "?"} · sequence ${delivery.sequence || "?"} · generation ${delivery.instance?.generation ?? "?"}`;
    details.append(row, meta, envelopeZones(delivery.meta, delivery.data));
    list.append(details);
  }
  if (mailboxVisibleLimit < deliveries.length) {
    const more = document.createElement("button");
    more.type = "button";
    more.className = "mailbox-more secondary";
    more.textContent = `Show ${Math.min(100, deliveries.length - mailboxVisibleLimit)} more`;
    more.addEventListener("click", () => {
      mailboxVisibleLimit += 100;
      renderMailbox(topology);
    });
    list.append(more);
  }
  target.className = "";
  target.replaceChildren(heading, list);
}

function inspectScenarioDefinition() {
  scenarioDefinitionActive = true;
  selectedTopologyItem = null;
  selectedMailboxRoute = null;
  renderTopology(currentSnapshot?.topology, false);
  if (currentSnapshot) renderRelatedPanels(currentSnapshot);
  activateInspectorTab("definition");
}

function scenarioDefinitionPanel(scenario) {
  const panel = document.createElement("aside");
  panel.className = `node-definition${scenario ? " populated" : " empty"}`;
  panel.setAttribute("aria-label", "Selected scenario definition");
  if (!scenario) {
    const title = document.createElement("h3");
    title.textContent = "Scenario definition";
    panel.append(title);
    return panel;
  }
  const heading = document.createElement("div");
  heading.className = "node-definition-heading";
  const identity = document.createElement("div");
  const eyebrow = document.createElement("p");
  eyebrow.className = "node-definition-kind";
  eyebrow.textContent = "scenario";
  const title = document.createElement("h3");
  title.textContent = `${scenario.simulation || "?"}.${scenario.scenario || "?"}`;
  identity.append(eyebrow, title);
  const close = document.createElement("button");
  close.type = "button";
  close.className = "node-definition-close secondary";
  close.textContent = "Close";
  close.setAttribute("aria-label", "Close scenario definition");
  close.addEventListener("click", () => {
    scenarioDefinitionActive = false;
    renderRelatedPanels(currentSnapshot);
    activateInspectorTab("envelope");
  });
  heading.append(identity, close);
  const progress = document.createElement("p");
  progress.className = "scenario-definition-progress";
  progress.textContent = `A${scenario.next_action_index || 0}/${scenario.action_count || 0} · ${scenario.complete ? "scenario complete" : `next: ${scenario.next_action || "—"}`}`;
  panel.append(heading, progress);
  const definition = scenario.definition;
  if (typeof definition?.code === "string") {
    const pre = document.createElement("pre");
    pre.className = "node-definition-code scenario-definition-code";
    const actionIndex = Number(scenario.next_action_index || 0);
    const displayedAction = actionIndex > 0 ? actionIndex - 1 : 0;
    const currentLine = definition.action_lines?.[displayedAction];
    definition.code.split("\n").forEach((text, index) => {
      const line = document.createElement("span");
      line.className = `source-line${index + 1 === currentLine ? " current" : ""}`;
      line.dataset.line = String(Number(definition.first_line) + index);
      line.textContent = text;
      if (index + 1 === currentLine) line.setAttribute("aria-current", "step");
      const action = definition.action_lines?.indexOf(index + 1) ?? -1;
      if (action >= 0) {
        const frame = timelineFrames(currentSnapshot?.timeline?.session || 0).find(frame => Number(frame.action) === action + 1);
        line.dataset.action = String(action + 1);
        line.title = frame ? `Action ${action + 1} · transition ${frame.boundary}` : `Action ${action + 1} · not yet executed`;
        if (frame) {
          line.tabIndex = 0;
          line.setAttribute("role", "button");
          const open = () => { pausePresentation(); showBoundary(currentSnapshot.timeline.session, frame.boundary).catch(showViewError); };
          line.addEventListener("click", open);
          line.addEventListener("keydown", event => { if (event.key === "Enter") { event.preventDefault(); open(); } });
        }
      }
      pre.append(line);
    });
    panel.append(pre);
  } else {
    const note = document.createElement("p");
    note.className = "scenario-source-limit";
    note.textContent = definition?.reason || "Authored scenario source is unavailable in this snapshot.";
    panel.append(note);
  }
  return panel;
}

function updateControls() {
  const legal = new Set(liveSnapshot?.execution?.legal_commands || currentSnapshot?.execution?.legal_commands || []);
  const atLive = sameTimelinePoint(currentSnapshot, liveSnapshot);
  const session = Number(currentSnapshot?.timeline?.session || 0);
  const boundary = Number(currentSnapshot?.timeline?.boundary || 0);
  const frames = timelineFrames(session);
  const canReplayForward = frames.some((frame) => Number(frame.boundary) > boundary);
  for (const button of document.querySelectorAll("button[data-command]")) {
    const command = button.dataset.command;
    const replayPlay = button.dataset.command === "resume" && !atLive && canReplayForward;
    const replayPause = button.dataset.command === "pause" && bufferedPlayback;
    const recordingNavigation = liveSnapshot?.mode === "replay" && canReplayForward && (command === "step" || command === "run-to-completion");
    const localReplayControl = replayPlay || replayPause || recordingNavigation;
    const physicalControl = button.dataset.command === "pause" || button.dataset.command === "close";
    button.disabled = pendingCommandRevision !== null
      || (!localReplayControl && !controls)
      || (!localReplayControl && ((!atLive && !physicalControl) || !legal.has(button.dataset.command)));
    const base = COMMAND_HINTS[command] || command;
    let reason = "";
    if (button.disabled) {
      if (pendingCommandRevision !== null) reason = "the previous command is still being recorded";
      else if (liveSnapshot?.mode === "replay" && command !== "close") reason = "recordings are read-only; only recorded timeline navigation is available";
      else if (!controls) reason = "this browser is an observer; take control first";
      else if (!atLive && !physicalControl) {
        reason = "available in LIVE only; select Return live";
      } else if (!legal.has(command)) {
        reason = command === "next-scenario-action" && liveSnapshot?.execution?.state === "paused"
          ? "the current scenario action is still executing; use Next transition or Replay"
          : "not legal at the current execution state";
      }
    }
    button.title = reason ? `${base} — unavailable: ${reason}.` : base;
    button.setAttribute("aria-label", reason ? `${base}. Unavailable: ${reason}.` : base);
  }
  const selectScenario = document.querySelector("#select-scenario");
  selectScenario.disabled = !controls || !atLive || !legal.has("select-scenario") || !document.querySelector("#scenario-select").value;
  selectScenario.title = selectScenario.disabled
    ? "Select scenario — unavailable until this controller returns to LIVE and selection is legal."
    : "Select the checked scenario";
  const back = document.querySelector("#back-boundary");
  back.disabled = frames.findIndex((frame) => Number(frame.boundary) === boundary) <= 0;
  back.title = back.disabled
    ? "Back — no earlier recorded transition exists on this session."
    : "Back: view the previous recorded transition";
  document.querySelector("#take-control").hidden = controls;
}

function renderFault(fault) {
  const panel = document.querySelector("#fault-panel");
  panel.hidden = !fault;
  if (!fault) return;
  text("fault-title", "Execution fault");
  text("fault-message", fault.message || "The transaction faulted.");
  text("fault-help", fault.help || "Inspect the recorded state before choosing a legal action.");
  text("fault-commit", fault.commit || "Nothing from the faulting transaction was committed.");
}

function renderScenario(scenario, enabled) {
  if (!enabled || !scenario) return;
  const current = scenario.current || {};
  const next = current.next_action ? `next: ${current.next_action}` : "no remaining action";
  text(
    "scenario-progress",
    `${current.simulation || "?"}.${current.scenario || "?"} · A${current.next_action_index || 0}/${current.action_count || 0}`,
  );
  const failures = (current.failures || []).length + (current.golden_failures || []).length;
  text(
    "scenario-status",
    `A${current.next_action_index || 0}/${current.action_count || 0} · ${next} · ${failures} failure${failures === 1 ? "" : "s"}`,
  );
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
  document.querySelector("#scenario-definition").textContent = current.scenario || "Scenario";
}

function keepGraphTextAtInterfaceSize(svg, intrinsicWidth) {
  const sync = () => {
    const renderedWidth = svg.getBoundingClientRect().width;
    if (!(renderedWidth > 0) || !(intrinsicWidth > 0)) return;
    const scale = Math.max(renderedWidth / intrinsicWidth / graphViewport.scale, 0.1);
    svg.style.setProperty("--graph-title-font-size", `${12 / scale}px`);
    svg.style.setProperty("--graph-mono-font-size", `${10 / scale}px`);
    svg.style.setProperty("--graph-small-font-size", `${9 / scale}px`);
    svg.style.setProperty("--graph-text-halo-width", `${3 / scale}px`);
  };
  graphTextSync = sync;
  graphResizeObserver?.disconnect();
  if (typeof ResizeObserver === "function") {
    graphResizeObserver = new ResizeObserver(sync);
    graphResizeObserver.observe(svg);
  }
  sync();
  requestAnimationFrame(sync);
}

function clampGraphZoom(value) {
  return Math.min(2.5, Math.max(0.5, value));
}

function applyGraphViewport(layer, label) {
  layer.style.transform = `matrix(${graphViewport.scale}, 0, 0, ${graphViewport.scale}, ${graphViewport.x}, ${graphViewport.y})`;
  if (label) {
    label.textContent = `${Math.round(graphViewport.scale * 100)}%`;
    label.title = "Zoom relative to Fit graph (100%); buttons zoom around the canvas centre, wheel around the pointer";
    label.setAttribute("aria-label", `${label.textContent} of fitted graph`);
  }
  requestAnimationFrame(() => graphTextSync?.());
}

function zoomGraphAt(layer, label, canvas, nextScale, clientX, clientY) {
  const scale = clampGraphZoom(nextScale);
  const bounds = canvas.getBoundingClientRect();
  const anchorX = clientX - bounds.left;
  const anchorY = clientY - bounds.top;
  const worldX = (anchorX - graphViewport.x) / graphViewport.scale;
  const worldY = (anchorY - graphViewport.y) / graphViewport.scale;
  graphViewport.x = anchorX - worldX * scale;
  graphViewport.y = anchorY - worldY * scale;
  graphViewport.scale = scale;
  applyGraphViewport(layer, label);
}

function installGraphViewport(canvas, layer) {
  const controls = document.createElement("div");
  controls.className = "graph-viewport-controls";
  controls.setAttribute("aria-label", "Graph viewport controls");
  const label = document.createElement("span");
  label.className = "graph-zoom-value";
  const button = (icon, accessible, action) => {
    const control = document.createElement("button");
    control.type = "button";
    control.title = accessible;
    control.setAttribute("aria-label", accessible);
    const image = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
    use.setAttribute("href", `#icon-${icon}`);
    image.append(use);
    control.append(image);
    control.addEventListener("click", (event) => {
      event.stopPropagation();
      action();
    });
    return control;
  };
  const center = () => {
    const bounds = canvas.getBoundingClientRect();
    return { x: bounds.left + bounds.width / 2, y: bounds.top + bounds.height / 2 };
  };
  controls.append(
    button("zoom-out", "Zoom out", () => {
      const point = center();
      zoomGraphAt(layer, label, canvas, graphViewport.scale / 1.2, point.x, point.y);
    }),
    button("zoom-in", "Zoom in", () => {
      const point = center();
      zoomGraphAt(layer, label, canvas, graphViewport.scale * 1.2, point.x, point.y);
    }),
    button("fit", "Fit graph", () => {
      graphViewport.scale = 1;
      graphViewport.x = 0;
      graphViewport.y = 0;
      applyGraphViewport(layer, label);
    }),
    label,
  );

  let drag = null;
  canvas.addEventListener("pointerdown", (event) => {
    if (event.button !== 0 || event.target.closest("button, .graph-node, .graph-instance-badge, .edge-counter, .graph-scope-title-hit, text")) return;
    drag = { x: event.clientX, y: event.clientY, panX: graphViewport.x, panY: graphViewport.y };
    canvas.classList.add("panning");
    canvas.setPointerCapture(event.pointerId);
  });
  canvas.addEventListener("pointermove", (event) => {
    if (!drag) return;
    graphViewport.x = drag.panX + event.clientX - drag.x;
    graphViewport.y = drag.panY + event.clientY - drag.y;
    applyGraphViewport(layer, label);
  });
  const finishPan = (event) => {
    if (!drag) return;
    drag = null;
    canvas.classList.remove("panning");
    if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
  };
  canvas.addEventListener("pointerup", finishPan);
  canvas.addEventListener("pointercancel", finishPan);
  canvas.addEventListener("wheel", (event) => {
    event.preventDefault();
    zoomGraphAt(
      layer,
      label,
      canvas,
      graphViewport.scale * (event.deltaY < 0 ? 1.12 : 1 / 1.12),
      event.clientX,
      event.clientY,
    );
  }, { passive: false });
  applyGraphViewport(layer, label);
  return controls;
}

function renderTopology(topology, animateBoundary = false) {
  const tooltip = document.querySelector("#graph-tooltip");
  if (tooltip) tooltip.hidden = true;
  const target = document.querySelector("#graph");
  const focusKey = target.contains(document.activeElement)
    ? document.activeElement.dataset.focusKey : null;
  if (!topology || !(topology.nodes || []).length) {
    empty(target, "Waiting for a validated topology.");
    return;
  }
  const displayed = (item) => ({ ...item, ports: (item.ports || []).map((port) => ({
    ...port, display_label: commandPortLabel(topology, item, port),
  })) });
  topology = { ...topology, nodes: (topology.nodes || []).map(displayed), owners: (topology.owners || []).map(displayed) };
  const activity = topology.activity || null;
  const causalIds = lineage();
  const relatedRecords = (currentSnapshot?.history?.records || []).filter(record => lineageRecord(record, causalIds));
  const relatedOwners = new Set(relatedRecords.map(record => record.owner ?? record.connector));
  const drainedOutputs = new Set((topology.edges || [])
    .filter((edge) => edge.target?.kind === "discard")
    .map((edge) => `${edge.source.id}\u0000${edge.source.port || ""}`));
  const activityPanel = document.createElement("div");
  activityPanel.className = `graph-activity${activity ? ` activity-${activity.kind}` : ""}${animateBoundary ? " activity-new" : ""}`;
  const activityLabel = document.createElement("span");
  activityLabel.className = "graph-activity-label";
  const activityKind = activity?.kind === "scenario-action"
    ? "action"
    : activity?.kind === "inbound"
      ? "admission"
      : activity?.kind === "transaction"
    ? "transaction"
    : activity?.kind === "lifecycle"
      ? "lifecycle"
      : activity?.kind === "call-boundary"
        ? "call"
        : activity?.kind === "session-boundary"
          ? "session"
          : "exchange";
  activityLabel.textContent = activity
    ? activity.ordinal === null
      ? `External · ${activityKind}`
      : `transition ${activity.ordinal} · ${activityKind}`
    : "transition 0 · initial";
  const activityText = document.createElement("strong");
  activityText.textContent = activity?.detail || "Before admission · no transition has advanced.";
  activityPanel.append(activityLabel, activityText);
  if (activity?.kind === "lifecycle") {
    const note = document.createElement("span");
    note.className = "graph-activity-note";
    note.textContent = "Instance lifecycle only · no envelope moved";
    activityPanel.append(note);
  }

  const graph = graphLayout.materializeGraph(topology);
  const layout = graphLayout.layoutGraph(graph.nodes, graph.edges, topology.owners || []);
  const edgeRoutes = graphLayout.routeGraphEdges(graph, layout);
  const canvas = document.createElement("div");
  canvas.className = "graph-canvas";
  const svg = svgElement("svg", {
    viewBox: `0 0 ${layout.width} ${layout.height}`,
    width: layout.width,
    height: layout.height,
    preserveAspectRatio: "xMinYMin meet",
    role: "img",
    "aria-label": "Validated Rastok graph with current mailbox occupancy",
  });
  svg.style.setProperty("--graph-fit-width", `${Math.ceil(layout.width * 1.5)}px`);
  svg.style.setProperty("--graph-readable-width", `${layout.width * graphLayout.minimumReadableScale}px`);
  svg.classList.add("graph-svg");
  const definitions = svgElement("defs");
  const marker = svgElement("marker", {
    id: "graph-arrow",
    viewBox: "0 0 11 7",
    refX: "11",
    refY: "3.5",
    markerWidth: "11",
    markerHeight: "7",
    markerUnits: "userSpaceOnUse",
    orient: "auto",
  });
  marker.append(svgElement("path", { d: "M 0 0 L 7 3.5 L 0 7 z" }));
  const activeMarker = svgElement("marker", {
    id: "graph-arrow-active",
    viewBox: "0 0 11 7",
    refX: "11",
    refY: "3.5",
    markerWidth: "11",
    markerHeight: "7",
    markerUnits: "userSpaceOnUse",
    orient: "auto",
  });
  activeMarker.append(svgElement("path", { d: "M 0 0 L 7 3.5 L 0 7 z" }));
  definitions.append(marker, activeMarker);
  svg.append(definitions);

  const scopeLayer = svgElement("g", { class: "graph-scope-layer" });
  const orderedOwners = [...(topology.owners || [])]
    .sort((left, right) => (layout.scopes.get(left.id)?.depth || 0) - (layout.scopes.get(right.id)?.depth || 0));
  for (const owner of orderedOwners) {
    const position = layout.scopes.get(owner.id);
    if (!position) continue;
    const active = activity?.node === owner.path || currentSnapshot?.current_position?.node_path === owner.path;
    const selectable = Boolean(owner.definition);
    const selected = selectedTopologyItem === owner.id;
    const group = svgElement("g", {
      class: `graph-scope${position.depth === 0 ? " root" : " nested"}${active ? " active" : ""}${active && animateBoundary ? " moving" : ""}${selectable ? " selectable" : ""}${selected ? " selected" : ""}`,
    });
    group.classList.toggle("causal-related", relatedOwners.has(owner.path));
    if (selectable) {
      group.setAttribute("role", "button");
      group.dataset.focusKey = owner.id;
      group.setAttribute("tabindex", "0");
      group.setAttribute("aria-pressed", String(selected));
      group.setAttribute("aria-label", `Inspect ${owner.path || "App"} Graph definition`);
      group.addEventListener("click", (event) => {
        const classList = event.target.classList;
        if (classList.contains("graph-scope-title-hit")
          || classList.contains("graph-scope-name")
          || classList.contains("graph-scope-type")) {
          selectTopologyItem(owner.id);
        } else if (classList.contains("graph-scope-body")) {
          selectTopologyItem(null);
        }
      });
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
      rx: "0",
    });
    const header = svgElement("line", {
      class: "graph-scope-header-line",
      x1: position.x,
      y1: position.y + 26,
      x2: position.x + position.width,
      y2: position.y + 26,
    });
    const titleHit = svgElement("rect", {
      class: "graph-scope-title-hit",
      x: position.x,
      y: position.y,
      width: position.width,
      height: "26",
    });
    const name = svgElement("text", {
      class: "graph-scope-name",
      x: position.x + 14,
      y: position.y + 18,
    });
    name.textContent = truncate(owner.label || "App", 28);
    attachTooltip(name, `${owner.label || 'App'} · Kind: Graph · ${(owner.instances || []).length} open instances`);
    const type = svgElement("text", {
      class: "graph-scope-type",
      x: position.x + position.width - (owner.instances?.length ? 56 : 8),
      y: position.y + 18,
      "text-anchor": "end",
    });
    type.textContent = scopeDescriptor(owner);
    attachTooltip(type, extensionDescription(owner));
    appendLoopDial(type, owner);
    group.append(title, body, header, titleHit, name, type);
    appendInstanceBadge(
      group,
      owner.instances || [],
      position.x + position.width - 8,
      position.y + 7,
      owner.id,
    );
    for (const port of owner.ports || []) {
      const anchor = graphLayout.scopePortAnchor(owner, position, port, layout);
      const pin = svgElement("rect", {
        class: `graph-port graph-port-${port.direction}`,
        x: anchor.x - 4,
        y: anchor.y - 4,
        width: "8",
        height: "8",
      });
      const label = svgElement("text", {
        class: "graph-scope-port-label",
        x: anchor.x + (port.direction === "input" ? 9 : -9),
        y: anchor.y - 5,
        "text-anchor": port.direction === "input" ? "start" : "end",
      });
      label.textContent = port.display_label || port.name;
      pin.dataset.port = port.name;
      attachTooltip(pin, portDescription(topology, owner, port));
      const inspectPort = event => {
        const outputs = owner.path ? [] : (currentSnapshot?.history?.outputs || []);
        const observed = [...(currentSnapshot?.history?.deliveries || []), ...outputs]
          .findLast(delivery => (delivery.node || "") === (owner.path || "") && delivery.port === port.name);
        if (observed) { event.stopPropagation(); selectEnvelope(observed.envelope_id); }
      };
      pin.addEventListener("click", inspectPort);
      pin.addEventListener("keydown", event => { if (event.key === "Enter") { event.preventDefault(); inspectPort(event); } });
      group.append(pin, label);
    }
    scopeLayer.append(group);
  }
  svg.append(scopeLayer);

  const edgeLayer = svgElement("g", { class: "graph-edge-layer" });
  const activeRoutes = new Map((activity?.routes || []).map((route) => [route.route, route.phase]));
  for (const edge of graph.edges) {
    const source = graph.byId.get(edge.source.id);
    const destination = graph.byId.get(edge.target.id);
    const sourcePosition = layout.positions.get(edge.source.id);
    const destinationPosition = layout.positions.get(edge.target.id);
    if (!source || !destination || !sourcePosition || !destinationPosition) continue;
    const activityPhase = activeRoutes.get(edge.route) || null;
    const active = activityPhase !== null;
    const queuedNow = Number(edge.queued_count || 0) > 0;
    const mailbox = edge.target?.kind === "node";
    const selected = selectedMailboxRoute === edge.route;
    const group = svgElement("g", {
      class: `graph-edge${active ? " active" : ""}${queuedNow ? " queued" : ""}${selected ? " selected" : ""}${active && animateBoundary ? " moving" : ""}`,
      "data-route": edge.route,
      "data-activity": activityPhase || "idle",
    });
    const sourceOwner = (topology.nodes || []).find(node => node.id === edge.source.id)?.path ?? null;
    const sourceDelivery = (currentSnapshot?.history?.deliveries || []).findLast(delivery => delivery.node === sourceOwner && delivery.port === edge.source.port && (!selectedEnvelopeId || causalIds.has(delivery.envelope_id)));
    const selectedCase = sourceDelivery?.data?.content?.case ?? sourceDelivery?.data?.content;
    group.classList.toggle("causal-related", Boolean(sourceDelivery && causalIds.has(sourceDelivery.envelope_id)));
    group.classList.toggle("case-selected", Boolean(sourceDelivery && edge.cases?.includes(String(selectedCase))));
    if (edge.cases?.length) group.dataset.selectedCase = edge.cases.includes(String(selectedCase)) ? String(selectedCase) : "";
    const route = edgeRoutes.get(edge.index);
    if (!route) continue;
    const pathData = graphLayout.curvedCorridorPath(route.presentationPoints || route.points);
    const path = svgElement("path", {
      class: "graph-edge-path",
      d: pathData,
      "marker-end": active
        ? "url(#graph-arrow-active)"
        : "url(#graph-arrow)",
    });
    const title = svgElement("title");
    title.textContent = mailbox
      ? `${edge.route} · ${edge.queued_count || "0"} messages waiting · capacity ${edge.capacity ?? "not exposed"} (shared instance mailbox)${edge.cases?.length ? ` · cases ${edge.cases.join(" | ")}` : ""}`
      : edge.route;
    path.append(title);
    attachTooltip(path, title.textContent);
    if (sourceDelivery) path.addEventListener("click", () => selectEnvelope(sourceDelivery.envelope_id));
    group.append(path);

    if (active && animateBoundary) {
      const packet = svgElement("circle", {
        class: "graph-edge-packet",
        r: "4",
        "aria-hidden": "true",
      });
      packet.append(svgElement("animateMotion", {
        dur: "700ms",
        fill: "freeze",
        path: pathData,
      }));
      group.append(packet);
    }

    if (route.label.leader) {
      group.append(svgElement("line", {
        class: "edge-label-leader",
        x1: route.label.leader.from.x,
        y1: route.label.leader.from.y,
        x2: route.label.leader.to.x,
        y2: route.label.leader.to.y,
      }));
    }

    if (mailbox) {
      const counterWidth = route.label.box.width;
      const badge = svgElement("g", {
        class: "edge-counter",
        transform: `translate(${route.label.x - counterWidth / 2} ${route.label.y - 9})`,
        role: "button",
        tabindex: "0",
        "aria-label": `Open mailbox ${destination.path || destination.label}.${edge.target.port}, ${edge.queued_count || "0"} messages waiting`,
      });
      badge.dataset.focusKey = `mailbox:${edge.route}`;
      const counterTitle = svgElement("title");
      counterTitle.textContent = `Mailbox ${destination.path || destination.label}.${edge.target.port}: ${edge.queued_count || "0"} messages waiting`;
      badge.append(svgElement("rect", { width: counterWidth, height: "18", rx: "2" }));
      const queued = svgElement("text", { x: counterWidth / 2, y: "13", "text-anchor": "middle" });
      queued.textContent = `≡ ${edge.queued_count || "0"}`;
      badge.append(counterTitle, queued);
      const preview = document.createElement("div");
      const header = document.createElement("strong");
      header.textContent = counterTitle.textContent;
      preview.append(header);
      for (const delivery of (edge.mailbox || []).slice(0, 5)) {
        const row = document.createElement("div");
        row.className = "mailbox-preview-row";
        row.textContent = `${String(delivery.envelope_id).slice(0, 8)}${delivery.claimed ? " · claimed" : ""}`;
        appendDataTree(row, delivery.data);
        preview.append(row);
      }
      if ((edge.mailbox || []).length > 5) preview.append(document.createTextNode(`First 5 of ${edge.mailbox.length}; open the mailbox for the complete FIFO.`));
      attachTooltip(badge, preview);
      const inspect = (event) => {
        event.stopPropagation();
        inspectMailbox(edge.route);
      };
      badge.addEventListener("click", inspect);
      badge.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          inspect(event);
        }
      });
      group.append(badge);
    }
    edgeLayer.append(group);
  }
  svg.append(edgeLayer);

  const nodeLayer = svgElement("g", { class: "graph-node-layer" });
  for (const node of graph.nodes) {
    const position = layout.positions.get(node.id);
    if (!position) continue;
    const active = activity?.node === node.path || currentSnapshot?.current_position?.node_path === node.path;
    const selectable = Boolean(node.definition);
    const selected = selectedTopologyItem === node.id;
    const boundary = node.kind === "boundary" || node.kind === "discard";
    const group = svgElement("g", {
      class: `graph-node graph-node-${node.kind}${active ? " active" : ""}${active && animateBoundary ? " moving" : ""}${selectable ? " selectable" : ""}${selected ? " selected" : ""}`,
      transform: `translate(${position.x} ${position.y})`,
    });
    group.classList.toggle("causal-related", relatedOwners.has(node.path));
    if (selectable) {
      const inspectId = node.id;
      group.setAttribute("role", "button");
      group.dataset.focusKey = node.id;
      group.setAttribute("tabindex", "0");
      group.setAttribute("aria-pressed", String(selected));
      group.setAttribute("aria-label", `Inspect ${node.path} definition`);
      group.addEventListener("click", () => {
        selectTopologyItem(inspectId);
      });
      group.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          selectTopologyItem(inspectId);
        }
      });
    }
    const title = svgElement("title");
    title.textContent = node.path
      ? `${node.path} · ${node.kind} · ${itemDescriptor(node)} · ${node.profile || "DSL Dispatch"} · ${(node.instances || []).length} open instances`
      : `${node.label} · ${node.kind}`;
    group.append(title);
    if (node.kind === "boundary") {
      continue;
    }
    if (boundary) {
      const direction = node.direction || (node.kind === "discard" ? "discard" : "input");
      const pinX = direction === "input" ? position.width - 10 : 10;
      const pin = svgElement("rect", {
        class: `graph-port graph-port-${direction}`,
        x: pinX - 4,
        y: position.height / 2 - 4,
        width: "8",
        height: "8",
      });
      const label = svgElement("text", {
        class: "graph-boundary-label",
        x: pinX - 12,
        y: position.height / 2 - 5,
        "text-anchor": "end",
      });
      const boundaryLabel = node.kind === "discard"
        ? String(node.label || "Discard").replace(/^Discard · /, "")
        : node.label || node.ports?.[0]?.name || "port";
      label.textContent = boundaryLabel;
      group.append(pin, label);
      nodeLayer.append(group);
      continue;
    }
    const body = svgElement("rect", {
      class: "graph-node-body",
      width: position.width,
      height: position.height,
      rx: "3",
    });
    const subtitle = graphLayout.nodeSubtitle(node);
    const name = svgElement("text", {
      class: "graph-node-name",
      x: "10",
      y: subtitle ? "17" : "25",
      "text-anchor": "start",
    });
    name.textContent = truncate(node.label || node.path || node.id, 22);
    attachTooltip(name, title.textContent);
    group.append(body, name);
    if (subtitle) {
      const type = svgElement("text", {
        class: "graph-node-type",
        x: "10",
        y: "33",
        "text-anchor": "start",
      });
      type.textContent = subtitle;
      attachTooltip(type, extensionDescription(node) || node.profile || node.type);
      appendLoopDial(type, node);
      group.append(type);
    }
    appendInstanceBadge(group, node.instances || [], position.width - 8, 6, node.id);
    if (active && animateBoundary && ["inbound", "outbound"].includes(activity?.motion)) {
      const outward = activity.motion === "outbound";
      const from = outward ? position.width / 2 : position.width + 20;
      const to = outward ? position.width + 20 : position.width / 2;
      const packet = svgElement("circle", {
        class: "connector-boundary-packet",
        cx: String(from),
        cy: String(position.height / 2),
        r: "4",
        "aria-hidden": "true",
      });
      packet.append(svgElement("animate", {
        attributeName: "cx",
        from: String(from),
        to: String(to),
        dur: "700ms",
        fill: "freeze",
      }));
      group.append(packet);
    }
    for (const port of node.ports || []) {
      const anchor = graphLayout.portAnchor(
        node,
        { x: 0, y: 0, ...position },
        port.name,
        port.direction,
        layout,
      );
      const pin = svgElement("rect", {
        class: `graph-port graph-port-${port.direction}`,
        x: anchor.x - position.x - 4,
        y: anchor.y - position.y - 4,
        width: "8",
        height: "8",
      });
      const portTitle = svgElement("title");
      const drained = port.direction === "output"
        && drainedOutputs.has(`${node.id}\u0000${port.name}`);
      const commands = (topology.commands || []).filter((command) =>
        command.source_owner === node.path && command.source_port === port.name);
      portTitle.textContent = commands.length
        ? `${port.direction} port ${port.name} · ${commands.map((command) => `${command.command} on ${command.owner || "App"}`).join("; ")} · manager command, not a spatial edge`
        : drained
        ? `output port ${port.name} · drained to nothing; no downstream mailbox`
        : `${port.direction} port ${port.name}`;
      pin.append(portTitle);
      pin.dataset.port = port.name;
      attachTooltip(pin, portDescription(topology, node, port));
      const inspectPort = event => {
        const delivery = (currentSnapshot?.history?.deliveries || []).findLast(delivery => delivery.node === node.path && delivery.port === port.name);
        if (delivery) { event.stopPropagation(); selectEnvelope(delivery.envelope_id); }
      };
      pin.addEventListener('click', inspectPort);
      pin.addEventListener('keydown', event => { if (event.key === 'Enter') { event.preventDefault(); inspectPort(event); } });
      const portLabel = svgElement("text", {
        class: "graph-node-port-label",
        x: port.direction === "input" ? "-8" : String(position.width + 8),
        y: anchor.y - position.y - 5,
        "text-anchor": port.direction === "input" ? "end" : "start",
      });
      portLabel.textContent = port.display_label || port.name;
      group.append(pin, portLabel);
    }
    nodeLayer.append(group);
  }
  svg.append(nodeLayer);
  const historyWatermark = document.createElement("span");
  historyWatermark.className = "history-watermark";
  historyWatermark.textContent = "HISTORY";
  const panLayer = document.createElement("div");
  panLayer.className = "graph-pan-layer";
  panLayer.append(svg);
  const viewportControls = installGraphViewport(canvas, panLayer);
  canvas.append(panLayer, historyWatermark, viewportControls);
  canvas.addEventListener("click", (event) => {
    if (event.target === canvas || event.target === panLayer || event.target === svg) {
      selectTopologyItem(null);
    }
  });

  if (selectedTopologyItem && !selectedTopologyItemView(topology)) {
    selectedTopologyItem = null;
  }
  if (selectedMailboxRoute && !(topology.edges || []).some((edge) => edge.route === selectedMailboxRoute)) {
    selectedMailboxRoute = null;
  }

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
  evidence.textContent = "≡ queued messages · ⧉ open instances · dashed = queued route";
  legend.append(evidence);

  target.className = "topology";
  target.replaceChildren(activityPanel, canvas, legend);
  keepGraphTextAtInterfaceSize(svg, layout.width);
  if (focusKey) target.querySelector(`[data-focus-key="${CSS.escape(focusKey)}"]`)?.focus({ preventScroll: true });
}

function selectTopologyItem(id) {
  scenarioDefinitionActive = false;
  selectedMailboxRoute = null;
  selectedLoopInstance = null;
  selectedTopologyItem = selectedTopologyItem === id ? null : id;
  selectedSourceDefinition = selectedTopologyItemView(currentSnapshot?.topology)?.definition || null;
  sourceSelectionPending = Boolean(selectedSourceDefinition);
  if (activeSourceView !== "graph" && selectedSourceDefinition) activeSourceView = String(selectedSourceDefinition.source_index);
  renderSource(currentSnapshot);
  renderTopology(currentSnapshot?.topology, false);
  if (currentSnapshot) renderRelatedPanels(currentSnapshot);
}

function inspectMailbox(route) {
  scenarioDefinitionActive = false;
  selectedTopologyItem = null;
  selectedMailboxRoute = route;
  mailboxVisibleLimit = 100;
  renderTopology(currentSnapshot?.topology, false);
  if (currentSnapshot) renderRelatedPanels(currentSnapshot);
  activateInspectorTab("mailbox");
}

function activateInspectorTab(name, focus = false) {
  activeInspectorTab = name;
  for (const button of document.querySelectorAll("[data-inspector-tab]")) {
    const selected = button.dataset.inspectorTab === name;
    button.setAttribute("aria-selected", String(selected));
    button.tabIndex = selected ? 0 : -1;
    document.querySelector(`#panel-${button.dataset.inspectorTab}`).hidden = !selected;
    if (selected && focus) button.focus();
  }
}

function selectedTopologyItemView(topology) {
  if (!selectedTopologyItem || !topology) return null;
  const owner = (topology.owners || []).find((item) => item.id === selectedTopologyItem);
  if (owner) return { ...owner, kind: "graph" };
  return (topology.nodes || []).find((item) => item.id === selectedTopologyItem) || null;
}

function selectedTopologyContext(topology) {
  if (!selectedTopologyItem || !topology) return null;
  const owner = (topology.owners || []).find((item) => item.id === selectedTopologyItem);
  if (owner) {
    const ownerIds = new Set([owner.id]);
    let changed = true;
    while (changed) {
      changed = false;
      for (const candidate of topology.owners || []) {
        if (!ownerIds.has(candidate.id) && ownerIds.has(candidate.parent)) {
          ownerIds.add(candidate.id);
          changed = true;
        }
      }
    }
    const paths = new Set([
      ...(topology.owners || [])
        .filter((item) => ownerIds.has(item.id))
        .map((item) => item.path || ""),
      ...(topology.nodes || [])
        .filter((item) => ownerIds.has(item.owner))
        .map((item) => item.path || ""),
    ]);
    return {
      kind: "graph",
      path: owner.path || "",
      label: `Graph ${owner.label || "App"}`,
      all: owner.parent === null,
      paths,
    };
  }
  const node = (topology.nodes || []).find((item) => item.id === selectedTopologyItem);
  if (!node) return null;
  return {
    kind: "node",
    path: node.path || "",
    label: `Node ${node.path || node.label || "?"}`,
    all: false,
    paths: new Set([node.path || ""]),
  };
}

function pathMatchesSelection(path, selection) {
  if (!selection) return true;
  if (selection.all) return true;
  if (typeof path !== "string") return false;
  return selection.paths.has(path);
}

function rowMatchesSelection(row, selection, fields) {
  return !selection || fields.some((field) => pathMatchesSelection(row?.[field], selection));
}

function envelopePanel(snapshot) {
  const panel = document.createElement("aside");
  panel.className = "envelope-inspector";
  panel.setAttribute("aria-label", "Selected envelope");
  const nextInjection = snapshot?.scenario?.current?.next_injection;
  const deliveries = snapshot?.history?.deliveries || [];
  const sources = snapshot?.history?.sources || [];
  const outputs = snapshot?.history?.outputs || [];
  const delivery = selectedEnvelopeId
    ? deliveries.find(row => row.envelope_id === selectedEnvelopeId) || sources.find(row => row.envelope_id === selectedEnvelopeId) || outputs.find(row => row.envelope_id === selectedEnvelopeId)
    : deliveries.at(-1) || sources.at(-1);
  const heading = document.createElement("div");
  heading.className = "envelope-heading";
  const identity = document.createElement("div");
  const position = document.createElement("p");
  position.className = "envelope-position";
  const title = document.createElement("h3");
  identity.append(position, title);
  heading.append(identity);
  panel.append(heading);

  if (selectedEnvelopeId && !delivery) {
    const sysFields = (snapshot.history?.envelopes || []).find(envelope => envelope.envelope_id === selectedEnvelopeId);
    position.textContent = `Transition ${snapshot.timeline.boundary}`;
    title.textContent = 'Recorded causal envelope';
    panel.append(identifier(selectedEnvelopeId, 'envelope'));
    if (sysFields) panel.append(envelopeZones({ sys: { id: sysFields.envelope_id, origin_id: sysFields.origin_id, parent_id: sysFields.parent_id } }, { content: 'No DATA observation is recorded for this envelope at this transition.' }));
    else panel.append(document.createTextNode('This envelope is outside the selected journal prefix.'));
    return panel;
  }

  if (!selectedEnvelopeId && ["before-app-input", "before-source-output"].includes(nextInjection?.position)) {
    position.textContent = "Next authored input";
    title.textContent = `${nextInjection.connector || "App"}.${nextInjection.port}`;
    const badges = document.createElement("div");
    badges.className = "envelope-badges";
    const type = document.createElement("span");
    type.textContent = nextInjection.type || "unknown";
    badges.append(type);
    panel.append(badges);
    panel.append(envelopeZones(nextInjection.meta, nextInjection.data));
    return panel;
  }

  if (delivery) {
    position.textContent = selectedEnvelopeId ? "Selected envelope" : "Latest observed envelope";
    title.textContent = `${delivery.node || "App"}.${delivery.port || "?"}`;
    const badges = document.createElement("div");
    badges.className = "envelope-badges";
    for (const [name, id] of [["id", delivery.envelope_id], ["origin", delivery.origin_id]]) {
      if (id) badges.append(identifier(id, name, () => selectEnvelope(id)));
    }
    panel.append(badges, envelopeZones(delivery.meta, delivery.data));
    if (selectedEnvelopeId) {
      const ids = lineage();
      const commands = (snapshot.history?.commands || []).filter(command => ids.has(command.cause));
      if (commands.length) {
        const title = document.createElement('h3'); title.textContent = 'Causal commands'; panel.append(title);
        for (const command of commands) {
          const row = document.createElement('div'); row.className = 'causal-command';
          row.append(identifier(command.reference, `${command.kind} command`));
          for (const [field, label] of [['produced', 'CommandProduced'], ['consumed', 'CommandConsumed'], ['admitted', 'Admit']]) {
            if (command[field] == null) continue;
            const link = document.createElement('button');
            link.textContent = `${label} · record ${command[field]} · transition ${boundaryForRecord(command[field])}`;
            link.addEventListener('click', () => showRecordBoundary(command[field]).catch(showViewError));
            row.append(link);
          }
          panel.append(row);
        }
      }
    }
    return panel;
  }

  position.textContent = `Transition ${snapshot?.timeline?.boundary || 0}`;
  title.textContent = "No envelope delivery observed yet";
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
    if (value !== null && value !== undefined) appendDataTree(zone, value);
    zones.append(zone);
  }
  return zones;
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
  const extensions = [...new Set(item.extensions || [])];
  for (const label of [extensions.includes(item.type) ? null : item.type, ...extensions.map((extension) => `+ ${extension}`)]) {
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
  const record = decisionRecord(item.path);
  const decision = record?.decision;
  let sourceArms = item.definition.decisions?.find(handler => handler.trigger === decision?.trigger)?.arms || [];
  const highlights = new Set();
  const explanations = document.createElement("div");
  explanations.className = "decision-explanation";
  if (decision?.unavailable) explanations.textContent = decision.unavailable;
  for (const step of decision?.steps || []) {
    const selected = step.kind === "closed"
      ? sourceArms.find(arm => arm.cases.includes(step.case)) || sourceArms.find(arm => arm.fallback)
      : step.selected === null ? sourceArms.find(arm => arm.fallback) : sourceArms.filter(arm => !arm.fallback)[step.selected];
    const summary = document.createElement("p");
    summary.textContent = step.kind === "closed" ? `Selected case: ${step.case}` : step.selected === null ? "Selected fallback: _" : `Selected predicate ${step.selected + 1}`;
    explanations.append(summary);
    for (const predicate of step.predicates || []) {
      const line = document.createElement("p");
      line.className = predicate.value ? "predicate-true" : "predicate-false";
      line.textContent = `${sourceArms.filter(arm => !arm.fallback)[predicate.index]?.predicate || `Predicate ${predicate.index + 1}`} = ${predicate.value}`;
      explanations.append(line);
    }
    if (selected) {
      for (let line = selected.lines[0]; line <= selected.lines[1]; line++) {
        if (!selected.children.some(child => line >= child.lines[0] && line <= child.lines[1])) highlights.add(line);
      }
      sourceArms = selected.children;
    }
  }
  item.definition.code.split("\n").forEach((text, index) => {
    const line = document.createElement("span");
    const number = Number(item.definition.first_line) + index;
    line.className = `source-line${highlights.has(number) ? " fired" : ""}`;
    line.dataset.line = String(number);
    line.textContent = text;
    pre.append(line);
  });
  panel.append(heading, badges);
  if (record && decision) {
    const label = document.createElement("p");
    label.textContent = `Transaction · record ${record.record_index} · transition ${boundaryForRecord(record.record_index)}`;
    panel.append(label, explanations);
  }
  panel.append(location, pre);
  const commands = (currentSnapshot?.topology?.commands || []).filter((command) =>
    command.source_owner === item.path || command.owner === item.path);
  if (commands.length) {
    const heading = document.createElement("h3");
    heading.textContent = "Manager commands · not spatial edges";
    panel.append(heading);
    for (const command of commands) {
      const line = document.createElement("p");
      const cases = command.cases.length ? ` [${command.cases.join(" | ")}]` : "";
      line.textContent = `${command.source_owner || "App"}.${command.source_port}${cases} → ${command.owner || "App"}.${command.command}`;
      panel.append(line);
    }
  }
  return panel;
}

function decisionRecord(owner) {
  const records = currentSnapshot?.history?.records || [];
  return selectedRecordIndex === null
    ? records.findLast(record => record.owner === owner && record.decision)
    : records.find(record => record.record_index === selectedRecordIndex && record.owner === owner);
}

function renderAssertion(snapshot) {
  const target = document.querySelector('#assertion');
  const expectation = snapshot.scenario?.current?.expectation;
  if (!expectation) { empty(target, 'No assertion at this transition. Select an expect action in the scenario.'); return; }
  target.className = '';
  target.replaceChildren();
  const heading = document.createElement('h3');
  heading.textContent = expectation.failed ? 'Assertion failed' : 'Assertion passed';
  target.append(heading);
  const key = `${snapshot.timeline.session}:${snapshot.timeline.boundary}`;
  if (expectation.failed && explainedFailure !== key) { explainedFailure = key; activateInspectorTab('assertion'); }
  if (expectation.redacted) { target.append(document.createTextNode(expectation.reason)); return; }
  const scenario = snapshot.scenario.current;
  const definition = scenario.definition;
  const line = definition?.action_lines?.[Number(scenario.next_action_index) - 1];
  const nextLine = definition?.action_lines?.[Number(scenario.next_action_index)];
  if (line && definition.code) {
    const source = document.createElement('pre');
    source.textContent = definition.code.split('\n').slice(line - 1, nextLine ? nextLine - 1 : undefined).join('\n');
    target.append(source);
  }
  const summary = document.createElement('p');
  summary.textContent = `${expectation.target} · ${expectation.observed} observed · ${expectation.matched ?? 'unknown'} matching · expected ${expectation.quantifier}`;
  target.append(summary);
  for (const envelope of expectation.envelopes) {
    const row = document.createElement('div');
    row.className = 'assertion-envelope';
    row.append(identifier(envelope.envelope_id, 'envelope', () => selectEnvelope(envelope.envelope_id)));
    for (const comparison of envelope.comparisons) {
      const difference = document.createElement('div');
      difference.className = comparison.held ? '' : 'assertion-difference';
      const label = document.createElement('p');
      label.textContent = `${comparison.field} ${comparison.operator} · ${comparison.held ? 'matches' : 'differs'} (observed / expected)`;
      difference.append(label);
      appendDataTree(difference, { observed: comparison.actual.content, expected: comparison.expected.content });
      row.append(difference);
    }
    appendDataTree(row, envelope.data);
    target.append(row);
  }
}

async function renderComparison() {
  const request = ++comparisonRequest;
  const target = document.querySelector('#comparison');
  const mode = document.querySelector('#compare-mode').value;
  const snapshot = currentSnapshot;
  if (!snapshot) return;
  const loopOwners = new Set((Array.isArray(snapshot.loop_runs) ? snapshot.loop_runs : []).map(run => run.owner));
  const options = mode === 'sessions'
    ? (timelineIndex?.sessions || []).map(session => ({ value: String(session.session), label: sessionName(session.session) }))
    : (snapshot.instances || []).filter(instance => loopOwners.has(instance.owner)).map(instance => ({ value: JSON.stringify(instance.reference), label: `${instance.owner || 'App'} · ${instance.label} · gen ${instance.reference.generation}` }));
  for (const [index, id] of ['compare-left', 'compare-right'].entries()) {
    const select = document.getElementById(id);
    const old = select.value;
    select.replaceChildren(...options.map(option => { const element = document.createElement('option'); element.value = option.value; element.textContent = option.label; return element; }));
    select.value = options.some(option => option.value === old) ? old : options[index]?.value || options[0]?.value || '';
  }
  const rightSelect = document.querySelector('#compare-right');
  if (rightSelect.value === document.querySelector('#compare-left').value && options.length > 1) {
    rightSelect.value = options.find(option => option.value !== rightSelect.value).value;
  }
  const left = document.querySelector('#compare-left').value;
  const right = document.querySelector('#compare-right').value;
  if (!left || !right || left === right) { empty(target, 'Select two distinct Loop instances or recorded sessions.'); return; }
  let snapshots = [snapshot, snapshot];
  if (mode === 'sessions') {
    try {
      snapshots = await Promise.all([left, right].map(async session => {
        const frame = timelineFrames(session).find(frame => Number(frame.boundary) === Number(snapshot.timeline.boundary));
        if (!frame) return null;
        const response = await fetch(`/api/snapshot?session=${encodeURIComponent(session)}&boundary=${frame.boundary}`);
        if (!response.ok) throw new Error('A comparison frame is no longer available.');
        return response.json();
      }));
    } catch (error) { if (request === comparisonRequest) empty(target, error.message); return; }
  }
  if (request !== comparisonRequest) return;
  target.className = '';
  target.replaceChildren();
  const heading = document.createElement('p');
  heading.textContent = `Aligned at transition ${snapshot.timeline.boundary} · differences are values, not a replay verdict`;
  target.append(heading);
  const columns = snapshots.map((value, index) => {
    if (!value) return null;
    const reference = mode === 'instances' ? JSON.parse(index === 0 ? left : right) : null;
    const instances = (value.instances || []).filter(instance => !reference || sameInstance(instance.reference, reference));
    return { logical_time: formatLogicalTime(value.execution.logical_time_ns), instances: instances.map(instance => ({ owner: instance.owner || 'App', key: instance.label, phase: instance.phase, state: instance.state })),
      loops: (Array.isArray(value.loop_runs) ? value.loop_runs : []).filter(run => !reference || sameInstance(run.instance, reference)).map(run => ({ owner: run.owner || 'App', application: run.application, remaining: run.remaining, phase: run.phase, due: run.due_at_ns == null ? null : formatLogicalTime(run.due_at_ns) })) };
  });
  for (const field of ['logical_time', 'instances', 'loops']) {
    const row = document.createElement('div');
    row.className = `comparison-row${JSON.stringify(columns[0]?.[field]) !== JSON.stringify(columns[1]?.[field]) ? ' different' : ''}`;
    for (const column of columns) {
      const cell = document.createElement('div');
      const label = document.createElement('h4'); label.textContent = field.replaceAll('_', ' '); cell.append(label);
      if (column) appendDataTree(cell, column[field]); else cell.append(document.createTextNode('This transition is not recorded in this session.'));
      row.append(cell);
    }
    target.append(row);
  }
}

function lineage() {
  const result = new Set();
  if (!selectedEnvelopeId) return result;
  const envelopes = currentSnapshot?.history?.envelopes || [];
  const selected = envelopes.find(envelope => envelope.envelope_id === selectedEnvelopeId);
  const origin = selected?.origin_id;
  for (const envelope of envelopes) {
    if (envelope.envelope_id === selectedEnvelopeId || (origin && envelope.origin_id === origin)) result.add(envelope.envelope_id);
  }
  return result;
}

function lineageRecord(record, ids) {
  return (record.envelope_ids || []).some(id => ids.has(id))
    || (record.commands_produced || []).some(command => ids.has(command.cause));
}

const SVG_NAMESPACE = "http://www.w3.org/2000/svg";

function svgElement(name, attributes = {}) {
  const element = document.createElementNS(SVG_NAMESPACE, name);
  for (const [key, value] of Object.entries(attributes)) {
    element.setAttribute(key, String(value));
  }
  return element;
}

function itemDescriptor(item) {
  return [item.type || item.kind, ...(item.extensions || []).map((extension) => `+${extension}`)]
    .filter(Boolean)
    .join(" · ");
}

function scopeDescriptor(owner) {
  const base = owner.parent === null ? "App" : (owner.type || "Graph");
  return [base, ...(owner.extensions || []).filter((extension) => extension !== base)].join(" + ");
}

function sameInstance(left, right) {
  return left && right && left.address === right.address && String(left.generation) === String(right.generation);
}

function loopRunsFor(item) {
  return (Array.isArray(currentSnapshot?.loop_runs) ? currentSnapshot.loop_runs : [])
    .filter((run) => run.owner === (item.path || "")
      && (!selectedLoopInstance || selectedLoopInstance.owner !== run.owner || sameInstance(run.instance, selectedLoopInstance.reference)));
}

function appendLoopDial(label, item) {
  if (!(item.extensions || []).includes("Loop")) return;
  const runs = loopRunsFor(item).filter((run) => run.active);
  const admitted = runs.reduce((max, run) => {
    const value = BigInt(run.application || "0");
    return value > max ? value : max;
  }, 0n);
  const dial = svgElement("tspan", { class: "graph-loop-dial", "data-owner": item.path || "" });
  const fuel = item.loop?.fuel;
  const waiting = runs.filter(run => run.waiting);
  const due = waiting.length ? waiting.reduce((a, b) => BigInt(a.due_at_ns) < BigInt(b.due_at_ns) ? a : b).due_at_ns : null;
  const delay = due === null ? "" : ` ⏱ ${formatLogicalTime(BigInt(due) - BigInt(currentSnapshot.execution.logical_time_ns))}`;
  const value = `⟳ ${admitted}${fuel == null ? "" : `/${fuel}`}${runs.length > 1 ? ` ·${runs.length}` : ""}${delay}`;
  dial.textContent = `  ${value}`;
  dial.setAttribute("aria-label", `Loop: ${admitted} admitted applications; ${runs.length} active runs`);
  const title = svgElement("title");
  title.textContent = runs.length ? runs.map((run) =>
    `${String(run.reference).slice(0, 8)} · ${run.remaining}/${run.fuel} applications remaining · ${run.waiting ? `waiting until ${formatLogicalTime(run.due_at_ns)}` : "no pending delay"} · exhaustion: ${run.expired_port || "proved impossible"}`
  ).join("\n") : "No active Loop run · 0 admitted applications";
  dial.append(title);
  attachTooltip(dial, title.textContent);
  label.append(dial);
}

function commandPortLabel(topology, item, port) {
  if (port.direction !== "output") return port.name;
  const commands = (topology.commands || []).filter((command) =>
    command.source_owner === (item.path || "") && command.source_port === port.name);
  const spatial = (topology.edges || []).some((edge) =>
    edge.source?.id === item.id && edge.source?.port === port.name);
  if (!spatial && commands.length === 1 && !commands[0].cases?.length) {
    return `${port.name} ⇢ ${commands[0].command}`;
  }
  return port.name;
}

function appendInstanceBadge(group, instances, right, top, itemId) {
  if (!instances.length) return;
  const width = 40;
  const badge = svgElement("g", {
    class: "graph-instance-badge",
    transform: `translate(${right - width} ${top})`,
    role: "button",
    tabindex: "0",
  });
  badge.dataset.focusKey = `instances:${itemId}`;
  const title = svgElement("title");
  const description = `${instances.length} open logical instance${instances.length === 1 ? "" : "s"}`;
  title.textContent = description;
  badge.setAttribute("aria-label", `${description}. Inspect these instances.`);
  const inspectInstances = (event) => {
    event.stopPropagation();
    selectedTopologyItem = itemId;
    selectedLoopInstance = null;
    renderTopology(currentSnapshot?.topology, false);
    if (currentSnapshot) renderRelatedPanels(currentSnapshot);
    activateInspectorTab("instances");
  };
  badge.addEventListener("click", inspectInstances);
  badge.addEventListener("keydown", (event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      inspectInstances(event);
    }
  });
  badge.append(title, svgElement("rect", { width, height: "17", rx: "2" }));
  const label = svgElement("text", { x: String(width / 2), y: "12", "text-anchor": "middle" });
  label.textContent = `⧉ ${instances.length}`;
  badge.append(label);
  group.append(badge);
}

function truncate(value, maximum) {
  const text = String(value || "");
  return text.length <= maximum ? text : `${text.slice(0, maximum - 1)}…`;
}

function renderHistory(history, selection) {
  const target = document.querySelector("#history");
  const allRecords = history?.records || [];
  const allDeliveries = history?.deliveries || [];
  const records = allRecords.filter((row) => rowMatchesSelection(row, selection, ["owner", "connector"]));
  const deliveries = allDeliveries.filter((row) => rowMatchesSelection(row, selection, ["node"]));
  const suffix = selection ? ` Filtered by ${selection.label}.` : "";
  text("history-summary", `${records.length} causal record${records.length === 1 ? "" : "s"}; ${deliveries.length} envelope observation${deliveries.length === 1 ? "" : "s"}.${suffix}`);
  if (!records.length && !deliveries.length) {
    empty(target, selection ? "No recorded activity for this selection." : "No committed records.");
    document.querySelector("#show-older").hidden = true;
    return;
  }
  const start = Math.max(0, records.length - historyLimit);
  const fragment = document.createDocumentFragment();
  fragment.append(historyTable("Causal records", records.slice(start), ["record_index", "record_type", "summary", "trigger", "origin_id"], true));
  fragment.append(historyTable("Observed envelopes", deliveries.slice(Math.max(0, deliveries.length - historyLimit)), ["index", "node", "port", "envelope_id", "meta", "data"], false));
  target.className = "history-stack";
  target.replaceChildren(fragment);
  const older = document.querySelector("#show-older");
  older.hidden = start === 0;
  older.textContent = `Show ${Math.min(100, start)} older records`;
}

function historyTable(label, rows, columns, open) {
  const details = document.createElement("details");
  details.open = open;
  const summary = document.createElement("summary");
  summary.textContent = `${label} (${rows.length} shown)`;
  const table = document.createElement("table");
  table.className = label === "Causal records" ? "causal-history-table" : "delivery-history-table";
  const head = document.createElement("thead");
  const header = document.createElement("tr");
  for (const column of columns) {
    const cell = document.createElement("th");
    cell.scope = "col";
    cell.textContent = column === "record_index" ? "record · transition" : ["meta", "data"].includes(column)
      ? column.toUpperCase()
      : column.replaceAll("_", " ");
    header.append(cell);
  }
  head.append(header);
  const body = document.createElement("tbody");
  for (const row of rows) {
    const line = document.createElement("tr");
    if (Object.hasOwn(row, "record_index")) {
      line.className = "timeline-record";
      line.classList.toggle("causal-related", lineageRecord(row, lineage()));
      line.dataset.record = String(row.record_index);
      line.tabIndex = 0;
      line.setAttribute("role", "button");
      line.title = `View the system at causal record R${row.record_index}`;
      const openRecord = () => { selectedRecordIndex = row.record_index; return showRecordBoundary(row.record_index)
        .then(() => { const item = currentSnapshot?.topology?.nodes.find(item => item.path === row.owner); if (item) selectedTopologyItem = item.id; renderRelatedPanels(currentSnapshot); })
        .catch((error) => setCommandStatus(`Historical view unavailable: ${error.message}`));
      };
      line.addEventListener("click", openRecord);
      line.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          openRecord();
        }
      });
    }
    for (const column of columns) {
      const cell = document.createElement("td");
      if (["meta", "data"].includes(column)) {
        const payload = document.createElement("details");
        const summary = document.createElement("summary");
        summary.textContent = column === "data" ? row.data?.kind || "DATA" : "META";
        payload.append(summary);
        appendDataTree(payload, row[column]);
        cell.append(payload);
      } else if (column === "record_index") {
        const boundary = boundaryForRecord(row[column]);
        appendValue(cell, boundary === null ? `record ${row[column]}` : `record ${row[column]} · transition ${boundary}`);
      } else if (["origin_id", "envelope_id"].includes(column) && typeof row[column] === "string") {
        cell.append(identifier(row[column], "", () => selectEnvelope(row[column])));
      } else {
        appendValue(cell, row[column]);
        if (typeof row[column] === "string") cell.title = row[column];
      }
      line.append(cell);
    }
    body.append(line);
  }
  table.append(head, body);
  details.append(summary, table);
  return details;
}

async function showRecordBoundary(recordIndex) {
  const session = Number(currentSnapshot?.timeline?.session || 0);
  const boundary = boundaryForRecord(recordIndex);
  const frame = timelineFrames(session).find((candidate) => Number(candidate.boundary) === boundary);
    if (!frame) throw new Error(`no recorded transition contains record ${recordIndex}`);
  clearPlaybackTimer();
  bufferedPlayback = false;
  replayPaused = false;
  await showBoundary(session, frame.boundary, true);
}

function boundaryForRecord(recordIndex) {
  const session = Number(currentSnapshot?.timeline?.session || 0);
  const frame = timelineFrames(session).find((candidate) =>
    candidate.record_through !== null
      && candidate.record_through !== undefined
      && Number(candidate.record_through) >= Number(recordIndex));
  return frame ? Number(frame.boundary) : null;
}

function renderInjections(snapshot, selection) {
  const target = document.querySelector("#injections");
  const roots = new Map();
  for (const delivery of snapshot?.history?.deliveries || []) {
    const origin = delivery?.origin_id || delivery?.meta?.sys?.origin_id;
    if (!origin || delivery?.meta?.sys?.parent_id !== null || roots.has(origin)) continue;
    roots.set(origin, delivery);
  }
  const rows = [...roots.entries()].filter(([, delivery]) =>
    rowMatchesSelection(delivery, selection, ["node"]));
  if (!rows.length) {
    empty(target, selection ? "No admitted injection for this selection." : "No admitted injection.");
    return;
  }
  const session = Number(snapshot?.timeline?.session || 0);
  const table = document.createElement("table");
  const header = document.createElement("tr");
  for (const label of ["Routed instance", "Logical time", "Transition", "Session"]) {
    const cell = document.createElement("th");
    cell.textContent = label;
    header.append(cell);
  }
  const head = document.createElement("thead");
  head.append(header);
  const body = document.createElement("tbody");
  for (const [origin, delivery] of rows) {
    const admitted = (snapshot?.history?.records || []).find((record) =>
      record.origin_id === origin && String(record.summary || "").startsWith("Input admitted"));
    const boundary = admitted ? boundaryForRecord(admitted.record_index) : null;
    const line = document.createElement("tr");
    line.className = boundary === null ? "" : "timeline-record";
    if (boundary !== null) {
      line.tabIndex = 0;
      line.setAttribute("role", "button");
      const open = () => showBoundary(session, boundary, true)
        .catch((error) => setCommandStatus(`Injection history unavailable: ${error.message}`));
      line.addEventListener("click", open);
      line.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          open();
        }
      });
    }
    const logical = delivery?.meta?.sys?.logical_time_ns
      ?? delivery?.meta?.sys?.timestamp_ns
      ?? null;
    const source = (snapshot?.history?.sources || []).find(source => source.envelope_id === origin);
    const values = [source?.instance_labels?.join(" · ") || "no spatial target", logical === null ? "…" : formatLogicalTime(logical), boundary === null ? "—" : String(boundary), sessionName(session)];
    const cells = [];
    for (const value of values) {
      const cell = document.createElement("td");
      cell.textContent = String(value);
      line.append(cell);
      cells.push(cell);
    }
    if (logical === null && boundary !== null) {
      fetch(`/api/snapshot?session=${encodeURIComponent(session)}&boundary=${encodeURIComponent(boundary)}`, { cache: "no-store" })
        .then((response) => response.ok ? response.json() : null)
        .then((frame) => { cells[1].textContent = frame ? formatLogicalTime(frame.execution?.logical_time_ns) : "—"; })
        .catch(() => { cells[1].textContent = "—"; });
    }
    body.append(line);
  }
  table.append(head, body);
  target.className = "injection-history";
  target.replaceChildren(table);
}

function renderInstances(instances, selection) {
  const target = document.querySelector("#state");
  const visible = (instances || []).filter((instance) =>
    rowMatchesSelection(instance, selection, ["owner", "node"]));
  if (!visible.length) {
    empty(target, selection ? "No active instance for this selection." : "No active instance.");
    return;
  }
  target.className = "card-stack";
  target.replaceChildren(...visible.map((instance) => {
    const card = document.createElement("article");
    card.className = "data-card";
    const heading = document.createElement("h3");
    const select = document.createElement("button");
    select.type = "button";
    select.textContent = instance.owner || "App";
    select.title = "Filter Loop runs, Retry operations and the iteration dial by this instance";
    select.setAttribute("aria-pressed", String(Boolean(sameInstance(instance.reference, selectedLoopInstance?.reference))));
    select.addEventListener("click", () => {
      selectedLoopInstance = sameInstance(instance.reference, selectedLoopInstance?.reference)
        ? null : { owner: instance.owner, reference: instance.reference };
      renderTopology(currentSnapshot?.topology, false);
      renderRelatedPanels(currentSnapshot);
    });
    heading.append(select);
    const meta = document.createElement("p");
    meta.className = "card-meta";
    meta.textContent = `${instance.label || instance.owner || "App"} · gen ${instance.reference?.generation ?? "?"} · ${instance.phase}`;
    const address = document.createElement("details");
    address.className = "instance-address";
    const summary = document.createElement("summary");
    summary.textContent = "Instance address";
    const full = document.createElement("code");
    full.textContent = instance.reference?.address || "?";
    address.append(summary, full, identifier(instance.reference?.address || "?", "address"));
    card.append(heading, meta, address);
    appendValue(card, instance.state);
    return card;
  }));
}

function renderLoopRuns(runs, selection) {
  const target = document.querySelector("#loop-runs");
  if (!Array.isArray(runs)) { empty(target, runs?.error || "No recorded Loop run."); return; }
  const visible = runs.filter((run) => rowMatchesSelection(run, selection, ["owner"])
    && (!selectedLoopInstance || (run.owner === selectedLoopInstance.owner && sameInstance(run.instance, selectedLoopInstance.reference))));
  if (!visible.length) { empty(target, "No recorded Loop run for this selection."); return; }
  const table = document.createElement("table");
  const header = document.createElement("tr");
  for (const name of ["Run", "Application", "Remaining", "Due", "State"]) {
    const cell = document.createElement("th"); cell.textContent = name; header.append(cell);
  }
  const head = document.createElement("thead"); head.append(header); table.append(head);
  const body = document.createElement("tbody");
  for (const run of visible) {
    const row = document.createElement("tr");
    for (const value of [String(run.reference).slice(0, 8), run.application, `${run.remaining}/${run.fuel}`, formatLogicalTime(run.due_at_ns), run.phase]) {
      const cell = document.createElement("td"); cell.textContent = value; row.append(cell);
    }
    row.firstChild.title = run.reference;
    row.firstChild.replaceChildren(identifier(run.reference, 'Loop run'));
    body.append(row);
  }
  table.append(body); target.className = "table-scroll"; target.replaceChildren(table);
}

function renderExchanges(exchanges, selection) {
  const target = document.querySelector("#exchanges");
  const calls = (exchanges?.calls || []).filter((call) =>
    rowMatchesSelection(call, selection, ["connector"]));
  const sessions = (exchanges?.sessions || []).filter((session) =>
    rowMatchesSelection(session, selection, ["connector"]));
  if (!calls.length && !sessions.length) {
    empty(target, selection ? "No open exchange for this selection." : "No open exchange.");
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
    meta.textContent = `${exchange.direction}; ${exchange.completion_recorded ? "terminal outcome recorded" : "open"}`;
    card.append(heading, meta);
    return card;
  }));
}

function renderRetryOperations(operations, selection) {
  const target = document.querySelector("#retry-operations");
  const visible = (operations || []).filter(operation => rowMatchesSelection(operation, selection, ["owner"])
    && (!selectedLoopInstance || (operation.owner === selectedLoopInstance.owner && sameInstance(operation.instance, selectedLoopInstance.reference))));
  if (!visible.length) { empty(target, "No recorded Retry operation for this selection."); return; }
  target.className = "card-stack";
  target.replaceChildren(...visible.map(operation => {
    const card = document.createElement("article");
    card.className = "data-card retry-operation";
    const title = document.createElement("h3");
    title.append(`${operation.owner} · `, identifier(operation.reference, "operation"));
    const status = document.createElement("p");
    status.className = "card-meta";
    const phase = {
      Initial: "opened", Admitted: "admitted", CallOpen: "Call open",
      Settled: "Call closed, result pending", Held: "result held by manager",
      AwaitingAdmission: "waiting for next attempt", Closed: operation.closed === "Stopped" ? "stopped without new publication" : "final result published",
    }[operation.phase] || operation.phase;
    status.textContent = `Attempt ${operation.attempt}/${operation.limit} · ${phase}${operation.active && operation.due_at_ns != null ? ` · due ${formatLogicalTime(operation.due_at_ns)}` : ""}`;
    card.append(title, status);
    const table = document.createElement("table");
    const head = document.createElement("thead");
    const header = document.createElement("tr");
    for (const text of ["Attempt", "Terminal", "Disposition", "Evidence"]) {
      const cell = document.createElement("th"); cell.textContent = text; header.append(cell);
    }
    head.append(header); table.append(head);
    const body = document.createElement("tbody");
    for (const attempt of operation.attempts) {
      const row = document.createElement("tr");
      for (const text of [attempt.ordinal, attempt.terminal || "not yet emitted", attempt.disposition === "manager" ? "held by Retry" : attempt.disposition === "graph" ? "published" : "pending"]) {
        const cell = document.createElement("td"); cell.textContent = text; row.append(cell);
      }
      const evidence = document.createElement("td");
      if (attempt.call) evidence.append(identifier(attempt.call, "Call"));
      for (const [field, label] of [["admitted_record", "Admitted"], ["closed_record", "Call closed"], ["produced_record", "Result"], ["consumed_record", "Consumed"]]) {
        if (attempt[field] == null) continue;
        const button = document.createElement("button");
        button.type = "button";
        button.textContent = `${label} · record ${attempt[field]}`;
        button.addEventListener("click", () => showRecordBoundary(attempt[field]).catch(showViewError));
        evidence.append(button);
      }
      const emissions = attempt.emissions || [];
      if (emissions.length) {
        const details = document.createElement("details");
        const summary = document.createElement("summary"); summary.textContent = `${emissions.length} recorded envelopes`;
        details.append(summary);
        for (const id of emissions) details.append(identifier(id, "envelope", () => selectEnvelope(id)));
        evidence.append(details);
      }
      row.append(evidence); body.append(row);
    }
    table.append(body);
    const scroll = document.createElement("div"); scroll.className = "table-scroll"; scroll.append(table);
    card.append(scroll);
    return card;
  }));
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
    appendDataTree(target, value);
    return;
  }
  if (typeof value === "string" && /^[0-9a-f]{32,}$/i.test(value)) { target.append(identifier(value)); return; }
  const code = document.createElement("code");
  code.textContent = value === undefined ? "—" : String(value);
  target.append(code);
}

function appendDataTree(target, value) {
  const tree = document.createElement("div");
  tree.className = "data-tree";
  if (value !== null && typeof value === "object") {
    appendDataTreeEntries(tree, value, 0);
  } else {
    tree.append(dataTreeLeaf(null, value));
  }
  target.append(tree);
}

function appendDataTreeEntries(target, value, depth) {
  const entries = Array.isArray(value)
    ? value.map((item, index) => [`[${index}]`, item])
    : Object.entries(value);
  if (!entries.length) {
    const emptyValue = document.createElement("span");
    emptyValue.className = "data-tree-empty";
    emptyValue.textContent = Array.isArray(value) ? "empty array" : "empty object";
    target.append(emptyValue);
    return;
  }
  for (const [name, child] of entries) {
    if (child !== null && typeof child === "object") {
      const branch = document.createElement("details");
      branch.className = "data-tree-branch";
      branch.open = depth < 1;
      const summary = document.createElement("summary");
      const key = document.createElement("span");
      key.className = "data-tree-key";
      key.textContent = name;
      const size = document.createElement("span");
      size.className = "data-tree-size";
      const count = Array.isArray(child) ? child.length : Object.keys(child).length;
      size.textContent = Array.isArray(child)
        ? `${count} item${count === 1 ? "" : "s"}`
        : `${count} field${count === 1 ? "" : "s"}`;
      summary.append(key, size);
      const children = document.createElement("div");
      children.className = "data-tree-children";
      appendDataTreeEntries(children, child, depth + 1);
      branch.append(summary, children);
      target.append(branch);
    } else {
      target.append(dataTreeLeaf(name, child));
    }
  }
}

function dataTreeLeaf(name, value) {
  const row = document.createElement("div");
  row.className = "data-tree-leaf";
  if (name !== null) {
    const key = document.createElement("span");
    key.className = "data-tree-key";
    key.textContent = name;
    row.append(key);
  }
  const content = document.createElement("code");
  const kind = value === null ? "null" : typeof value;
  content.className = `data-tree-value data-tree-value-${kind}`;
  content.textContent = value === undefined ? "—" : value === null ? "null" : String(value);
  if (typeof value === "string" && /^[0-9a-f]{32,}$/i.test(value)) {
    const causal = ["id", "origin_id", "parent_id", "envelope_id"].includes(name);
    row.append(identifier(value, "", causal ? () => selectEnvelope(value) : null));
  } else row.append(content);
  return row;
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

function formatLogicalTime(value) {
  if (value === undefined || value === null) return "—";
  let nanoseconds;
  try {
    nanoseconds = BigInt(String(value));
  } catch (_) {
    return String(value);
  }
  const negative = nanoseconds < 0n;
  const absolute = negative ? -nanoseconds : nanoseconds;
  const totalSeconds = absolute / 1_000_000_000n;
  const fractionalNanoseconds = absolute % 1_000_000_000n;
  const seconds = totalSeconds % 60n;
  const minutes = (totalSeconds / 60n) % 60n;
  const hours = totalSeconds / 3_600n;
  let fraction = fractionalNanoseconds.toString().padStart(9, "0").replace(/0+$/, "");
  if (!fraction) fraction = "0";
  const secondsText = `${seconds.toString().padStart(totalSeconds >= 60n ? 2 : 1, "0")}.${fraction} s`;
  const prefix = negative ? "−" : "";
  if (hours > 0n) return `${prefix}${hours} h ${minutes.toString().padStart(2, "0")} min ${secondsText}`;
  if (totalSeconds >= 60n) return `${prefix}${totalSeconds / 60n} min ${secondsText}`;
  return `${prefix}${secondsText}`;
}

function installWorkspaceSplitter() {
  const workspace = document.querySelector(".workspace");
  const splitter = document.querySelector("#workspace-splitter");
  const stored = Number(sessionStorage.getItem("rastok-inspector-width"));
  let width = Number.isFinite(stored) && stored > 0 ? stored : 360;
  const limits = () => ({
    minimum: 280,
    maximum: Math.max(280, Math.min(720, workspace.getBoundingClientRect().width - 305)),
  });
  const apply = (candidate) => {
    const { minimum, maximum } = limits();
    width = Math.min(maximum, Math.max(minimum, candidate));
    workspace.style.setProperty("--inspector-width", `${Math.round(width)}px`);
    splitter.setAttribute("aria-valuemin", String(minimum));
    splitter.setAttribute("aria-valuemax", String(maximum));
    splitter.setAttribute("aria-valuenow", String(Math.round(width)));
    sessionStorage.setItem("rastok-inspector-width", String(Math.round(width)));
  };
  let dragging = false;
  splitter.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    dragging = true;
    workspace.classList.add("resizing");
    splitter.setPointerCapture(event.pointerId);
  });
  splitter.addEventListener("pointermove", (event) => {
    if (!dragging) return;
    const bounds = workspace.getBoundingClientRect();
    apply(bounds.right - event.clientX);
  });
  const stop = (event) => {
    if (!dragging) return;
    dragging = false;
    workspace.classList.remove("resizing");
    if (splitter.hasPointerCapture(event.pointerId)) splitter.releasePointerCapture(event.pointerId);
  };
  splitter.addEventListener("pointerup", stop);
  splitter.addEventListener("pointercancel", stop);
  splitter.addEventListener("keydown", (event) => {
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const { minimum, maximum } = limits();
    const next = event.key === "Home"
      ? minimum
      : event.key === "End"
        ? maximum
        : width + (event.key === "ArrowLeft" ? 24 : -24);
    apply(next);
  });
  window.addEventListener("resize", () => apply(width));
  apply(width);
}

installWorkspaceSplitter();

for (const button of document.querySelectorAll("button[data-command]")) {
  button.addEventListener("click", async () => {
    if (pendingCommandRevision !== null) return;
    const command = button.dataset.command;
    const session = Number(currentSnapshot?.timeline?.session || 0);
    const boundary = Number(currentSnapshot?.timeline?.boundary || 0);
    const hasReplayAhead = timelineFrames(session)
      .some((frame) => Number(frame.boundary) > boundary);
    if (liveSnapshot?.mode === "replay" && hasReplayAhead && (command === "step" || command === "run-to-completion")) {
      pauseBufferedPlayback();
      const next = command === "step"
        ? timelineFrames(session).find(frame => Number(frame.boundary) > boundary).boundary
        : liveSnapshot.timeline.boundary;
      await showBoundary(session, next);
      return;
    }
    if (command === "resume" && !sameTimelinePoint(currentSnapshot, liveSnapshot) && hasReplayAhead) {
      startBufferedPlayback();
      return;
    }
    if (command === "pause" && bufferedPlayback) {
      pauseBufferedPlayback();
      if (!(liveSnapshot?.execution?.legal_commands || []).includes("pause")) return;
    }
    // Returning to the current head via arrows is still an inspection gesture;
    // an enabled execution control explicitly resumes following the live head.
    if (sameTimelinePoint(currentSnapshot, liveSnapshot)) manualHistoricalView = false;
    if (command === "resume") {
      clearPlaybackTimer();
      manualHistoricalView = false;
      replayPaused = false;
      bufferedPlayback = true;
    } else if (command === "run-to-completion") {
      clearPlaybackTimer();
      manualHistoricalView = false;
      replayPaused = false;
      bufferedPlayback = false;
    } else if (command === "reset-scenario") {
      clearPlaybackTimer();
      manualHistoricalView = false;
      replayPaused = false;
      bufferedPlayback = false;
      selectedTopologyItem = null;
      selectedEnvelopeId = null;
      selectedRecordIndex = null;
      selectedLoopInstance = null;
      explainedFailure = null;
      selectedMailboxRoute = null;
      scenarioDefinitionActive = false;
      setCommandStatus("");
    }
    pendingCommandRevision = liveSnapshot?.revision ?? currentSnapshot?.revision ?? 0;
    commandOrigin = { command, boundary, action: currentSnapshot?.scenario?.current?.next_action_index, quiescent: currentSnapshot?.scenario?.current?.quiescent };
    updateControls();
    try {
      await post(`/api/command/${command}`);
      await refresh();
    } catch (error) {
      if (command === "resume") bufferedPlayback = false;
      pendingCommandRevision = null;
      commandOrigin = null;
      updateControls();
      setCommandStatus(`UI command refused: ${error.message}`);
    }
  });
}

document.querySelector("#take-control").addEventListener("click", () => claim(true));
for (const button of document.querySelectorAll("[data-inspector-tab]")) {
  button.addEventListener("click", () => activateInspectorTab(button.dataset.inspectorTab));
  button.addEventListener("keydown", (event) => {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const tabs = [...document.querySelectorAll("[data-inspector-tab]")];
    const current = tabs.indexOf(button);
    const target = event.key === "Home"
      ? 0
      : event.key === "End"
        ? tabs.length - 1
        : (current + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
    activateInspectorTab(tabs[target].dataset.inspectorTab, true);
  });
}
document.querySelector("#back-boundary").addEventListener("click", () => {
  const session = Number(currentSnapshot?.timeline?.session || 0);
  const boundary = Number(currentSnapshot?.timeline?.boundary || 0);
  const frames = timelineFrames(session);
  const index = frames.findIndex((frame) => Number(frame.boundary) === boundary);
  if (index <= 0) return;
  clearPlaybackTimer();
  bufferedPlayback = false;
  replayPaused = false;
  showBoundary(session, frames[index - 1].boundary, true)
    .catch((error) => setCommandStatus(`Historical view unavailable: ${error.message}`));
});
document.querySelector("#return-live").addEventListener("click", () => {
  clearPlaybackTimer();
  manualHistoricalView = false;
  bufferedPlayback = false;
  replayPaused = false;
  setCommandStatus("");
  if (liveSnapshot) renderSnapshot(liveSnapshot);
});
document.querySelector("#timeline").addEventListener("change", (event) => {
  const session = Number(currentSnapshot?.timeline?.session || 0);
  clearPlaybackTimer();
  bufferedPlayback = false;
  replayPaused = false;
  showBoundary(session, Number(event.target.value), true)
    .catch((error) => setCommandStatus(`Historical view unavailable: ${error.message}`));
});
document.querySelector("#timeline-session").addEventListener("change", (event) => {
  const session = Number(event.target.value);
  const frames = timelineFrames(session);
  const target = frames.at(-1);
  if (!target) return;
  clearPlaybackTimer();
  bufferedPlayback = false;
  replayPaused = false;
  showBoundary(session, target.boundary, true)
    .catch((error) => setCommandStatus(`Historical session unavailable: ${error.message}`));
});
document.querySelector("#playback-speed").addEventListener("change", () => {
  clearPlaybackTimer();
  if (bufferedPlayback) scheduleBufferedBoundary();
});
document.querySelector("#show-older").addEventListener("click", () => {
  historyLimit += 100;
  if (currentSnapshot) renderRelatedPanels(currentSnapshot);
});
document.querySelector("#scenario-definition").addEventListener("click", inspectScenarioDefinition);
document.querySelector("#scenario-select").addEventListener("change", () => {
  updateControls();
});
document.querySelector("#select-scenario").addEventListener("click", async () => {
  const [simulation, scenario] = document.querySelector("#scenario-select").value.split("\u0000");
  try {
    clearPlaybackTimer();
    manualHistoricalView = false;
    bufferedPlayback = false;
    replayPaused = false;
    selectedTopologyItem = null;
    selectedEnvelopeId = null;
    selectedRecordIndex = null;
    selectedLoopInstance = null;
    explainedFailure = null;
    selectedMailboxRoute = null;
    pendingCommandRevision = liveSnapshot?.revision ?? currentSnapshot?.revision ?? 0;
    updateControls();
    await post(`/api/command/select-scenario?simulation=${encodeURIComponent(simulation)}&scenario=${encodeURIComponent(scenario)}`);
  } catch (error) {
    pendingCommandRevision = null;
    updateControls();
    setCommandStatus(`Scenario selection refused: ${error.message}`);
  }
});

const events = new EventSource("/api/events");
events.addEventListener("revision", () => refresh().catch((error) => setCommandStatus(error.message)));
events.addEventListener("open", () => {
  if (!controls) claim().then(refresh).catch((error) => setCommandStatus(error.message));
});
events.onerror = () => {
  text("role", "reconnecting");
  controls = false;
  updateControls();
};
window.addEventListener("pagehide", releaseControl);

claim().then(refresh).catch((error) => setCommandStatus(error.message));

function dismissFirstContact() {
  if (!firstContact) return;
  firstContact = false;
  setCommandStatus("");
}
document.addEventListener("pointerdown", dismissFirstContact, { once: true, capture: true });
document.addEventListener("keydown", dismissFirstContact, { once: true, capture: true });

function pausePresentation() {
  clearPlaybackTimer();
  bufferedPlayback = false;
  replayPaused = false;
}

function showViewError(error) { setCommandStatus(`Historical view unavailable: ${error.message}`); }

function stepForward() {
  const session = currentSnapshot?.timeline?.session || 0;
  const next = timelineFrames(session).find(frame => Number(frame.boundary) > Number(currentSnapshot?.timeline?.boundary || 0));
  if (next) { pausePresentation(); showBoundary(session, next.boundary).catch(showViewError); }
  else document.querySelector('#step:not(:disabled)')?.click();
}

function openSearch() {
  document.querySelector('#search-panel').hidden = false;
  document.querySelector('#search').focus();
  renderSearchResults();
}

function renderSearchResults() {
  const target = document.querySelector('#search-results');
  if (!target || document.querySelector('#search-panel').hidden) return;
  const query = document.querySelector('#search').value.toLowerCase();
  target.replaceChildren();
  const results = [];
  for (const item of [...(currentSnapshot?.topology?.owners || []), ...(currentSnapshot?.topology?.nodes || [])]) {
    results.push({ label: item.path || "App", search: item.path || "App", open: () => selectTopologyItem(item.id) });
    for (const port of item.ports) results.push({ label: `${item.path || 'App'}.${port.name}`, search: `${item.path || 'App'}.${port.name}`, open: () => {
      const delivery = currentSnapshot.history.deliveries.findLast(delivery => (delivery.node || '') === item.path && delivery.port === port.name);
      if (delivery) selectEnvelope(delivery.envelope_id); else selectTopologyItem(item.id);
    } });
  }
  for (const envelope of currentSnapshot?.history?.envelopes || []) results.push({ label: `Envelope ${envelope.envelope_id.slice(0, 8)}`, search: envelope.envelope_id, open: () => selectEnvelope(envelope.envelope_id) });
  for (const result of results.filter(result => result.search.toLowerCase().includes(query)).slice(0, 50)) {
    const button = document.createElement('button');
    button.textContent = result.label;
    button.addEventListener('click', () => { document.querySelector('#search-panel').hidden = true; result.open(); });
    target.append(button);
  }
  if (!target.children.length) target.textContent = 'No matching node, port or recorded envelope.';
}

document.querySelector('#open-search').addEventListener('click', openSearch);
document.querySelector('#search').addEventListener('input', renderSearchResults);
for (const id of ['compare-mode', 'compare-left', 'compare-right']) document.getElementById(id).addEventListener('change', renderComparison);
document.addEventListener('keydown', event => {
  if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.altKey) return;
  if (event.key === 'Escape') {
    event.preventDefault();
    document.querySelector('#search-panel').hidden = true;
    document.querySelector('#graph-tooltip')?.setAttribute('hidden', '');
    document.querySelector('#future').open = false;
    selectedEnvelopeId = null; selectedRecordIndex = null;
    selectTopologyItem(null);
    (document.querySelector('#step:not(:disabled)') || document.querySelector('#open-search')).focus();
    return;
  }
  if (event.target.closest('input, select, textarea, [contenteditable=true]')) return;
  const command = name => document.querySelector(`[data-command="${name}"]:not(:disabled)`)?.click();
  switch (event.key) {
    case '/': event.preventDefault(); openSearch(); break;
    case 'ArrowLeft': event.preventDefault(); document.querySelector('#back-boundary:not(:disabled)')?.click(); break;
    case 'ArrowRight': event.preventDefault(); stepForward(); break;
    case 'n': event.preventDefault(); stepForward(); break;
    case 'a': event.preventDefault(); command('next-scenario-action'); break;
    case ' ': event.preventDefault(); command(bufferedPlayback || liveSnapshot?.execution?.state === 'running' ? 'pause' : 'resume'); break;
    case 'Enter':
      if (!event.target.closest('button, a, [role=button], summary')) { event.preventDefault(); activateInspectorTab('envelope', true); }
      break;
  }
});
