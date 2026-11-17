"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const layoutEngine = require("./graph_layout.js");
const { syntheticGraph } = require("./graph_fixture.cjs");

function owner(id, parent, ports = []) {
  return { id, parent, label: parent === null ? "App" : id, ports, extensions: [] };
}

function operator(id, ownerId, ports) {
  return {
    id,
    owner: ownerId,
    kind: "operator",
    label: id,
    path: id,
    type: "Operator",
    ports,
  };
}

function edge(index, sourceId, sourcePort, targetId, targetPort) {
  return {
    index,
    route: `${sourceId}.${sourcePort}>${targetId}.${targetPort}`,
    source: { id: sourceId, port: sourcePort, kind: sourceId.startsWith("boundary:") ? "input" : "node", owner: "owner:0" },
    target: { id: targetId, port: targetPort, kind: "node" },
    queued_count: "0",
    mailbox: [],
  };
}

function render(topology) {
  const graph = layoutEngine.materializeGraph(topology);
  const layout = layoutEngine.layoutGraph(graph.nodes, graph.edges, topology.owners);
  const routes = layoutEngine.routeGraphEdges(graph, layout);
  return { graph, layout, routes };
}

function stableProjection(result) {
  return JSON.stringify({
    positions: [...result.layout.positions.entries()],
    scopes: [...result.layout.scopes.entries()],
    nodePorts: [...result.layout.portAnchors.entries()],
    scopePorts: [...result.layout.scopePortAnchors.entries()],
    routes: [...result.routes.entries()].map(([index, route]) => [
      index,
      route.points,
      route.presentationPoints,
      route.portals,
    ]),
  });
}

function assertNoBodyCrossings({ graph, layout, routes }) {
  for (const edge of graph.edges) {
    const samples = layoutEngine.sampleCurvedCorridor(routes.get(edge.index).presentationPoints, undefined, 64);
    for (const node of graph.nodes) {
      if (node.kind === "boundary" || [edge.source.id, edge.target.id].includes(node.id)) continue;
      const box = layout.positions.get(node.id);
      assert.ok(!samples.some(point => point.x > box.x && point.x < box.x + box.width
        && point.y > box.y && point.y < box.y + box.height), `${edge.index} avoids ${node.id}`);
    }
  }
}

// A mailbox badge reserves four digits. Its box may graze a Node by less than
// the padding around those digits; anything deeper hides part of the count.
function assertBadgesClearBodies({ graph, layout, routes }, context = "") {
  for (const edge of graph.edges) {
    const badge = routes.get(edge.index).label.box;
    for (const node of graph.nodes) {
      if (node.kind === "boundary" || node.kind === "discard" || node.kind === "routing") continue;
      const box = layout.positions.get(node.id);
      const width = Math.min(badge.x + badge.width, box.x + box.width) - Math.max(badge.x, box.x);
      const height = Math.min(badge.y + badge.height, box.y + box.height) - Math.max(badge.y, box.y);
      assert.ok(Math.min(width, height) < 4,
        `${context}the mailbox badge of ${edge.route} covers ${Math.round(width)}x${Math.round(height)} px of ${node.id}`);
    }
  }
}

test("channel tracks clear wider nodes even when those nodes do not emit into the channel", () => {
  const ports = [{ name: "in", direction: "input" }, { name: "out", direction: "output" }];
  for (const mode of ["unwired", "receiver", "drained", "emitter", "fan-in"]) {
    const nodes = ["s", "a", "wide", "b", "c", "d"].map(id => operator(id, "owner:0", ports));
    Object.assign(nodes[2], { label: "partner_with_a_long_name", kind: "connector",
      type: "HttpClient", extensions: ["Retry"] });
    const links = mode === "unwired" ? [] : [edge(0, "s", "out", "a", "in"),
      edge(1, "s", "out", "wide", "in"), edge(2, "s", "out", "b", "in")];
    links.push(edge(links.length, "a", "out", "c", "in"),
      edge(links.length + 1, "b", "out", mode === "fan-in" ? "c" : "d", "in"));
    if (mode === "emitter") links.push(edge(links.length, "wide", "out", "d", "in"));
    if (mode === "drained") links.push({ ...edge(links.length, "wide", "out", "discard", null),
      target: { id: "discard", kind: "discard", owner: "owner:0" } });
    const topology = { owners: [owner("owner:0", null)], nodes, edges: links };
    const result = render(topology);
    assertNoBodyCrossings(result);
    assert.equal(stableProjection(result), stableProjection(render(topology)), mode);
  }
});

test("generated topologies route deterministically outside unrelated node bodies", () => {
  let seed = 170026;
  const random = () => {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    return seed / 4294967296;
  };
  const ports = [{ name: "in", direction: "input" }, { name: "out", direction: "output" }];
  for (let sample = 0; sample < 300; sample += 1) {
    const nodes = Array.from({ length: 6 + sample % 9 }, (_, index) => ({
      ...operator(`node${index}`, "owner:0", ports),
      label: `node${index}${"_detail".repeat(Math.floor(random() * 6))}`,
    }));
    const edges = [];
    for (let from = 0; from < nodes.length; from += 1) {
      for (let to = from + 1; to < nodes.length; to += 1) {
        if (random() < 0.2) edges.push(edge(edges.length, nodes[from].id, "out", nodes[to].id, "in"));
      }
    }
    if (sample % 3 === 0) edges.push(edge(edges.length, nodes.at(-1).id, "out", nodes[0].id, "in"));
    const topology = { owners: [owner("owner:0", null)], nodes, edges };
    const result = render(topology);
    assertNoBodyCrossings(result);
    assertBadgesClearBodies(result, `sample ${sample}: `);
    assert.equal(stableProjection(result), stableProjection(render(topology)), `sample ${sample}`);
  }
});

test("a direct link keeps room for its mailbox badge between its two columns", () => {
  const ports = (...outputs) => [{ name: "in", direction: "input" },
    ...outputs.map(name => ({ name, direction: "output" }))];
  const cases = {
    // A bent link next to an unrelated wider column.
    bent: {
      nodes: [operator("unwired", "owner:0", ports("out", "exit", "err")),
        operator("send", "owner:0", ports("out", "exit")), operator("receive", "owner:0", ports("out"))],
      links: [["send", "exit", "receive"]],
    },
    // A straight link in a channel that also holds a bend.
    straight: {
      nodes: ["first", "second", "upper", "lower", "last"].map(id => operator(id, "owner:0", ports("out"))),
      links: [["first", "out", "upper"], ["first", "out", "lower"], ["second", "out", "last"], ["lower", "out", "last"]],
    },
    // A straight link joined by three bends at its receiving pin.
    joined: {
      nodes: ["a", "b", "c", "d", "e", "f", "g", "h"].map(id => operator(id, "owner:0", ports("out"))),
      links: [["a", "out", "g"], ["a", "out", "h"], ["b", "out", "c"], ["b", "out", "d"], ["b", "out", "h"],
        ["c", "out", "h"], ["d", "out", "e"], ["e", "out", "f"], ["e", "out", "g"], ["f", "out", "h"]],
    },
  };
  for (const [name, value] of Object.entries(cases)) {
    const topology = { owners: [owner("owner:0", null)], nodes: value.nodes,
      edges: value.links.map(([from, port, to], index) => edge(index, from, port, to, "in")) };
    const result = render(topology);
    assertBadgesClearBodies(result, `${name}: `);
    assertNoBodyCrossings(result);
    assert.equal(stableProjection(result), stableProjection(render(topology)), name);
  }
});

