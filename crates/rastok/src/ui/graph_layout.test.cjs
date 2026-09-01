"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const layoutEngine = require("./graph_layout.js");

function owner(id, parent, ports = []) {
  return { id, parent, label: parent === null ? "App" : id, ports, aspects: [] };
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
    traversal_count: "0",
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
    routes: [...result.routes.entries()].map(([index, route]) => [index, route.points, route.portals]),
  });
}

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

test("fan-out pipes separate at the pin instead of sharing an unreadable trunk", () => {
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
  assert.equal(layoutEngine.countRouteOverlaps(routes), 0);
  assert.notEqual(
    routes.get(0).points[1].y,
    routes.get(1).points[1].y,
    "the two pipes become visually distinct immediately after their shared pin",
  );
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
  const { routes } = render(topology);
  assert.deepEqual(routes.get(0).portals, []);
});