test("shared pins meet at their height without a staircase on arrival or departure", () => {
  const ports = [{ name: "in", direction: "input" }, { name: "out", direction: "output" }];
  for (const [count, outgoing] of [[2, false], [2, true], [3, false], [4, false]]) {
    const nodes = [operator("shared", "owner:0", ports),
      ...Array.from({ length: count }, (_, index) => operator(`peer${index}`, "owner:0", ports))];
    const edges = nodes.slice(1).map((node, index) => outgoing
      ? edge(index, "shared", "out", node.id, "in") : edge(index, node.id, "out", "shared", "in"));
    const result = render({ owners: [owner("owner:0", null)], nodes, edges });
    const tracks = [];
    for (const route of result.routes.values()) {
      const vertical = layoutEngine.orthogonalSegments(route.points).filter(segment => segment.a.x === segment.b.x);
      assert.ok(vertical.length <= 1, "a link turns directly onto the pin height, without a second vertical step");
      if (vertical.length) tracks.push(vertical[0].a.x);
      const pin = outgoing ? route.start : route.end;
      const approach = outgoing ? route.points[1] : route.points.at(-2);
      assert.equal(approach.y, pin.y, "the shared horizontal span stays at the pin height");
    }
    assert.equal(new Set(tracks).size, tracks.length, "vertical tracks remain distinct");
    assertNoBodyCrossings(result);
  }
});

test("opposing links do not add a staircase at their shared receiving pins", () => {
  const result = render(syntheticGraph());
  for (const edge of result.graph.edges) {
    if (result.layout.rank.get(edge.target.id) !== result.layout.rank.get(edge.source.id) + 1) continue;
    const peers = result.graph.edges.filter(other => other.target.id === edge.target.id && other.target.port === edge.target.port);
    if (peers.length < 2) continue;
    const vertical = layoutEngine.orthogonalSegments(result.routes.get(edge.index).points)
      .filter(segment => segment.a.x === segment.b.x);
    assert.ok(vertical.length <= 1, `edge ${edge.index} turns onto the pin height without a receiving jog`);
  }
});

test("boundary arrivals keep their vertical track separate from return links", () => {
  const ports = (input, output) => [...input.map(name => ({ name, direction: "input" })),
    ...output.map(name => ({ name, direction: "output" }))];
  const topology = {
    owners: [owner("owner:0", null, ports(["peer"], ["process_exit", "reply", "session_exit"])),
      owner("owner:1", "owner:0", ports(["in"], ["exit", "out"]))],
    nodes: [operator("source", "owner:1", ports(["in", "remember"], ["out"])),
      operator("filter", "owner:0", ports(["in"], ["accepted", "rejected"])),
      operator("receiver", "owner:0", ports(["in"], ["exit", "out"]))],
    edges: [edge(0, "receiver", "out", "filter", "in"),
      edge(1, "source", "out", "receiver", "in"),
      { ...edge(2, "filter", "rejected", "boundary:output:reply", "reply"),
        target: { id: "boundary:output:reply", kind: "output", port: "reply", owner: "owner:0" } },
      edge(3, "filter", "rejected", "receiver", "in")],
  };
  const { routes } = render(topology);
  const vertical = [...routes.values()].map(route => layoutEngine.orthogonalSegments(route.points)
    .filter(({ a, b }) => a.x === b.x));
  for (let left = 0; left < vertical.length; left++) {
    for (let right = left + 1; right < vertical.length; right++) {
      for (const a of vertical[left]) for (const b of vertical[right]) {
        if (a.a.x !== b.a.x) continue;
        const shared = Math.min(Math.max(a.a.y, a.b.y), Math.max(b.a.y, b.b.y))
          - Math.max(Math.min(a.a.y, a.b.y), Math.min(b.a.y, b.b.y));
        assert.ok(shared <= 0, "only horizontal pin approaches may be shared");
      }
    }
  }
});

test("a long early exit joins a later exit without reversing the incoming pipes", () => {
  const ports = [{ name: "in", direction: "input" }, { name: "out", direction: "output" },
    { name: "exit", direction: "output" }];
  const topology = {
    owners: [owner("owner:0", null, [{ name: "failed", direction: "output" },
      { name: "size", direction: "output" }, { name: "empty", direction: "output" }])],
    nodes: [operator("inspect", "owner:0", [{ name: "in", direction: "input" },
      { name: "accepted", direction: "output" }, { name: "size", direction: "output" },
      { name: "empty", direction: "output" }]), operator("reader", "owner:0", ports),
      operator("writer", "owner:0", ports)],
    edges: [edge(0, "reader", "out", "inspect", "in"), edge(1, "inspect", "accepted", "writer", "in"),
      ...[["reader", "exit", "failed"], ["writer", "exit", "failed"],
        ["inspect", "size", "size"], ["inspect", "empty", "empty"]].map(([id, port, result], index) => ({
        ...edge(index + 2, id, port, `boundary:output:${result}`, result),
        target: { id: `boundary:output:${result}`, port: result, kind: "output", owner: "owner:0" },
      }))],
  };
  const result = render(topology);
  assert.equal(layoutEngine.countRouteCrossings(result.routes), 0);
  assert.equal(result.graph.nodes.filter(node => node.kind === "operator").length, 3);
  assertNoBodyCrossings(result);
  assert.equal(stableProjection(result), stableProjection(render(topology)));
});

test("self-returning pipes keep their ports and avoid the node body", () => {
  const topology = { owners: [owner("owner:0", null)], nodes: [operator("loop", "owner:0",
    [{ name: "in", direction: "input" }, { name: "out", direction: "output" }])],
    edges: [edge(0, "loop", "out", "loop", "in")] };
  const result = render(topology);
  const box = result.layout.positions.get("loop"), route = result.routes.get(0);
  assert.equal(route.start.x, box.x + box.width);
  assert.equal(route.end.x, box.x);
  const scope = result.layout.scopes.get("owner:0");
  assert.ok(route.points.every(point => point.x > scope.x && point.x < scope.x + scope.width
    && point.y > scope.y && point.y < scope.y + scope.height), "the return stays inside its scope");
  assert.ok(layoutEngine.sampleCurvedCorridor(route.presentationPoints, undefined, 64).every(point =>
    !(point.x > box.x && point.x < box.x + box.width && point.y > box.y && point.y < box.y + box.height)));
  assert.equal(stableProjection(result), stableProjection(render(topology)));
});

test("layered routing remains deterministic and interactive as the link count doubles", () => {
  const topology = syntheticGraph();
  const measure = value => {
    const graph = layoutEngine.materializeGraph(value);
    const layout = layoutEngine.layoutGraph(graph.nodes, graph.edges, value.owners);
    const start = performance.now();
    const routes = layoutEngine.routeGraphEdges(graph, layout);
    return { elapsed: performance.now() - start, result: { graph, layout, routes } };
  };
  const first = measure(topology);
  const second = measure(topology);
  assert.equal(stableProjection(first.result), stableProjection(second.result));
  assert.ok(layoutEngine.countRouteCrossings(first.result.routes) <= 128);
  assert.ok(layoutEngine.countRouteOverlaps(first.result.routes) <= 93);
  const groups = new Map();
  for (const node of first.result.graph.nodes) {
    const rank = first.result.layout.rank.get(node.id);
    if (!groups.has(rank)) groups.set(rank, []);
    groups.get(rank).push(node);
  }
  for (const group of groups.values()) group.sort((a, b) =>
    first.result.layout.positions.get(a.id).y - first.result.layout.positions.get(b.id).y);
  assert.ok(layoutEngine.countLayerCrossings(groups, first.result.graph.edges, first.result.layout.rank) <= 32);
  assertNoBodyCrossings(first.result);
  const single = Math.min(first.elapsed, second.elapsed);
  assert.ok(single < 2000, `67 links route in ${single.toFixed(1)} ms (limit 2000 ms)`);
  const doubled = { ...topology, edges: [...topology.edges,
    ...topology.edges.map((value, index) => ({ ...value, index: topology.edges.length + index,
      route: `${value.route}:parallel` }))] };
  const double = Math.min(measure(doubled).elapsed, measure(doubled).elapsed);
  assert.ok(double <= single * 3, `134 links: ${double.toFixed(1)} ms; 67 links: ${single.toFixed(1)} ms`);
});

test("barycentric sweeps untangle crossed adjacent layers deterministically", () => {
  const topology = {
    owners: [owner("owner:0", null)],
    nodes: [
      operator("a", "owner:0", [{ name: "out", direction: "output" }]),
      operator("b", "owner:0", [{ name: "out", direction: "output" }]),
      operator("x", "owner:0", [{ name: "in", direction: "input" }]),
      operator("y", "owner:0", [{ name: "in", direction: "input" }]),
    ],
    edges: [
      edge(0, "a", "out", "y", "in"),
      edge(1, "b", "out", "x", "in"),
    ],
  };

  const first = render(topology);
  const second = render(topology);
  const sourceOrder = Math.sign(first.layout.positions.get("a").y - first.layout.positions.get("b").y);
  const targetOrder = Math.sign(first.layout.positions.get("y").y - first.layout.positions.get("x").y);
  assert.equal(sourceOrder, targetOrder, "connected pairs keep the same vertical order");
  assert.equal(layoutEngine.countRouteCrossings(first.routes), 0);
  assert.equal(stableProjection(first), stableProjection(second));
});

test("independent parallel pipes do not widen their shared column gap", () => {
  const parallel = count => ({
    owners: [owner("owner:0", null)],
    nodes: Array.from({ length: count }, (_, index) => [
      operator(`source_${index}`, "owner:0", [{ name: "out", direction: "output" }]),
      operator(`target_${index}`, "owner:0", [{ name: "in", direction: "input" }]),
    ]).flat(),
    edges: Array.from({ length: count }, (_, index) => edge(index, `source_${index}`, "out", `target_${index}`, "in")),
  });
  const single = render(parallel(1)), stacked = render(parallel(8));
  const gap = result => {
    const source = result.layout.positions.get("source_0"), target = result.layout.positions.get("target_0");
    return target.x - source.x - source.width;
  };
  assert.equal(gap(stacked), gap(single), "disjoint rows do not reserve separate horizontal tracks");
  assert.equal(layoutEngine.countRouteCrossings(stacked.routes), 0);
  assert.equal(layoutEngine.countRouteOverlaps(stacked.routes), 0);
  assertNoBodyCrossings(stacked);
  assert.equal(stableProjection(stacked), stableProjection(render(parallel(8))));
});

test("vertically disjoint bends reuse a track without merging their pipes", () => {
  const topology = {
    owners: [owner("owner:0", null)],
    nodes: [...Array.from({ length: 8 }, (_, index) => [
      operator(`source_${index}`, "owner:0", [{ name: "out", direction: "output" }]),
      operator(`target_${index}`, "owner:0", [{ name: "in", direction: "input" }]),
    ]).flat(), operator("spare", "owner:0", [{ name: "out", direction: "output" }])],
    edges: Array.from({ length: 8 }, (_, index) => edge(index, `source_${index}`, "out", `target_${index}`, "in")),
  };
  const result = render(topology);
  const vertical = [...result.routes.values()].flatMap(route => layoutEngine.orthogonalSegments(route.points)
    .filter(segment => segment.a.x === segment.b.x && segment.a.y !== segment.b.y));
  assert.equal(vertical.length, 8, "every pair of pins needs a bend");
  assert.ok(vertical.every(segment => Math.abs(segment.a.y - segment.b.y) > 12), "bends exceed a corner radius");
  assert.equal(new Set(vertical.map(segment => segment.a.x)).size, 1, "disjoint bends share one track");
  assert.equal(result.routes.size, 8, "each semantic pipe stays independently inspectable");
  assert.equal(layoutEngine.countRouteCrossings(result.routes), 0);
  assert.equal(layoutEngine.countRouteOverlaps(result.routes), 0);
  assertNoBodyCrossings(result);
  assert.equal(stableProjection(result), stableProjection(render(topology)));
});

test("overlapping vertical intervals keep distinct ordered tracks", () => {
  const topology = {
    owners: [owner("owner:0", null)],
    nodes: [
      operator("source", "owner:0", [{ name: "one", direction: "output" }, { name: "two", direction: "output" }]),
      operator("spare1", "owner:0", [{ name: "out", direction: "output" }]),
      operator("spare2", "owner:0", [{ name: "out", direction: "output" }]),
      operator("upper", "owner:0", [{ name: "in", direction: "input" }]),
      operator("lower", "owner:0", [{ name: "in", direction: "input" }]),
    ],
    edges: [edge(0, "source", "one", "upper", "in"), edge(1, "source", "two", "lower", "in")],
  };
  const result = render(topology);
  const bends = [...result.routes.values()].map(route => layoutEngine.orthogonalSegments(route.points)
    .filter(segment => segment.a.x === segment.b.x && segment.a.y !== segment.b.y));
  assert.deepEqual(bends.map(segments => segments.length), [1, 1]);
  const [short, long] = bends.map(segments => segments[0]);
  assert.ok(short.b.y > long.a.y, "the vertical intervals overlap");
  assert.ok(short.a.x - long.a.x >= 12 - 0.01, "the longer bend precedes the shorter bend on its own track");
  assert.equal(layoutEngine.countRouteCrossings(result.routes), 0);
  assert.equal(layoutEngine.countRouteOverlaps(result.routes), 0);
  assertNoBodyCrossings(result);
  assert.equal(stableProjection(result), stableProjection(render(topology)));
});

test("compaction keeps forward bends outside a return at the same output", () => {
  const ports = [{ name: "in", direction: "input" }, { name: "out", direction: "output" }];
  const topology = {
    owners: [owner("owner:0", null)],
    nodes: Array.from({ length: 6 }, (_, index) => operator(`n${index}`, "owner:0", ports)),
    edges: [[0, 3], [0, 4], [0, 5], [1, 5], [5, 0]]
      .map(([source, target], index) => edge(index, `n${source}`, "out", `n${target}`, "in")),
  };
  const result = render(topology);
  assert.equal(layoutEngine.countRouteCrossings(result.routes), 0);
  const forward = layoutEngine.orthogonalSegments(result.routes.get(1).points)
    .filter(segment => segment.a.x === segment.b.x);
  const returning = result.routes.get(2);
  const turns = layoutEngine.orthogonalSegments(returning.points)
    .filter(segment => segment.a.x === segment.b.x && segment.a.x > returning.start.x);
  assert.ok(forward.length && turns.length);
  assert.ok(Math.min(...forward.map(segment => segment.a.x))
    - Math.max(...turns.map(segment => segment.a.x)) >= 12 - 0.01, "the neighboring tracks keep their clearance");
  assertNoBodyCrossings(result);
  assert.equal(stableProjection(result), stableProjection(render(topology)));
});

test("request-response cycles keep forward columns and every validated pipe", () => {
  const ports = [{ name: "in", direction: "input" }, { name: "out", direction: "output" }];
  const topology = {
    owners: [owner("owner:0", null), owner("owner:1", "owner:0", ports)],
    nodes: [
      operator("html", "owner:0", ports),
      { ...operator("page", "owner:0", ports), kind: "connector", profile: "Call · Inbound" },
      operator("admit", "owner:1", ports),
      operator("model", "owner:1", ports),
      operator("address_reply", "owner:1", ports),
    ],
    edges: [
      edge(0, "page", "out", "html", "in"),
      edge(1, "html", "out", "page", "in"),
      edge(2, "admit", "out", "model", "in"),
      edge(3, "model", "out", "address_reply", "in"),
      edge(4, "address_reply", "out", "admit", "in"),
    ],
  };
  const before = JSON.stringify(topology);
  const result = render(topology);
  for (const [left, right] of [["page", "html"], ["admit", "model"], ["model", "address_reply"]]) {
    const a = result.layout.positions.get(left), b = result.layout.positions.get(right);
    assert.ok(a.x + a.width < b.x, `${left} precedes ${right} in the displayed flow`);
  }
  assert.equal(result.routes.size, topology.edges.length, "cycle removal is confined to ranking");
  for (const index of [1, 4]) {
    const route = result.routes.get(index);
    assert.match(layoutEngine.curvedCorridorPath(route.presentationPoints), / Q /);
    assert.ok(route.presentationPoints[1].x > route.start.x, "return leaves the output toward the right");
    assert.ok(route.presentationPoints.at(-2).x < route.end.x, "return approaches the input from the left");
    for (let point = 1; point < route.presentationPoints.length; point += 1) {
      const a = route.presentationPoints[point - 1], b = route.presentationPoints[point];
      assert.ok(a.x === b.x || a.y === b.y, "return tracks are orthogonal");
    }
  }
  assert.equal(JSON.stringify(topology), before);
  assert.equal(stableProjection(result), stableProjection(render(topology)));
});

test("every pipe follows orthogonal tracks with only local rounded corners", () => {
  const result = render(syntheticGraph());
  for (const route of result.routes.values()) {
    assert.deepEqual(route.presentationPoints, route.points, "presentation keeps the allocated tracks");
    const path = layoutEngine.curvedCorridorPath(route.presentationPoints);
    assert.doesNotMatch(path, / C /, "no free-form cubic replaces the tracks");
    for (const segment of layoutEngine.orthogonalSegments(route.presentationPoints)) {
      assert.ok(segment.a.x === segment.b.x || segment.a.y === segment.b.y);
    }
    for (const segment of layoutEngine.curvedCorridorSegments(route.presentationPoints)) {
      if (segment.line) {
        assert.ok(segment.from.x === segment.to.x || segment.from.y === segment.to.y);
      } else {
        assert.ok(Math.abs(segment.to.x - segment.from.x) <= 12, "rounding stays local to a corner");
        assert.ok(Math.abs(segment.to.y - segment.from.y) <= 12, "rounding stays local to a corner");
      }
    }
  }
  assert.equal(layoutEngine.curvedCorridorPath([{ x: 10, y: 20 }, { x: 80, y: 20 }]),
    "M 10 20 L 80 20", "aligned pins need only one straight segment");
});

test("forward tracks stay orthogonal and fan-in shares its final approach", () => {
  const ports = [{ name: "in", direction: "input" }, { name: "out", direction: "output" }];
  const topology = { owners: [owner("owner:0", null)],
    nodes: [operator("top", "owner:0", ports), operator("bottom", "owner:0", ports), operator("receive", "owner:0", ports)],
    edges: [edge(0, "top", "out", "receive", "in"), edge(1, "bottom", "out", "receive", "in")] };
  const single = render({ ...topology, edges: [topology.edges[0]] });
  const singlePath = layoutEngine.curvedCorridorPath(single.routes.get(0).presentationPoints);
  assert.match(singlePath, / L /);
  assert.doesNotMatch(singlePath, / C /);
  const { routes } = render(topology);
  const first = layoutEngine.curvedCorridorSegments(routes.get(0).presentationPoints);
  const second = layoutEngine.curvedCorridorSegments(routes.get(1).presentationPoints);
  assert.deepEqual(first.at(-1).to, second.at(-1).to, "the receiving pin remains shared");
  for (const segments of [first, second]) {
    const last = segments.at(-1);
    assert.ok(last.line && last.from.y === last.to.y, "the final approach is horizontal");
    assert.ok(last.to.x - last.from.x >= 16, "both pipes share a short final approach");
  }
  assert.notDeepEqual(first[0].from, second[0].from, "both original producers remain distinct");
  assert.equal(routes.size, 2);
});

test("an orthogonal scope exit stays between its pins despite another outgoing pipe", () => {
  const ports = [{ name: "in", direction: "input" }, { name: "out", direction: "output" }];
  const topology = { owners: [owner("owner:0", null, [{ name: "reply", direction: "output" }]),
    owner("owner:1", "owner:0", ports)],
    nodes: [{ ...operator("peer", "owner:0", ports), kind: "connector", profile: "Call · Inbound" },
      operator("inside", "owner:1", ports)],
    edges: [edge(0, "peer", "out", "inside", "in"), edge(1, "inside", "out", "peer", "in"),
      { ...edge(2, "inside", "out", "boundary:output:reply", "reply"),
        target: { id: "boundary:output:reply", port: "reply", kind: "output", owner: "owner:0" } }] };
  const { routes } = render(topology);
  const route = routes.get(2), portal = route.portals.at(-1);
  assert.ok(portal);
  for (const point of route.presentationPoints.filter(point => point.x >= portal.x)) {
    assert.ok(point.y >= Math.min(portal.y, route.end.y) && point.y <= Math.max(portal.y, route.end.y),
      "fan-out does not add a vertical overshoot after the child scope");
  }
});

test("recursive scopes preserve containment, flow order and instance-title space", () => {
  const ports = [{ name: "in", direction: "input" }, { name: "out", direction: "output" }];
  const topology = {
    owners: [owner("owner:0", null), owner("owner:1", "owner:0", ports)],
    nodes: [
      operator("listener", "owner:0", ports),
      operator("validate", "owner:0", ports),
      operator("deduplicate", "owner:0", ports),
      operator("request", "owner:1", ports),
      operator("partner", "owner:1", ports),
      operator("accounting", "owner:1", ports),
    ],
    edges: [
      edge(0, "listener", "out", "validate", "in"),
      edge(1, "validate", "out", "deduplicate", "in"),
      edge(2, "deduplicate", "out", "request", "in"),
      edge(3, "request", "out", "partner", "in"),
      edge(4, "partner", "out", "accounting", "in"),
    ],
  };
  const before = JSON.stringify(topology);
  const initial = render(topology);
  for (const [first, second] of [["listener", "validate"], ["validate", "deduplicate"],
    ["request", "partner"], ["partner", "accounting"]]) {
    assert.ok(initial.layout.positions.get(first).x < initial.layout.positions.get(second).x,
      "each scope keeps its own left-to-right flow");
  }
  const child = initial.layout.scopes.get("owner:1");
  const parent = initial.layout.scopes.get("owner:0");
  assert.equal(parent.depth, 0);
  assert.equal(child.depth, 1, "a nested scope keeps its containment styling");
  assert.ok(child.x > parent.x && child.y > parent.y);
  assert.ok(child.x + child.width < parent.x + parent.width);
  assert.ok(child.y + child.height < parent.y + parent.height);
  for (const id of ["request", "partner", "accounting"]) {
    const box = initial.layout.positions.get(id);
    assert.ok(box.x > child.x && box.y > child.y);
    assert.ok(box.x + box.width < child.x + child.width);
    assert.ok(box.y + box.height < child.y + child.height);
  }
  assertNoBodyCrossings(initial);
  assert.ok(initial.layout.positions.get("deduplicate").width >= 140, "the full name and instance badge have separate space");
  assert.equal(JSON.stringify(topology), before, "layout does not change validated topology");
  const opened = render({ ...topology, nodes: topology.nodes.map(node => ({ ...node, instances: [{}] })) });
  assert.equal(stableProjection(initial), stableProjection(opened), "opening an instance does not move the graph");
});

test("a four-digit mailbox count fits the routed badge without changing the edge", () => {
  const topology = {
    owners: [owner("owner:0", null)],
    nodes: [
      operator("a", "owner:0", [{ name: "out", direction: "output" }]),
      operator("b", "owner:0", [{ name: "in", direction: "input" }]),
    ],
    edges: [edge(0, "a", "out", "b", "in")],
  };
  const empty = render(topology);
  const full = render({ ...topology, edges: [{ ...topology.edges[0], queued_count: "1024" }] });
  assert.deepEqual(full.routes.get(0).presentationPoints, empty.routes.get(0).presentationPoints);
  assert.deepEqual(full.routes.get(0).label, empty.routes.get(0).label, 'four digits are reserved before admission');
  assert.ok(full.routes.get(0).label.box.width * layoutEngine.minimumReadableScale >= 40);
});

test("Connector subtitles and Extension labels fit before any instance opens", () => {
  const topology = {
    owners: [owner("owner:0", null)],
    nodes: [{
      ...operator("partner", "owner:0", [
        { name: "in", direction: "input" },
        { name: "out", direction: "output" },
        { name: "exit", direction: "output" },
      ]),
      kind: "connector", type: "std/http-client@1", extensions: ["Retry"],
    }],
    edges: [],
  };
  const { layout } = render(topology);
  const subtitle = "HttpClient · + Retry";
  assert.ok(layout.positions.get("partner").width >= subtitle.length * 6.8 / layoutEngine.minimumReadableScale + 20,
    "the subtitle stays inside its box, away from external pins and routes");
  const opened = render({ ...topology, nodes: topology.nodes.map(node => ({ ...node, instances: [{}] })) });
  assert.deepEqual(layout.positions, opened.layout.positions, "admission must not reflow the graph");
});

test("Graph-side ports slide to their neighbours instead of retaining declaration order", () => {
  const topology = {
    owners: [owner("owner:0", null, [
      { name: "beta", direction: "input" },
      { name: "alpha", direction: "input" },
    ])],
    nodes: [
      operator("top", "owner:0", [{ name: "in", direction: "input" }]),
      operator("bottom", "owner:0", [{ name: "in", direction: "input" }]),
    ],
    edges: [
      edge(0, "boundary:input:alpha", "alpha", "top", "in"),
      edge(1, "boundary:input:beta", "beta", "bottom", "in"),
    ],
  };
  const { graph, layout } = render(topology);
  const root = topology.owners[0];
  const alpha = layoutEngine.scopePortAnchor(root, layout.scopes.get(root.id), root.ports[1], layout);
  const beta = layoutEngine.scopePortAnchor(root, layout.scopes.get(root.id), root.ports[0], layout);
  assert.ok(alpha.y < beta.y, "the alpha pin follows its upper consumer");
  const boundary = graph.byId.get("boundary:input:alpha");
  const boundaryAnchor = layoutEngine.portAnchor(
    boundary,
    layout.positions.get(boundary.id),
    "alpha",
    "output",
    layout,
  );
  assert.equal(boundaryAnchor.y, alpha.y, "the visible root pin and routing endpoint are one point");
});

test("a cross-hierarchy edge passes through the identifiable nested Graph port", () => {
  const topology = {
    owners: [
      owner("owner:0", null, [{ name: "in", direction: "input" }]),
      owner("owner:1", "owner:0", [{ name: "in", direction: "input" }]),
    ],
    nodes: [operator("inside", "owner:1", [{ name: "in", direction: "input" }])],
    edges: [edge(0, "boundary:input:in", "in", "inside", "in")],
  };
  const { layout, routes } = render(topology);
  const route = routes.get(0);
  assert.deepEqual(
    route.portals.map((portal) => [portal.owner, portal.port, portal.direction]),
    [["owner:1", "in", "input"]],
  );
  const portal = route.portals[0];
  const crossesPortal = layoutEngine.orthogonalSegments(route.points).some((segment) =>
    segment.a.y === portal.y
      && segment.b.y === portal.y
      && Math.min(segment.a.x, segment.b.x) <= portal.x
      && Math.max(segment.a.x, segment.b.x) >= portal.x);
  assert.ok(crossesPortal, "the pipe visibly crosses the nested pin");
  assert.equal(portal.x, layout.scopes.get("owner:1").x);
});

test("independent pipes receive orthogonal non-overlapping tracks", () => {
  const topology = {
    owners: [owner("owner:0", null)],
    nodes: [
      operator("a", "owner:0", [{ name: "out", direction: "output" }]),
      operator("b", "owner:0", [{ name: "out", direction: "output" }]),
      operator("x", "owner:0", [{ name: "in", direction: "input" }]),
      operator("y", "owner:0", [{ name: "in", direction: "input" }]),
    ],
    edges: [edge(0, "a", "out", "x", "in"), edge(1, "b", "out", "y", "in")],
  };
  const { routes } = render(topology);
  for (const route of routes.values()) {
    for (const segment of layoutEngine.orthogonalSegments(route.points)) {
      assert.ok(segment.a.x === segment.b.x || segment.a.y === segment.b.y);
    }
  }
  assert.equal(layoutEngine.countRouteCrossings(routes), 0);
  assert.equal(layoutEngine.countRouteOverlaps(routes), 0);
});

test("fan-out separates onto distinct tracks after its common departure", () => {
  const topology = {
    owners: [owner("owner:0", null, [{ name: "in", direction: "input" }])],
    nodes: [
      operator("top", "owner:0", [{ name: "in", direction: "input" }]),
      operator("bottom", "owner:0", [{ name: "in", direction: "input" }]),
    ],
    edges: [
      edge(0, "boundary:input:in", "in", "top", "in"),
      edge(1, "boundary:input:in", "in", "bottom", "in"),
    ],
  };
  const { routes } = render(topology);
  assert.equal(layoutEngine.countRouteCrossings(routes), 0);
  assert.equal(layoutEngine.countRouteOverlaps(routes), 1, "pipes share only their short departure");
  const upper = layoutEngine.curvedCorridorSegments(routes.get(0).presentationPoints);
  const lower = layoutEngine.curvedCorridorSegments(routes.get(1).presentationPoints);
  assert.deepEqual(upper[0].from, lower[0].from, "both pipes start at the exact output pin");
  assert.ok(upper[0].line && lower[0].line, "both departures are straight");
  assert.notDeepEqual(upper, lower, "the pipes remain distinct after their shared departure");
  assert.ok(routes.get(0).presentationPoints.some(point => point.y < routes.get(0).start.y));
  assert.ok(routes.get(1).presentationPoints.some(point => point.y > routes.get(1).start.y));
});

test("ambiguous flattened composition seams never invent a Graph port", () => {
  const topology = {
    owners: [
      owner("owner:0", null),
      owner("owner:1", "owner:0", [
        { name: "alpha", direction: "input" },
        { name: "beta", direction: "input" },
      ]),
    ],
    nodes: [
      operator("outside", "owner:0", [{ name: "out", direction: "output" }]),
      operator("inside", "owner:1", [{ name: "in", direction: "input" }]),
    ],
    edges: [edge(0, "outside", "out", "inside", "in")],
  };
  const { layout, routes } = render(topology);
  assert.deepEqual(routes.get(0).portals, []);
  const outside = layout.positions.get("outside"), scope = layout.scopes.get("owner:1");
  const gap = scope.x - outside.x - outside.width;
  assert.ok(gap > 0 && gap <= 128,
    "internal routing names and inward-facing scope labels do not widen the parent gap");
});

test("orthogonal detours keep straight tracks and bounded rounded corners", () => {
  const points = [
    { x: 10, y: 20 },
    { x: 80, y: 20 },
    { x: 80, y: 90 },
    { x: 140, y: 90 },
  ];
  const path = layoutEngine.curvedCorridorPath(points);
  assert.match(path, /^M 10 20 /);
  assert.match(path, / 140 90$/);
  assert.match(path, / L /, "detours retain straight tracks");
  assert.match(path, / Q /, "corners are rounded without a spline through the whole route");
  const segments = layoutEngine.curvedCorridorSegments(points);
  assert.equal(segments[0].from.y, segments[0].c1.y, "the output tangent is horizontal");
  assert.equal(segments.at(-1).c2.y, segments.at(-1).to.y, "the input tangent is horizontal");
  for (const point of layoutEngine.sampleCurvedCorridor(points, undefined, 16)) {
    assert.ok(point.x >= 10 && point.x <= 140);
    assert.ok(point.y >= 20 && point.y <= 90);
  }
  assert.equal(layoutEngine.orthogonalSegments(points).length, 3);
});

test("explicit nothing drains remain local output ports", () => {
  const topology = {
    owners: [owner("owner:0", null)],
    nodes: [{ ...operator("model", "owner:0", [
      { name: "err", direction: "output" },
      { name: "exit", direction: "output" },
      { name: "out", direction: "output" },
    ]), kind: "connector" }],
    edges: [
      {
        index: 0,
        route: "model.err>!discard",
        source: { id: "model", port: "err", kind: "node", owner: "owner:0" },
        target: { id: "discard:0", port: null, kind: "discard", owner: "owner:0", label: "Discard · model.err" },
      },
      {
        index: 1,
        route: "model.out>!discard",
        source: { id: "model", port: "out", kind: "node", owner: "owner:0" },
        target: { id: "discard:1", port: null, kind: "discard", owner: "owner:0", label: "Discard · model.out" },
      },
    ],
  };
  const { graph, routes } = render(topology);
  assert.deepEqual(graph.edges, [], "a drain with no inspectable destination is not drawn as a pipe");
  assert.equal(graph.nodes.some((node) => node.kind === "discard"), false, "no synthetic sink box is created");
  assert.deepEqual(
    graph.byId.get("model").ports.map((port) => port.name),
    ["err", "exit", "out"],
    "all declared contract ports remain visible",
  );
  assert.equal(routes.size, 0);
});

test("manager commands never create spatial nodes or ports", () => {
  const topology = {
    owners: [owner("owner:0", null, [
      { name: "in", direction: "input" },
      { name: "out", direction: "output" },
      { name: "Loop.expired", direction: "output" },
    ])],
    nodes: [operator("judge", "owner:0", [
      { name: "in", direction: "input" }, { name: "revised", direction: "output" },
    ])],
    edges: [edge(0, "boundary:input:in", "in", "judge", "in")],
    commands: [{ source: "judge.revised", owner: "App", command: "Loop.next", cases: [] }],
  };
  const { graph, routes } = render(topology);
  assert.equal(graph.edges.length, 1);
  assert.equal(routes.size, 1);
  assert.ok(graph.nodes.every((node) => node.kind === "operator" || node.kind === "boundary"));
  assert.ok(graph.nodes.every((node) => node.ports.every((port) =>
    !["Loop.next", "Loop.again", "Loop.done"].includes(port.name))));
});

test("Graph outputs keep a non-overlapping side band without synthetic sinks", () => {
  const topology = {
    owners: [owner("owner:0", null, [
      { name: "done", direction: "output" },
      { name: "failed", direction: "output" },
    ])],
    nodes: [operator("model", "owner:0", [{ name: "out", direction: "output" }])],
    edges: [{
      index: 0,
      route: "model.out>!discard",
      source: { id: "model", port: "out", kind: "node", owner: "owner:0" },
      target: { id: "discard:0", port: null, kind: "discard", owner: "owner:0", label: "Discard · model.out" },
      queued_count: "0",
      mailbox: [],
    }],
  };
  const { graph, layout, routes } = render(topology);
  const root = topology.owners[0];
  const ys = root.ports
    .map((port) => layoutEngine.scopePortAnchor(root, layout.scopes.get(root.id), port, layout).y)
    .sort((left, right) => left - right);
  for (let index = 1; index < ys.length; index += 1) {
    assert.ok(ys[index] - ys[index - 1] >= 20, "every right-side target keeps the declared spacing");
  }
  assert.equal(graph.nodes.some((node) => node.kind === "discard"), false);
  assert.equal(routes.size, 0);
});

test("the root reserves a complete output gutter without stretching a nested scope", () => {
  const topology = {
    owners: [
      owner("owner:0", null, [
        { name: "done", direction: "output" },
        { name: "expired", direction: "output" },
        { name: "failed", direction: "output" },
        { name: "next", direction: "output" },
        { name: "in", direction: "input" },
      ]),
      owner("owner:1", "owner:0", [{ name: "in", direction: "input" }]),
    ],
    nodes: [
      operator("observe", "owner:1", [
        { name: "in", direction: "input" },
        { name: "out", direction: "output" },
      ]),
      { ...operator("model", "owner:0", [
        { name: "in", direction: "input" },
        { name: "err", direction: "output" },
        { name: "exit", direction: "output" },
        { name: "out", direction: "output" },
      ]), kind: "connector" },
    ],
    edges: [
      edge(0, "boundary:input:in", "in", "observe", "in"),
      edge(1, "boundary:input:in", "in", "model", "in"),
      {
        index: 2,
        route: "observe.out>!discard",
        source: { id: "observe", port: "out", kind: "node", owner: "owner:1" },
        target: { id: "discard:2", port: null, kind: "discard", owner: "owner:1", label: "Discard · audit.observe.out" },
      },
      ...["err", "out"].map((port, offset) => ({
        index: 3 + offset,
        route: `model.${port}>!discard`,
        source: { id: "model", port, kind: "node", owner: "owner:0" },
        target: { id: `discard:${3 + offset}`, port: null, kind: "discard", owner: "owner:0", label: `Discard · model.${port}` },
      })),
    ],
  };
  const { graph, layout, routes } = render(topology);
  const root = layout.scopes.get("owner:0");
  const nested = layout.scopes.get("owner:1");
  const rightmostWork = Math.max(
    layout.positions.get("model").x + layout.positions.get("model").width,
    layout.positions.get("observe").x + layout.positions.get("observe").width,
  );
  assert.ok(root.x + root.width - rightmostWork >= 140, "the root keeps its 140 px result column");
  assert.ok(nested.x + nested.width < root.x + root.width - 100, "the nested Graph does not consume the result gutter");
  assert.equal(graph.nodes.some((node) => node.kind === "discard"), false);
  assert.ok([...routes.values()].every((route) => route.label.leader === null), "mailboxes sit directly beside their route");
  const rootInput = layoutEngine.scopePortAnchor(
    topology.owners[0],
    root,
    topology.owners[0].ports.find((port) => port.direction === "input"),
    layout,
  );
  const nestedInput = layoutEngine.scopePortAnchor(
    topology.owners[1],
    nested,
    topology.owners[1].ports[0],
    layout,
  );
  const fanLabel = routes.get(0).label.box;
  const rootLabelEnd = rootInput.x + 9 + 16;
  const nestedLabelStart = nestedInput.x + 9;
  const nestedLabelEnd = nestedLabelStart + 16;
  assert.ok(fanLabel.x - rootLabelEnd >= 16, "the fan-out counter clears the App input label");
  assert.ok(nestedLabelStart - (fanLabel.x + fanLabel.width) >= 16
    || fanLabel.x - nestedLabelEnd >= 16, "the fan-out counter clears either side of the nested input label");
});

test("compact input gutters keep long facing labels and their mailbox apart", () => {
  const ports = [{ name: "heartbeat", direction: "input" }];
  for (const nested of [false, true]) {
    const scope = nested ? "owner:1" : "owner:0";
    const topology = {
      owners: [owner("owner:0", null, ports), ...(nested ? [owner(scope, "owner:0", ports)] : [])],
      nodes: [operator("watchdog", scope, ports)],
      edges: [edge(0, "boundary:input:heartbeat", "heartbeat", "watchdog", "heartbeat")],
    };
    const { layout, routes } = render(topology);
    const source = layout.scopes.get(scope).x;
    const target = layout.positions.get("watchdog").x;
    const labelWidth = ("heartbeat".length * 6 + 4) / layoutEngine.minimumReadableScale;
    const badge = routes.get(0).label.box;
    assert.ok(target - 8 - labelWidth > source + 9 + labelWidth, "the facing input labels do not overlap");
    assert.ok(badge.x >= source + 9 + labelWidth + 16, "the mailbox clears the scope input label");
    assert.ok(badge.x + badge.width <= target - 8 - labelWidth - 16, "the mailbox clears the node input label");
  }
});

test("nested frames do not repeat the App's outer gutters or shrink their contents", () => {
  const ports = [{ name: "in", direction: "input" }, { name: "out", direction: "output" }];
  const topology = nested => ({
    owners: [owner("owner:0", null, ports), ...(nested ? [owner("owner:1", "owner:0", ports)] : [])],
    nodes: [operator("work", nested ? "owner:1" : "owner:0", ports)],
    edges: [edge(0, "boundary:input:in", "in", "work", "in"),
      edge(1, "work", "out", "boundary:output:out", "out")],
  });
  const standalone = render(topology(false)), nested = render(topology(true));
  const a = standalone.layout.positions.get("work"), b = nested.layout.positions.get("work");
  assert.equal(b.width, a.width);
  assert.equal(b.height, a.height);
  assert.ok(nested.layout.scopes.get("owner:1").width < standalone.layout.scopes.get("owner:0").width,
    "the nested frame keeps local routing space without repeating the outer App gutters");
  assertNoBodyCrossings(nested);
});

test("mailbox labels avoid every unrelated presentation curve", () => {
  const topology = {
    owners: [owner("owner:0", null, [{ name: "in", direction: "input" }])],
    nodes: [
      operator("top", "owner:0", [{ name: "in", direction: "input" }, { name: "out", direction: "output" }]),
      operator("bottom", "owner:0", [{ name: "in", direction: "input" }, { name: "out", direction: "output" }]),
    ],
    edges: [
      edge(0, "boundary:input:in", "in", "top", "in"),
      edge(1, "boundary:input:in", "in", "bottom", "in"),
    ],
  };
  const { routes } = render(topology);
  for (const [index, route] of routes) {
    const box = route.label.box;
    for (const [otherIndex, other] of routes) {
      if (index === otherIndex) continue;
      const crosses = layoutEngine.sampleCurvedCorridor(other.presentationPoints, undefined, 24).some((point) =>
        point.x >= box.x - 4 && point.x <= box.x + box.width + 4
          && point.y >= box.y - 4 && point.y <= box.y + box.height + 4);
      assert.equal(crosses, false, `curve ${otherIndex} avoids label ${index}`);
    }
  }
});

test("a mailbox stays attached beside a compact vertical track without covering its neighbor", () => {
  const ports = [{ name: "in", direction: "input" }, { name: "out", direction: "output" }];
  const topology = {
    owners: [owner("owner:0", null)],
    nodes: Array.from({ length: 6 }, (_, index) => operator(`n${index}`, "owner:0", ports)),
    edges: [[0, 1], [0, 2], [0, 3], [1, 2], [1, 3], [2, 4]]
      .map(([source, target], index) => edge(index, `n${source}`, "out", `n${target}`, "in")),
  };
  const result = render(topology);
  for (const [index, route] of result.routes) {
    const box = route.label.box;
    const samples = layoutEngine.sampleCurvedCorridor(route.presentationPoints, undefined, 64);
    assert.ok(samples.some(point => point.x >= box.x - 0.01 && point.x <= box.x + box.width + 0.01
      && point.y >= box.y && point.y <= box.y + box.height), "the badge touches its own pipe");
    assert.equal(route.label.leader, null);
    for (const [otherIndex, other] of result.routes) {
      if (index === otherIndex) continue;
      const crosses = layoutEngine.sampleCurvedCorridor(other.presentationPoints, undefined, 64).some(point =>
        point.x >= box.x - 4 && point.x <= box.x + box.width + 4
          && point.y >= box.y - 4 && point.y <= box.y + box.height + 4);
      assert.equal(crosses, false, `pipe ${otherIndex} avoids badge ${index}`);
    }
  }
  assert.equal(stableProjection(result), stableProjection(render(topology)));
});

test("presentation curves avoid unrelated port labels and square pins", () => {
  const topology = {
    owners: [owner("owner:0", null, [{ name: "in", direction: "input" }])],
    nodes: [
      operator("source", "owner:0", [
        { name: "in", direction: "input" },
        { name: "alpha", direction: "output" },
        { name: "beta", direction: "output" },
      ]),
      operator("upper", "owner:0", [{ name: "in", direction: "input" }]),
      operator("lower", "owner:0", [{ name: "in", direction: "input" }]),
    ],
    edges: [
      edge(0, "boundary:input:in", "in", "source", "in"),
      edge(1, "source", "alpha", "upper", "in"),
      edge(2, "source", "beta", "lower", "in"),
    ],
  };
  const { graph, layout, routes } = render(topology);
  const portGeometry = [];
  for (const node of graph.nodes) {
    if (["boundary", "discard"].includes(node.kind)) continue;
    const position = layout.positions.get(node.id);
    for (const port of node.ports) {
      const anchor = layoutEngine.portAnchor(node, position, port.name, port.direction, layout);
      const labelX = anchor.x + (port.direction === "input" ? -8 : 8);
      const width = Math.max(10, port.name.length * 6 + 4);
      portGeometry.push({
        node: node.id,
        port: port.name,
        anchor,
        label: {
          x: port.direction === "input" ? labelX - width : labelX,
          y: anchor.y - 15,
          width,
          height: 13,
        },
      });
    }
  }
  for (const edgeValue of graph.edges) {
    const route = routes.get(edgeValue.index);
    if (!route) continue;
    const related = new Set([
      `${edgeValue.source.id}\u0000${edgeValue.source.port || ""}`,
      `${edgeValue.target.id}\u0000${edgeValue.target.port || ""}`,
    ]);
    const samples = layoutEngine.sampleCurvedCorridor(route.presentationPoints, undefined, 32);
    for (const geometry of portGeometry) {
      if (related.has(`${geometry.node}\u0000${geometry.port}`)) continue;
      const hitsDisk = samples.some((point) => Math.hypot(
        point.x - geometry.anchor.x,
        point.y - geometry.anchor.y,
      ) < 6);
      assert.equal(hitsDisk, false, `${edgeValue.route} avoids ${geometry.node}.${geometry.port} pin`);
      const hitsLabel = samples.some((point) =>
        point.x >= geometry.label.x - 2
          && point.x <= geometry.label.x + geometry.label.width + 2
          && point.y >= geometry.label.y - 2
          && point.y <= geometry.label.y + geometry.label.height + 2);
      assert.equal(hitsLabel, false, `${edgeValue.route} avoids ${geometry.node}.${geometry.port} label`);
    }
  }
});
