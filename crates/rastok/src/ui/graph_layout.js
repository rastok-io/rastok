/*
 * Deterministic compound-graph layout for the embedded Rastok UI.
 *
 * Dependency-free implementation of the applicable port-constrained phases
 * in Schulze, Spönemann and von Hanxleden (2014): proper layering, port-aware
 * sweeps, inverted-port dummies and recursive compound placement. Channel
 * constraints allocate orthogonal tracks before the curve presentation.
 * See docs/spec/ui.md for the reference and deterministic adaptations.
 * Geometry never changes semantic node, port or edge identities.
 */
(function install(root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) {
    module.exports = api;
  } else {
    root.RastokGraphLayout = api;
  }
}(typeof globalThis === "undefined" ? this : globalThis, function graphLayoutFactory() {
  "use strict";

  const SCOPE_HEADER = 26;
  const SCOPE_PADDING = 12;
  const PORT_SPACING = 20;
  const TRACK_SPACING = 12;
  const BOX_EDGE_CLEARANCE = 6;
  const FAN_STUB = 16;
  const ROOT_INPUT_GUTTER = 82;
  const ROOT_CONTENT_GUTTER = 144;
  const ROOT_OUTPUT_GUTTER = 112;
  const EDGE_COUNTER_WIDTH = 36;
  const PORT_LABEL_CLEARANCE = 16;
  const CORNER_RADIUS = 12;
  const MINIMUM_READABLE_SCALE = 0.85;

  function materializeGraph(topology) {
    const rootOwner = (topology.owners || []).find((owner) => owner.parent === null)?.id || "owner:0";
    const semanticNodes = (topology.nodes || []).map((node) => ({
      ...node,
      owner: node.owner || rootOwner,
      ports: node.ports || [],
    }));
    const nodes = new Map(semanticNodes.map((node) => [node.id, node]));
    // `-> nothing` is a semantic drain, not a destination that a reader can
    // inspect. Keep the source port visible, but do not invent a sink box or a
    // pipe which appears to carry an envelope somewhere.
    const edges = (topology.edges || []).filter((edge) => edge.target?.kind !== "discard");
    for (const edge of edges) {
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
    // Only a retained route with an inspectable destination becomes a pipe.
    // Unwired and explicitly drained outputs remain local contract ports.
    return { nodes: [...nodes.values()], byId: nodes, edges };
  }

  function nodeSubtitle(node) {
    const contract = node.kind === "connector"
      ? String(node.type || "Connector").split("/").at(-1).split("@")[0]
        .split(/[-_]/)
        .map(part => part ? `${part[0].toUpperCase()}${part.slice(1)}` : "")
        .join("")
      : "";
    return [contract, ...(node.extensions || []).map(extension => `+ ${extension}`)]
      .filter(Boolean).join(" · ");
  }

  function layoutGraph(nodes, edges, owners) {
    if (owners.length < 2) return layoutFlatGraph(nodes, edges, owners);
    const ownerMap = new Map(owners.map(owner => [owner.id, owner]));
    const root = owners.find(owner => owner.parent === null) || owners[0];
    const children = new Map(owners.map(owner => [owner.id, []]));
    for (const owner of owners) if (owner.parent) children.get(owner.parent)?.push(owner.id);
    const byId = new Map(nodes.map(node => [node.id, node]));
    const portions = new Map(), localEdges = new Map(owners.map(owner => [owner.id, []]));
    const drawingPorts = new Map(owners.map(owner => [owner.id, [...(owner.ports || [])]]));
    const boundary = (ownerId, direction, edge) => {
      const known = resolveScopePort(ownerMap.get(ownerId), direction, edge);
      // An unresolved flattened seam needs a routing attachment, not a new
      // public Graph port. Only the original owner ports are ever rendered.
      const name = known?.name || `routing:${edge.index}:${direction}`;
      if (!drawingPorts.get(ownerId).some(port => port.name === name && port.direction === direction)) {
        drawingPorts.get(ownerId).push({ name, direction });
      }
      return { id: `scope-boundary:${ownerId}:${direction}:${name}`, port: name, kind: direction, owner: ownerId };
    };
    for (const edge of edges) {
      let source = edge.source, target = edge.target;
      let sourceOwner = byId.get(source.id)?.owner || root.id;
      let targetOwner = byId.get(target.id)?.owner || root.id;
      const sourceChain = ownerChain(sourceOwner, ownerMap), targetChain = ownerChain(targetOwner, ownerMap);
      let depth = 0;
      while (sourceChain[depth] && sourceChain[depth] === targetChain[depth]) depth += 1;
      const common = sourceChain[depth - 1] || root.id;
      const before = [], after = [];
      while (sourceOwner !== common) {
        const pin = boundary(sourceOwner, "output", edge);
        before.push({ owner: sourceOwner, source, target: pin });
        source = { id: sourceOwner, port: pin.port, kind: "node" };
        sourceOwner = ownerMap.get(sourceOwner).parent;
      }
      while (targetOwner !== common) {
        const pin = boundary(targetOwner, "input", edge);
        after.unshift({ owner: targetOwner, source: pin, target });
        target = { id: targetOwner, port: pin.port, kind: "node" };
        targetOwner = ownerMap.get(targetOwner).parent;
      }
      const parts = [...before, { owner: common, source, target }, ...after];
      portions.set(edge.index, parts.map(part => part.owner));
      for (const part of parts) localEdges.get(part.owner).push({ ...edge, source: part.source, target: part.target });
    }
    const local = new Map();
    function arrange(ownerId) {
      const owner = ownerMap.get(ownerId);
      const direct = nodes.filter(node => node.owner === ownerId);
      for (const childId of children.get(ownerId)) {
        arrange(childId);
        const child = local.get(childId).layout, frame = child.scopes.get(childId);
        direct.push({ ...ownerMap.get(childId), id: childId, owner: ownerId, kind: "scope",
          ports: drawingPorts.get(childId), layoutSize: { width: frame.width, height: frame.height },
          fixedPorts: new Map([...child.scopePortAnchors].map(([key, point]) => [key,
            { x: point.x - frame.x, y: point.y - frame.y }])) });
      }
      const graph = materializeGraph({ nodes: direct, edges: localEdges.get(ownerId),
        owners: [{ ...owner, parent: null, ports: drawingPorts.get(ownerId) }] });
      const layout = layoutFlatGraph(graph.nodes, graph.edges,
        [{ ...owner, parent: null, ports: drawingPorts.get(ownerId) }]);
      local.set(ownerId, { graph, layout });
    }
    arrange(root.id);
    const result = { ...local.get(root.id).layout, positions: new Map(), scopes: new Map(),
      portAnchors: new Map(), scopePortAnchors: new Map(), owners: ownerMap,
      ownerChildren: children, compound: { local, portions } };
    function place(ownerId, x, y, depth) {
      const { layout } = local.get(ownerId), frame = layout.scopes.get(ownerId);
      frame.depth = depth;
      const dx = x - frame.x, dy = y - frame.y;
      for (const collection of [layout.positions, layout.scopes, layout.portAnchors, layout.scopePortAnchors]) {
        for (const point of collection.values()) { point.x += dx; point.y += dy; }
      }
      result.scopes.set(ownerId, frame);
      for (const [id, point] of layout.positions) result.positions.set(id, point);
      for (const [key, point] of layout.portAnchors) result.portAnchors.set(key, point);
      for (const [key, point] of layout.scopePortAnchors) result.scopePortAnchors.set(key, point);
      for (const childId of children.get(ownerId)) {
        const box = layout.positions.get(childId);
        place(childId, box.x, box.y, depth + 1);
      }
    }
    const rootBox = local.get(root.id).layout.scopes.get(root.id);
    place(root.id, rootBox.x, rootBox.y, 0);
    const columns = [...new Set(nodes.map(node => result.positions.get(node.id)?.x))].sort((a,b) => a-b);
    result.rank = new Map(nodes.map(node => [node.id, columns.indexOf(result.positions.get(node.id)?.x)]));
    return result;
  }

  function layoutFlatGraph(nodes, edges, owners) {
    const originalNodes = nodes;
    const byId = new Map(nodes.map((node, index) => [node.id, { node, index }]));
    const incoming = new Map(nodes.map((node) => [node.id, 0]));
    const predecessors = new Map(nodes.map((node) => [node.id, []]));
    const outgoing = new Map(nodes.map((node) => [node.id, []]));
    for (const edge of edges) {
      if (!byId.has(edge.source.id) || !byId.has(edge.target.id)) continue;
      outgoing.get(edge.source.id).push(edge.target.id);
      predecessors.get(edge.target.id).push(edge.source.id);
      incoming.set(edge.target.id, (incoming.get(edge.target.id) || 0) + 1);
    }

    const rank = new Map(nodes.map((node) => [node.id, 0]));
    const queue = nodes
      .filter((node) => incoming.get(node.id) === 0)
      .map((node) => node.id);
    const remaining = new Set(nodes.map(node => node.id));
    const rootId = owners.find(owner => owner.parent === null)?.id;
    const entryPriority = node => node.owner === rootId && node.kind === "connector"
      && /Inbound/.test(node.profile || "") ? 0 : 1;
    let cursor = 0;
    while (remaining.size) {
      if (cursor === queue.length) {
        // A cycle has no ready node. Prefer an inbound root Connector, then
        // the node reached furthest from the already placed flow. Its later
        // incoming edges become visual returns, never removed semantic pipes.
        const entry = nodes.filter(node => remaining.has(node.id)).sort((left, right) =>
          entryPriority(left) - entryPriority(right)
          || rank.get(right.id) - rank.get(left.id)
          || byId.get(left.id).index - byId.get(right.id).index)[0];
        queue.push(entry.id);
      }
      const source = queue[cursor++];
      if (!remaining.delete(source)) continue;
      for (const target of outgoing.get(source) || []) {
        if (!remaining.has(target)) continue;
        rank.set(target, Math.max(rank.get(target) || 0, (rank.get(source) || 0) + 1));
        incoming.set(target, (incoming.get(target) || 0) - 1);
        if (incoming.get(target) === 0) queue.push(target);
      }
    }

    // Public outputs are at the far side of the entire drawing, including
    // early exits. Their long pipes need dummy space past later Executors too.
    const outputRank = Math.max(0, ...nodes.filter(node => node.kind !== "boundary").map(node => rank.get(node.id))) + 1;
    for (const node of nodes) if (node.kind === "boundary" && node.direction === "output") rank.set(node.id, outputRank);
    for (const node of nodes) if (node.kind === "boundary" && node.direction === "input") rank.set(node.id, -1);
    const seedOrder = initialLayerOrder(nodes, edges, rank, predecessors, outgoing);
    const layered = properLayering(nodes, edges, rank);
    nodes = [...nodes, ...layered.dummies];
    for (const node of layered.dummies) byId.set(node.id, { node, index: byId.size });
    const groups = new Map();
    for (const node of nodes) {
      const level = rank.get(node.id) || 0;
      if (!groups.has(level)) groups.set(level, []);
      groups.get(level).push(node);
    }
    for (const group of groups.values()) {
      group.sort((left, right) => (seedOrder.get(left.id) ?? byId.get(left.id).index)
        - (seedOrder.get(right.id) ?? byId.get(right.id).index));
    }
    const maximumRank = Math.max(0, ...groups.keys());
    const fixedPortOrder = new Map();
    for (const node of nodes.filter(node => node.fixedPorts)) {
      for (const direction of ["input", "output"]) {
        const ports = node.ports.filter(port => port.direction === direction).sort((a,b) =>
          node.fixedPorts.get(portKey(node.id, direction, a.name)).y
          - node.fixedPorts.get(portKey(node.id, direction, b.name)).y);
        ports.forEach((port, index) => fixedPortOrder.set(portKey(node.id, direction, port.name),
          (index + 1) / (ports.length + 1)));
      }
    }

    const orderByNeighbors = (level, forward) => {
      const group = groups.get(level);
      if (!group || group.length < 2) return;
      const order = new Map();
      for (const layer of groups.values()) layer.forEach((node, index) => order.set(node.id, index));
      const centers = new Map();
      const barycenter = (node) => {
        if (centers.has(node.id)) return centers.get(node.id);
        // In-layer edges connect a real node to an inverted-port dummy.
        // Its adjacent-layer barycenter replaces the undefined in-layer rank
        // (Schulze et al., equation 6). Never recurse through the real node.
        const adjacent = (forward ? layered.incoming : layered.outgoing).get(node.id) || [];
        const coordinates = adjacent.map(endpoint => {
          if (rank.get(endpoint.id) === level) {
            return byId.get(endpoint.id)?.node.kind === "routing"
              ? barycenter(byId.get(endpoint.id).node) : null;
          }
          // Node-relative port ranking (equation 4), with FixedSide ports
          // sharing a rank until their final order is chosen.
          return (order.get(endpoint.id) ?? 0)
            + (fixedPortOrder.get(portKey(endpoint.id, endpoint.direction, endpoint.port)) ?? 0.5);
        }).filter(value => value !== null);
        const result = coordinates.length
          ? coordinates.reduce((sum, coordinate) => sum + coordinate, 0) / coordinates.length
          : null;
        centers.set(node.id, result);
        return result;
      };
      for (const node of group.filter(node => node.kind === "routing")) barycenter(node);
      group.sort((left, right) => {
        const leftCenter = barycenter(left);
        const rightCenter = barycenter(right);
        if (leftCenter !== null && rightCenter !== null && leftCenter !== rightCenter) {
          return leftCenter - rightCenter;
        }
        return order.get(left.id) - order.get(right.id);
      });
    };

    // Repeated deterministic sweeps are cheap for the small inspection graph
    // and keep source order as the final tie-break. The local swap pass accepts
    // only a strict reduction in crossings, so equivalent layouts never flap.
    const visibleCrossingBound = countLayerCrossings(groups, edges, rank);
    const crossingCost = () => {
      const visible = countLayerCrossings(groups, edges, rank);
      return Math.max(0, visible - visibleCrossingBound) * (layered.edges.length ** 2 + 1)
        + countLayerCrossings(groups, layered.edges, rank, fixedPortOrder);
    };
    let bestGroups = new Map([...groups].map(([level, group]) => [level, [...group]]));
    let bestCrossings = crossingCost();
    const initialGroups = new Map([...groups].map(([level, group]) => [level, [...group]]));
    for (let seed = 0; seed < 8; seed += 1) {
      for (const [level, group] of initialGroups) {
        const offset = (seed - 2) % Math.max(1, group.length);
        groups.set(level, seed === 1 ? [...group].reverse() : seed === 2
          ? [...group].sort((a,b) => Number(b.kind === "routing") - Number(a.kind === "routing"))
          : seed > 2 ? [...group.slice(offset), ...group.slice(0, offset)] : [...group]);
      }
      for (let pass = 0; pass < 12; pass += 1) {
        if ((pass + seed) % 2 === 0) {
          for (let level = 0; level <= maximumRank; level += 1) orderByNeighbors(level, true);
        } else {
          for (let level = maximumRank - 1; level >= -1; level -= 1) orderByNeighbors(level, false);
        }
        const crossings = crossingCost();
        if (crossings < bestCrossings) {
          bestCrossings = crossings;
          bestGroups = new Map([...groups].map(([level, group]) => [level, [...group]]));
        }
      }
    }
    for (const [level, group] of bestGroups) groups.set(level, group);
    minimizeAdjacentCrossings(groups, crossingCost);

    const rankOrder = new Map();
    for (const [level, group] of groups) {
      group.forEach((node, index) => rankOrder.set(node.id, [level, index]));
    }
    const dimensions = new Map(nodes.map((node) => {
      if (node.layoutSize) return [node.id, node.layoutSize];
      if (node.kind === "routing") return [node.id, { width: 0, height: 0 }];
      if (node.kind === "boundary" || node.kind === "discard") {
        const label = String(node.label || node.ports?.[0]?.name || "boundary");
        return [node.id, {
          width: Math.max(94, Math.min(250, 28 + label.length * 6.2)),
          height: 28,
        }];
      }
      const inputs = (node.ports || []).filter((port) => port.direction === "input").length;
      const outputs = (node.ports || []).filter((port) => port.direction === "output").length;
      return [node.id, {
        // Reserve the instance badge even before the first instance opens.
        // Admission must not move the title into a badge or reflow the graph.
        width: Math.max(126, String(node.label || node.path || node.id).length * 7 + 68,
          nodeSubtitle(node).length * 7 / MINIMUM_READABLE_SCALE + 20),
        height: Math.max(54, (Math.max(inputs, outputs) - 1) * PORT_SPACING + 36),
      }];
    }));
    for (const node of layered.dummies) {
      dimensions.get(node.id).width = Math.max(0, ...nodes
        .filter(other => other.owner === node.owner && rank.get(other.id) === rank.get(node.id)
          && other.kind !== "routing" && other.kind !== "boundary")
        .map(other => dimensions.get(other.id).width));
    }

    const columnGap = 128;
    const ownerMap = new Map(owners.map((owner, index) => [owner.id, { ...owner, index }]));
    const rootOwner = owners.find((owner) => owner.parent === null) || owners[0] || {
      id: "owner:0",
      parent: null,
      label: "App",
      ports: [],
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
      ownerNodes.sort((left, right) => {
        const [leftRank, leftOrder] = rankOrder.get(left.id) || [0, 0];
        const [rightRank, rightOrder] = rankOrder.get(right.id) || [0, 0];
        return leftRank - rightRank || leftOrder - rightOrder;
      });
    }

    function place(channelSpace) {
      // A child scope is a sized node in this local layer graph. Its internal
      // ranks have already been placed recursively and do not leak into ours.
      const ownerRankX = new Map();
      const columnGaps = new Map();
      for (const [ownerId, ownerNodes] of directNodes) {
        const levels = [...new Set(ownerNodes
          .filter(node => node.kind !== "boundary" && node.kind !== "discard")
          .map(node => rank.get(node.id)))].sort((a, b) => a - b);
        const coordinates = new Map();
        let x = 58;
        for (const level of levels) {
          coordinates.set(level, x);
          const ranked = ownerNodes.filter(node => rank.get(node.id) === level);
          const width = Math.max(...ranked.map(node => dimensions.get(node.id).width));
          const outputLabels = Math.max(0, ...ranked.filter(node => node.kind !== "scope").flatMap(node => (node.ports || [])
            .filter(port => port.direction === "output")
            .map(port => textWidth(port.display_label || port.name))));
          const nextLevel = levels[levels.indexOf(level) + 1];
          const inputLabels = Math.max(0, ...ownerNodes.filter(node => node.kind !== "scope" && rank.get(node.id) === nextLevel)
            .flatMap(node => (node.ports || []).filter(port => port.direction === "input")
              .map(port => textWidth(port.display_label || port.name))));
          const channelKey = `${ownerId}:${level}:${nextLevel}`;
          const tracks = channelSpace ? (channelSpace.get(channelKey)?.tracks || 0)
            + (channelSpace.get(`${ownerId}:${level}:east`)?.tracks || 0)
            + (channelSpace.get(`${ownerId}:${nextLevel}:west`)?.tracks || 0)
            : layered.edges.filter(edge => (rank.get(edge.source.id) === level
            && rank.get(edge.target.id) === nextLevel) || (rank.get(edge.source.id) === rank.get(edge.target.id)
              && [level, nextLevel].includes(rank.get(edge.source.id)))).length;
          const gap = Math.max(columnGap, outputLabels + inputLabels + 48 + tracks * TRACK_SPACING,
            channelSpace?.get(channelKey)?.gap || 0);
          columnGaps.set(channelKey, gap);
          x += width + gap;
        }
        ownerRankX.set(ownerId, coordinates);
      }

      // Leave routing tracks between bands as well as clearance at each box.
      const rowGap = BOX_EDGE_CLEARANCE * 2 + TRACK_SPACING * 2;
      const scopeGap = rowGap;
      const verticalPadding = SCOPE_PADDING + TRACK_SPACING;
      const measured = new Map();
      const directHeight = new Map();
      function measureOwner(ownerId) {
        if (measured.has(ownerId)) return measured.get(ownerId);
        const ownerGroups = new Map();
        for (const node of directNodes.get(ownerId) || []) {
          if (node.kind === "boundary" || node.kind === "discard") continue;
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
        const ownerPorts = ownerMap.get(ownerId)?.ports || [];
        const discardCount = ownerId === rootOwner.id
          ? nodes.filter((node) => node.kind === "discard").length
          : 0;
        const portBand = Math.max(
          ownerPorts.filter((port) => port.direction === "input").length,
          ownerPorts.filter((port) => port.direction === "output").length + discardCount,
        ) * PORT_SPACING;
        const height = SCOPE_HEADER + verticalPadding * 2
          + Math.max(36, portBand, bandHeight + contentGap + childHeight);
        measured.set(ownerId, height);
        return height;
      }
      measureOwner(rootOwner.id);

      const positions = new Map();
      const scopeVertical = new Map();
      function placeOwner(ownerId, top, depth) {
        const height = measured.get(ownerId);
        scopeVertical.set(ownerId, { y: top, height, depth });
        let cursorY = top + SCOPE_HEADER + verticalPadding;
        const bandHeight = directHeight.get(ownerId) || 0;
        const ownerGroups = new Map();
        for (const node of directNodes.get(ownerId) || []) {
          if (node.kind === "boundary" || node.kind === "discard") continue;
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
              x: ownerRankX.get(ownerId).get(level),
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
      for (const node of nodes) {
        if (positions.has(node.id)) continue;
        const size = dimensions.get(node.id);
        positions.set(node.id, { x: 0, y: 0, width: size.width, height: size.height });
      }

      const scopes = new Map();
      function placeHorizontalScope(ownerId) {
        const components = [];
        for (const node of directNodes.get(ownerId) || []) {
          if (node.kind === "boundary" || node.kind === "discard") continue;
          const position = positions.get(node.id);
          const outputLabel = node.kind === "scope" ? 0 : Math.max(0, ...(node.ports || [])
            .filter(port => port.direction === "output")
            .map(port => textWidth(port.display_label || port.name)));
          if (position) components.push({ left: position.x, right: position.x + position.width + outputLabel + 8 });
        }
        for (const childId of children.get(ownerId) || []) {
          placeHorizontalScope(childId);
          const child = scopes.get(childId);
          components.push({ left: child.x, right: child.x + child.width });
        }
        const hasInput = (ownerMap.get(ownerId)?.ports || []).some(port => port.direction === "input");
        const returnCount = edges.filter(edge => rank.get(edge.target.id) <= rank.get(edge.source.id)).length;
        const inputLabels = Math.max(0, ...nodes.flatMap(node => (node.ports || [])
          .filter(port => port.direction === "input").map(port => textWidth(port.display_label || port.name))));
        const returnPadding = returnCount ? FAN_STUB + inputLabels
          + returnCount * (TRACK_SPACING + 3) + BOX_EDGE_CLEARANCE : 0;
        const leftPadding = Math.max(returnPadding, hasInput
          ? ownerId === rootOwner.id ? ROOT_CONTENT_GUTTER : 64
          : SCOPE_PADDING * 2);
        let left = components.length ? Math.min(...components.map((component) => component.left)) - leftPadding : 42;
        const rootResultLabelWidth = Math.max(
          0,
          ...nodes
            .filter((node) => node.kind === "discard")
            .map((node) => textWidth(String(node.label || "Discard").replace(/^Discard · /, ""))),
        );
        const resultLaneWidth = rootResultLabelWidth
          + EDGE_COUNTER_WIDTH
          + PORT_LABEL_CLEARANCE * 2
          + 28;
        const rightPadding = Math.max(returnPadding, ownerId === rootOwner.id
          ? Math.max(ROOT_OUTPUT_GUTTER, resultLaneWidth)
          : 64);
        let right = components.length ? Math.max(...components.map((component) => component.right)) + rightPadding : left + 190;
        if (right - left < 190) {
          const extension = (190 - (right - left)) / 2;
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
      const rootHasInput = (rootOwner.ports || []).some(port => port.direction === "input");
      const shiftX = (rootHasInput ? ROOT_INPUT_GUTTER : SCOPE_PADDING * 2) - rootScope.x;
      if (shiftX !== 0) {
        for (const position of positions.values()) position.x += shiftX;
        for (const scope of scopes.values()) scope.x += shiftX;
      }

      const layout = {
        width: Math.max(...[...scopes.values()].map((scope) => scope.x + scope.width)) + 28,
        height: rootScope.y + rootScope.height + 28,
        positions,
        scopes,
        rank,
        layerOrder: new Map([...rankOrder].map(([id, [, order]]) => [id, order])),
        owners: ownerMap,
        ownerChildren: children,
        rootOwner: rootOwner.id,
        portAnchors: new Map(),
        scopePortAnchors: new Map(),
        edgeChains: layered.chains,
        columnGaps,
      };
      placeMovablePorts(nodes, edges, layout);
      // Once public boundary endpoints have moved, align child scope pins with
      // the same final producer/consumer ports instead of their initial boxes.
      placeMovablePorts(nodes, edges, layout);
      return layout;
    }

    // Port heights and row order are fixed before measuring routing demand.
    // Only placement runs twice; layer assignment and crossing sweeps run once.
    const measuredSpace = new Map();
    routeLocalEdges({ nodes: originalNodes, edges,
      byId: new Map(originalNodes.map(node => [node.id, node])) }, place(null), measuredSpace);
    return place(measuredSpace);
  }

  // Proper layering precedes crossing minimization and placement. These
  // vertices belong to the drawing only, never to the validated topology.
  // Regular long edges get one dummy in every intermediate layer. A return
  // gets an inverted-port dummy at each end as well (Schulze et al., §4.1).
  function properLayering(nodes, edges, rank) {
    const byId = new Map(nodes.map(node => [node.id, node]));
    const ownerLayers = new Map();
    for (const node of nodes) {
      if (!ownerLayers.has(node.owner)) ownerLayers.set(node.owner, new Set());
      ownerLayers.get(node.owner).add(rank.get(node.id));
    }
    const dummies = [], segments = [], chains = new Map();
    const dummy = (edge, level, owner, side) => {
      const id = `routing:${edge.index}:${level}:${side}`;
      const node = { id, owner, kind: "routing", ports: [] };
      dummies.push(node);
      rank.set(id, level);
      return id;
    };
    for (const edge of edges) {
      const source = byId.get(edge.source.id), target = byId.get(edge.target.id);
      if (!source || !target) continue;
      const first = rank.get(source.id), last = rank.get(target.id);
      const chain = [source.id];
      if (source.owner === target.owner) {
        if (last <= first) chain.push(dummy(edge, first, source.owner, "source"));
        const direction = last > first ? 1 : -1;
        for (const level of [...ownerLayers.get(source.owner)].sort((a,b) => direction * (a-b))) {
          if (direction * (level - first) <= 0 || direction * (last - level) <= 0) continue;
          chain.push(dummy(edge, level, source.owner, "long"));
        }
        if (last <= first) chain.push(dummy(edge, last, target.owner, "target"));
      }
      chain.push(target.id);
      chains.set(edge.index, chain);
      for (let index = 1; index < chain.length; index += 1) {
        const a = chain[index - 1], b = chain[index];
        const reversed = first > last;
        const from = { id: a, ...(a === source.id ? { port: edge.source.port, direction: "output" } : {}) };
        const to = { id: b, ...(b === target.id ? { port: edge.target.port, direction: "input" } : {}) };
        segments.push({ source: reversed ? to : from, target: reversed ? from : to });
      }
    }
    const incoming = new Map(), outgoing = new Map();
    for (const edge of segments) {
      addSample(incoming, edge.target.id, edge.source);
      addSample(outgoing, edge.source.id, edge.target);
    }
    return { dummies, edges: segments, chains, incoming, outgoing };
  }

  // A deterministic initial sweep on the unsplit graph retains the ordering
  // of the visible flow as a candidate; proper-layer sweeps may improve it.
  function initialLayerOrder(nodes, edges, rank, predecessors, successors) {
    const groups = new Map(), indices = new Map(nodes.map((node, index) => [node.id, index]));
    for (const node of nodes) addSample(groups, rank.get(node.id), node);
    const maximum = Math.max(0, ...groups.keys());
    const sweep = (level, neighbors) => {
      const group = groups.get(level);
      if (!group || group.length < 2) return;
      const order = layerOrder(groups);
      const center = node => {
        const adjacent = neighbors.get(node.id) || [];
        return adjacent.length ? adjacent.reduce((sum, id) => sum + order.get(id), 0) / adjacent.length : null;
      };
      group.sort((a, b) => {
        const left = center(a), right = center(b);
        return left !== null && right !== null && left !== right ? left - right : indices.get(a.id) - indices.get(b.id);
      });
    };
    for (let pass = 0; pass < 6; pass += 1) {
      for (let level = 1; level <= maximum; level += 1) sweep(level, predecessors);
      for (let level = maximum - 1; level >= 0; level -= 1) sweep(level, successors);
    }
    minimizeAdjacentCrossings(groups, () => countLayerCrossings(groups, edges, rank));
    return new Map([...groups.values()].flatMap(group => group.map((node, index) => [node.id, index])));
  }

  function layerOrder(groups) {
    const order = new Map();
    for (const [level, ranked] of groups) {
      ranked.forEach((node, index) => order.set(node.id, level * 1_000_000 + index));
    }
    return order;
  }

  function minimizeAdjacentCrossings(groups, crossingCost) {
    let best = crossingCost();
    const levels = [...groups.keys()].sort((left, right) => left - right);
    for (let pass = 0; pass < 4; pass += 1) {
      let changed = false;
      for (const level of levels) {
        const group = groups.get(level);
        for (let index = 0; index + 1 < group.length; index += 1) {
          [group[index], group[index + 1]] = [group[index + 1], group[index]];
          const candidate = crossingCost();
          if (candidate < best) {
            best = candidate;
            changed = true;
          } else {
            [group[index], group[index + 1]] = [group[index + 1], group[index]];
          }
        }
      }
      if (!changed) break;
    }
  }

  function countLayerCrossings(groups, edges, rank, portOrder) {
    const order = new Map();
    for (const group of groups.values()) {
      group.forEach((node, index) => order.set(node.id, index));
    }
    const buckets = new Map();
    for (const edge of edges) {
      if (!order.has(edge.source.id) || !order.has(edge.target.id)) continue;
      const sourceRank = rank.get(edge.source.id) || 0;
      const targetRank = rank.get(edge.target.id) || 0;
      const key = `${sourceRank}:${targetRank}`;
      if (!buckets.has(key)) buckets.set(key, []);
      buckets.get(key).push({
        source: order.get(edge.source.id) + (portOrder?.get(portKey(edge.source.id, edge.source.direction, edge.source.port)) ?? 0.5),
        target: order.get(edge.target.id) + (portOrder?.get(portKey(edge.target.id, edge.target.direction, edge.target.port)) ?? 0.5),
      });
    }
    let crossings = 0;
    for (const bucket of buckets.values()) {
      for (let left = 0; left < bucket.length; left += 1) {
        for (let right = left + 1; right < bucket.length; right += 1) {
          const sourceOrder = bucket[left].source - bucket[right].source;
          const targetOrder = bucket[left].target - bucket[right].target;
          if (sourceOrder * targetOrder < 0) crossings += 1;
        }
      }
    }
    return crossings;
  }

  function placeMovablePorts(nodes, edges, layout) {
    const byId = new Map(nodes.map((node) => [node.id, node]));
    const nodePortSamples = () => {
      const samples = new Map();
      for (const edge of edges) {
        const sourcePosition = layout.positions.get(edge.source.id);
        const targetPosition = layout.positions.get(edge.target.id);
        if (!sourcePosition || !targetPosition) continue;
        const sourceKey = portKey(edge.source.id, "output", edge.source.port);
        const targetKey = portKey(edge.target.id, "input", edge.target.port);
        const chain = layout.edgeChains.get(edge.index) || [];
        const sourceDummy = chain.length > 2 ? layout.positions.get(chain[1]) : null;
        const targetDummy = chain.length > 2 ? layout.positions.get(chain.at(-2)) : null;
        // Long-edge and inverted-port dummies participated in the sweep.
        // Their positions, not a local detour preference, now order the pins.
        addSample(samples, sourceKey,
          sourceDummy ? boxCenterY(sourceDummy) : (layout.portAnchors.get(targetKey)?.y ?? boxCenterY(targetPosition)));
        addSample(samples, targetKey,
          targetDummy ? boxCenterY(targetDummy) : (layout.portAnchors.get(sourceKey)?.y ?? boxCenterY(sourcePosition)));
      }
      return samples;
    };
    allocateNodePorts(nodes, layout, nodePortSamples());

    const descendantOwners = ownerDescendants(layout.owners, layout.ownerChildren);
    const ownerSamples = new Map();
    for (const [ownerId, owner] of layout.owners) {
      const descendants = descendantOwners.get(ownerId) || new Set([ownerId]);
      for (const edge of edges) {
        const source = byId.get(edge.source.id);
        const target = byId.get(edge.target.id);
        if (!source || !target) continue;
        if (source.kind === "boundary" && (source.owner || layout.rootOwner) === ownerId) {
          const direction = source.direction || "input";
          const port = resolveScopePort(owner, direction, edge);
          const targetPosition = layout.positions.get(target.id);
          if (port && targetPosition) {
            const chain = layout.edgeChains.get(edge.index) || [];
            const dummy = chain.length > 2 ? layout.positions.get(chain[1]) : null;
            addSample(
              ownerSamples,
              scopePortKey(ownerId, direction, port.name),
              dummy ? dummy.y : (layout.portAnchors.get(portKey(target.id, "input", edge.target.port))?.y ?? boxCenterY(targetPosition)),
            );
          }
        }
        if (target.kind === "boundary" && (target.owner || layout.rootOwner) === ownerId) {
          const direction = target.direction || "output";
          const port = resolveScopePort(owner, direction, edge);
          const sourcePosition = layout.positions.get(source.id);
          if (port && sourcePosition) {
            const chain = layout.edgeChains.get(edge.index) || [];
            const dummy = chain.length > 2 ? layout.positions.get(chain.at(-2)) : null;
            addSample(
              ownerSamples,
              scopePortKey(ownerId, direction, port.name),
              dummy ? dummy.y : (layout.portAnchors.get(portKey(source.id, "output", edge.source.port))?.y ?? boxCenterY(sourcePosition)),
            );
          }
        }
        const sourceInside = descendants.has(source.owner || layout.rootOwner);
        const targetInside = descendants.has(target.owner || layout.rootOwner);
        const direction = !sourceInside && targetInside
          ? "input"
          : sourceInside && !targetInside
            ? "output"
            : null;
        if (!direction) continue;
        const port = resolveScopePort(owner, direction, edge);
        if (!port) continue;
        const sourcePosition = layout.positions.get(source.id);
        const targetPosition = layout.positions.get(target.id);
        if (!sourcePosition || !targetPosition) continue;
        const inner = direction === "input" ? target : source;
        const innerPort = direction === "input" ? edge.target.port : edge.source.port;
        const innerAnchor = layout.portAnchors.get(portKey(inner.id, direction, innerPort));
        addSample(
          ownerSamples,
          scopePortKey(ownerId, direction, port.name),
          innerAnchor?.y ?? boxCenterY(direction === "input" ? targetPosition : sourcePosition),
        );
      }
    }
    for (const [ownerId, owner] of layout.owners) {
      const scope = layout.scopes.get(ownerId);
      if (!scope) continue;
      for (const direction of ["input", "output"]) {
        const ports = (owner.ports || []).filter((port) => port.direction === direction);
        const anchors = allocateSideItems(
          ports,
          scope.y + SCOPE_HEADER + 18,
          scope.y + scope.height - 18,
          (port, index) => medianOr(
            ownerSamples.get(scopePortKey(ownerId, direction, port.name)),
            centeredSideY(scope, index, ports.length),
          ),
          (port) => port.name,
          (port) => {
            const node = nodes.find(node => node.kind === "boundary" && node.owner === ownerId
              && node.direction === direction && node.ports[0]?.name === port.name);
            return layout.layerOrder.get(node?.id);
          },
        );
        for (const [port, y] of anchors) {
          layout.scopePortAnchors.set(scopePortKey(ownerId, direction, port.name), {
            x: direction === "input" ? scope.x : scope.x + scope.width,
            y,
          });
        }
      }
    }

    // Every public result and explicit discard lands in the root Graph's
    // reserved output column. A nested scope therefore encloses its work, not
    // an unrelated visual sink, while the semantic owner on each edge stays
    // untouched.
    const discards = nodes.filter((node) => node.kind === "discard");
    const rootScope = layout.scopes.get(layout.rootOwner);
    const root = layout.owners.get(layout.rootOwner);
    const outputItems = [
      ...(root?.ports || [])
        .filter((port) => port.direction === "output")
        .map((value) => ({ kind: "port", value })),
      ...discards.map((value) => ({ kind: "discard", value })),
    ];
    const outputAnchors = allocateSideItems(
      outputItems,
      rootScope.y + SCOPE_HEADER + 18,
      rootScope.y + rootScope.height - 18,
      (item, index) => item.kind === "port"
        ? medianOr(
          ownerSamples.get(scopePortKey(root.id, "output", item.value.name)),
          centeredSideY(rootScope, index, outputItems.length),
        )
        : medianOr(
          nodeSamples.get(portKey(item.value.id, "input", null)),
          centeredSideY(rootScope, index, outputItems.length),
        ),
      (item) => `${item.kind}:${item.kind === "port" ? item.value.name : item.value.id}`,
      (item) => {
        const node = nodes.find(node => node.kind === "boundary" && node.owner === root.id
          && node.direction === "output" && node.ports[0]?.name === item.value.name);
        return layout.layerOrder.get(node?.id);
      },
    );
    for (const [item, y] of outputAnchors) {
      if (item.kind === "port") {
        layout.scopePortAnchors.set(scopePortKey(root.id, "output", item.value.name), {
          x: rootScope.x + rootScope.width,
          y,
        });
      } else {
        const position = layout.positions.get(item.value.id);
        position.x = rootScope.x + rootScope.width - 10;
        position.y = y - position.height / 2;
      }
    }

    // Boundary pseudo-nodes are routing endpoints only. Align them after the
    // shared output band has selected its final pin positions.
    for (const node of nodes.filter((candidate) => candidate.kind === "boundary")) {
      const position = layout.positions.get(node.id);
      const port = node.ports?.[0];
      const owner = layout.owners.get(node.owner || layout.rootOwner);
      const scope = layout.scopes.get(node.owner || layout.rootOwner);
      if (!position || !port || !owner || !scope) continue;
      const anchor = scopePortAnchor(owner, scope, port, layout);
      position.y = anchor.y - position.height / 2;
      position.x = port.direction === "input"
        ? scope.x - position.width + 10
        : scope.x + scope.width - 10;
    }

    // Boundary movement changes the useful target of adjacent Node ports.
    // Refresh from the retained opposite anchors after that movement.
    // Only port positions, never columns or body positions, change here.
    allocateNodePorts(nodes, layout, nodePortSamples());
  }

  function allocateNodePorts(nodes, layout, samples) {
    layout.portAnchors.clear();
    for (const node of nodes) {
      if (node.kind === "boundary" || node.kind === "discard") continue;
      const position = layout.positions.get(node.id);
      if (!position) continue;
      if (node.fixedPorts) {
        for (const [key, point] of node.fixedPorts) layout.portAnchors.set(key,
          { x: position.x + point.x, y: position.y + point.y });
        continue;
      }
      for (const direction of ["input", "output"]) {
        const ports = (node.ports || []).filter((port) => port.direction === direction);
        const anchors = allocateSideItems(
          ports,
          position.y + 14,
          position.y + position.height - 12,
          (port, index) => medianOr(
            samples.get(portKey(node.id, direction, port.name)),
            position.y + position.height * (index + 1) / (ports.length + 1),
          ),
          (port) => port.name,
        );
        for (const [port, y] of anchors) {
          layout.portAnchors.set(portKey(node.id, direction, port.name), {
            x: direction === "input" ? position.x : position.x + position.width,
            y,
          });
        }
      }
    }
  }

  function allocateSideItems(items, minimum, maximum, desired, stableKey, order) {
    if (!items.length) return [];
    const ranked = items.map((item, index) => ({
      item,
      index,
      desired: desired(item, index),
      key: stableKey(item),
      order: order?.(item),
    }));
    ranked.sort((left, right) => (left.order !== undefined && right.order !== undefined ? left.order - right.order : 0)
      || left.desired - right.desired
      || String(left.key).localeCompare(String(right.key))
      || left.index - right.index);
    const spacing = Math.min(PORT_SPACING, (maximum - minimum) / Math.max(1, ranked.length - 1));
    // Sort barycenters before clamping coordinates. Otherwise all neighbors
    // below (or above) the body collapse to one rank and declaration/name
    // order silently replaces the crossing-minimizing port order.
    const values = ranked.map((entry) => clamp(entry.desired, minimum, maximum));
    for (let index = 1; index < values.length; index += 1) {
      values[index] = Math.max(values[index], values[index - 1] + spacing);
    }
    if (values.at(-1) > maximum) {
      values[values.length - 1] = maximum;
      for (let index = values.length - 2; index >= 0; index -= 1) {
        values[index] = Math.min(values[index], values[index + 1] - spacing);
      }
    }
    if (values[0] < minimum) {
      const shift = minimum - values[0];
      for (let index = 0; index < values.length; index += 1) values[index] += shift;
    }
    return ranked.map((entry, index) => [entry.item, values[index]]);
  }

  function routeGraphEdges(graph, layout) {
    let routes;
    if (layout.compound) {
      const localRoutes = new Map([...layout.compound.local].map(([id, item]) =>
        [id, routeLocalEdges(item.graph, item.layout)]));
      routes = new Map();
      for (const edge of graph.edges) {
        const points = [], presentationPoints = [];
        for (const owner of layout.compound.portions.get(edge.index)) {
          const part = localRoutes.get(owner).get(edge.index);
          appendPoints(points, part.points);
          appendPoints(presentationPoints, part.presentationPoints);
        }
        const source = graph.byId.get(edge.source.id), target = graph.byId.get(edge.target.id);
        routes.set(edge.index, { start: points[0], end: points.at(-1),
          points: compactOrthogonalPoints(points), presentationPoints: compactOrthogonalPoints(presentationPoints),
          portals: routingPortals(edge, source, target, layout) });
      }
    } else routes = routeLocalEdges(graph, layout);
    placeEdgeLabels(routes, graph, layout);
    growLayoutForRoutes(routes, layout);
    return routes;
  }

  function routeLocalEdges(graph, layout, channelSpace) {
    const candidates = [];
    for (const edge of graph.edges) {
      const source = graph.byId.get(edge.source.id);
      const target = graph.byId.get(edge.target.id);
      const sourcePosition = layout.positions.get(edge.source.id);
      const targetPosition = layout.positions.get(edge.target.id);
      if (!source || !target || !sourcePosition || !targetPosition) continue;
      const sourcePort = portAnchor(source, sourcePosition, edge.source.port, "output", layout);
      const targetPort = portAnchor(target, targetPosition, edge.target.port, "input", layout);
      candidates.push({
        edge,
        source,
        target,
        start: { x: sourcePort.x, y: sourcePort.y },
        end: { x: targetPort.x, y: targetPort.y },
      });
    }
    const sourceFans = fanOffsets(candidates, true, layout);
    const targetFans = fanOffsets(candidates, false, layout);
    const labelObstacles = graphLabelObstacles(graph, layout);
    const planned = channelRoutes(candidates, sourceFans, targetFans, layout, labelObstacles, channelSpace);
    const routes = new Map();
    const ordered = [...candidates].sort((left, right) =>
      left.start.x - right.start.x
      || left.end.x - right.end.x
      || left.edge.index - right.edge.index);

    for (const candidate of ordered) {
      const compact = planned.get(candidate.edge.index);
      const route = {
        start: candidate.start,
        end: candidate.end,
        points: compact,
        // Every pipe uses the allocated tracks. Only the corners are rounded;
        // a clear span never switches to a different presentation geometry.
        presentationPoints: compact.map(point => ({ ...point })),
        portals: [],
      };
      routes.set(candidate.edge.index, route);
    }
    return routes;
  }

  function fanOffsets(candidates, source, layout) {
    const groups = new Map();
    for (const candidate of candidates) {
      const endpoint = source ? candidate.edge.source : candidate.edge.target;
      const key = `${endpoint.id}\u0000${endpoint.port || ""}`;
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key).push(candidate);
    }
    const offsets = new Map();
    const adjacentY = candidate => {
      const chain = layout.edgeChains.get(candidate.edge.index) || [];
      const dummy = chain.length > 2 ? layout.positions.get(source ? chain[1] : chain.at(-2)) : null;
      return dummy?.y ?? (source ? candidate.end.y : candidate.start.y);
    };
    for (const group of groups.values()) {
      group.sort((left, right) => {
        return adjacentY(left) - adjacentY(right) || left.edge.index - right.edge.index;
      });
      group.forEach((candidate, index) => offsets.set(candidate.edge.index, {
        offset: (index - (group.length - 1) / 2) * TRACK_SPACING,
        count: group.length,
        index,
      }));
    }
    return offsets;
  }

  function channelRoutes(candidates, sourceFans, targetFans, layout, labels, channelSpace) {
    const channels = new Map(), sideChannels = new Map(), paths = new Map();
    const sideConnect = (parts, from, to, key, direction) => {
      const span = { from, to, direction };
      parts.push(span);
      addSample(sideChannels, key, span);
    };
    const connect = (parts, from, to, key, sideKeys = []) => {
      if (from.y === to.y) { parts.push([from, to]); return; }
      const span = { from, to, order: parts.length, key, sideKeys };
      parts.push(span);
      if (!channels.has(key)) channels.set(key, []);
      channels.get(key).push(span);
    };
    for (const candidate of candidates) {
      const { start, end, edge, source, target } = candidate;
      const sourceFan = sourceFans.get(edge.index), targetFan = targetFans.get(edge.index);
      const labelKey = (node, direction, port) => node.kind === "boundary"
        ? scopePortKey(node.owner, node.direction, node.ports[0].name) : portKey(node.id, direction, port);
      const sourceLabel = labels.find(box => box.portKey === labelKey(source, "output", edge.source.port));
      const targetLabel = labels.find(box => box.portKey === labelKey(target, "input", edge.target.port));
      const scopeExit = source.kind === "scope" && target.kind === "boundary"
        && target.direction === "output" && start.x < end.x;
      // A sibling pipe may fan out at this scope pin, but the final arrival
      // at the parent output must not overshoot either endpoint vertically.
      const fanY = y => scopeExit ? clamp(y, Math.min(start.y, end.y), Math.max(start.y, end.y)) : y;
      const parts = [], first = { x: Math.max(start.x + FAN_STUB, (sourceLabel?.x ?? start.x)
        + (sourceLabel?.width ?? 0) + BOX_EDGE_CLEARANCE) + sourceFan.index * 3, y: fanY(start.y + sourceFan.offset) };
      parts.push([start, { x: first.x, y: start.y }, first]);
      let current = first;
      const chain = layout.edgeChains.get(edge.index) || [];
      const forward = layout.rank.get(source.id) < layout.rank.get(target.id);
      for (let index = 1; index + 1 < chain.length; index += 1) {
        const box = layout.positions.get(chain[index]);
        const entry = { x: forward ? box.x : box.x + box.width, y: box.y };
        const leave = { x: forward ? box.x + box.width : box.x, y: box.y };
        const a = layout.rank.get(chain[index - 1]), b = layout.rank.get(chain[index]);
        const key = `${source.owner}:${Math.min(a, b)}:${Math.max(a, b)}`;
        if (a === b) {
          sideConnect(parts, current, entry, `${source.owner}:${a}:east`, 1);
        } else connect(parts, current, entry, key,
          [`${source.owner}:${Math.min(a, b)}:east`, `${source.owner}:${Math.max(a, b)}:west`]);
        parts.push([entry, leave]);
        current = leave;
      }
      const last = { x: Math.min(end.x - FAN_STUB, (targetLabel?.x ?? end.x) - BOX_EDGE_CLEARANCE)
        - targetFan.index * 3, y: fanY(end.y + targetFan.offset) };
      const a = layout.rank.get(chain.at(-2)), b = layout.rank.get(target.id);
      if (chain.length > 2 && a === b) {
        sideConnect(parts, current, last, `${target.owner}:${a}:west`, -1);
      } else {
        connect(parts, current, last, source.owner === target.owner
          ? `${source.owner}:${Math.min(a, b)}:${Math.max(a, b)}` : `portal:${current.x}:${last.x}`,
        source.owner === target.owner
          ? [`${source.owner}:${Math.min(a, b)}:east`, `${source.owner}:${Math.max(a, b)}:west`] : []);
      }
      parts.push([last, { x: last.x, y: end.y }, end]);
      paths.set(edge.index, parts);
    }
    for (const [key, spans] of sideChannels) {
      channelSpace?.set(key, { tracks: spans.length });
      const direction = spans[0].direction;
      const base = direction > 0 ? Math.max(...spans.flatMap(span => [span.from.x, span.to.x]))
        : Math.min(...spans.flatMap(span => [span.from.x, span.to.x]));
      spans.sort((a,b) => Math.abs(a.to.y-a.from.y) - Math.abs(b.to.y-b.from.y));
      spans.forEach((span,index) => { span.x = base + direction * (index + 1) * TRACK_SPACING; });
    }
    for (const [key, spans] of channels) {
      // Channel precedence is determined jointly, before assigning any x.
      // A horizontal endpoint inside another edge's vertical interval must
      // not cross that edge's track. Conflicting preferences form cycles;
      // remove those deterministically by weighted out-degree minus in-degree.
      const normalized = spans.map((span, index) => ({ span, index,
        left: span.from.x < span.to.x ? span.from : span.to,
        right: span.from.x < span.to.x ? span.to : span.from,
        before: new Map(), incoming: 0 }));
      const inside = (y, a, b) => y > Math.min(a, b) && y < Math.max(a, b);
      const cost = (a, b) => Number(inside(b.left.y, a.left.y, a.right.y))
        + Number(inside(a.right.y, b.left.y, b.right.y))
        + (a.right.y === b.left.y ? 1000 : 0);
      for (let a = 0; a < normalized.length; a += 1) {
        for (let b = a + 1; b < normalized.length; b += 1) {
          const left = normalized[a], right = normalized[b];
          const difference = cost(left, right) - cost(right, left);
          if (!difference) continue;
          const before = difference < 0 ? left : right, after = difference < 0 ? right : left;
          before.before.set(after, Math.abs(difference));
          after.incoming += Math.abs(difference);
        }
      }
      const pending = new Set(normalized), ordered = [];
      while (pending.size) {
        const ready = [...pending].filter(item => item.incoming === 0);
        const eligible = ready.length ? ready : [...pending];
        const balance = item => [...item.before].filter(([after]) => pending.has(after))
          .reduce((sum, [, weight]) => sum + weight, 0) - item.incoming;
        eligible.sort((a, b) => (ready.length ? 0 : balance(b) - balance(a))
          || a.left.y - b.left.y || a.right.y - b.right.y || a.index - b.index);
        const next = eligible[0];
        ordered.push(next);
        pending.delete(next);
        for (const [after, weight] of next.before) after.incoming -= weight;
      }
      // Inverted ports already occupy the sides of a compacted column gap.
      // Forward bends must stay beyond them when its columns move closer.
      // Boundary gutters are not compacted and keep their existing routing.
      const gap = layout.columnGaps.get(key);
      const sideKeys = gap === undefined ? [] : spans[0].sideKeys;
      const minimum = Math.max(...normalized.map(item => item.left.x),
        ...(sideChannels.get(sideKeys[0]) || []).map(span => span.x));
      const right = Math.min(...normalized.map(item => item.right.x),
        ...(sideChannels.get(sideKeys[1]) || []).map(span => span.x));
      // Resolve opposing horizontal runs before sharing tracks: their short
      // receiving jog extends the vertical interval that needs clearance.
      const ordinates = new Set(normalized.flatMap(item => [item.left.y, item.right.y]));
      for (let index = 0; index < ordered.length; index += 1) {
        const item = ordered[index];
        if (!ordered.slice(index + 1).some(other => item.right.y === other.left.y)) continue;
        let delta = TRACK_SPACING / 2;
        while (ordinates.has(item.right.y + delta)) delta += TRACK_SPACING / 2;
        item.jogY = item.right.y + delta;
        ordinates.add(item.jogY);
      }
      // Preserve the chosen precedence for overlapping intervals. Disjoint
      // vertical runs can reuse an x track, leaving room for rounded corners.
      for (let index = 0; index < ordered.length; index += 1) {
        const item = ordered[index];
        item.track = 0;
        for (const previous of ordered.slice(0, index)) {
          const separated = Math.max(item.left.y, item.right.y, item.jogY ?? item.right.y) + CORNER_RADIUS * 2
            <= Math.min(previous.left.y, previous.right.y)
            || Math.max(previous.left.y, previous.right.y, previous.jogY ?? previous.right.y) + CORNER_RADIUS * 2
              <= Math.min(item.left.y, item.right.y);
          if (!separated) item.track = Math.max(item.track, previous.track + 1);
        }
      }
      const trackCount = Math.max(...ordered.map(item => item.track)) + 1;
      // The track count alone omits fan-out stubs and inverted-port turns.
      // Keep their measured footprint when compacting this column gap.
      channelSpace?.set(key, { tracks: trackCount, gap: gap === undefined ? 0
        : Math.ceil(gap - (right - minimum) + TRACK_SPACING * (trackCount + 1) - 1e-6) });
      // Keep a straight receiving span long enough for a mailbox badge when
      // the channel permits it, without squeezing its ordered tracks together.
      const badgeWidth = Math.max(EDGE_COUNTER_WIDTH, textWidth("≡ 0000") + 8);
      const maximum = Math.min(right, Math.max(minimum + TRACK_SPACING * (trackCount + 1), right - badgeWidth));
      for (const { span, track } of ordered) {
        span.x = minimum + (maximum - minimum) * (track + 1) / (trackCount + 1);
      }
      for (const item of ordered) {
        if (item.jogY === undefined) continue;
        const jog = { x: item.right.x, y: item.jogY };
        const points = [item.left, { x: item.span.x, y: item.left.y },
          { x: item.span.x, y: jog.y }, jog, item.right];
        item.span.points = item.span.from === item.left ? points : points.reverse();
      }
    }
    return new Map([...paths].map(([index, parts]) => [index, compactOrthogonalPoints(parts.flatMap(part =>
      Array.isArray(part) ? part : part.points || [part.from, { x: part.x, y: part.from.y },
        { x: part.x, y: part.to.y }, part.to]))]));
  }

  function routingPortals(edge, source, target, layout) {
    const sourceChain = ownerChain(source.owner || layout.rootOwner, layout.owners);
    const targetChain = ownerChain(target.owner || layout.rootOwner, layout.owners);
    let common = 0;
    while (common < sourceChain.length
      && common < targetChain.length
      && sourceChain[common] === targetChain[common]) {
      common += 1;
    }
    const portals = [];
    for (let index = sourceChain.length - 1; index >= common; index -= 1) {
      const owner = layout.owners.get(sourceChain[index]);
      const scope = layout.scopes.get(sourceChain[index]);
      const port = owner && resolveScopePort(owner, "output", edge);
      if (!owner || !scope || !port) continue;
      const anchor = scopePortAnchor(owner, scope, port, layout);
      portals.push({ ...anchor, owner: owner.id, port: port.name, direction: "output" });
    }
    for (let index = common; index < targetChain.length; index += 1) {
      const owner = layout.owners.get(targetChain[index]);
      const scope = layout.scopes.get(targetChain[index]);
      const port = owner && resolveScopePort(owner, "input", edge);
      if (!owner || !scope || !port) continue;
      const anchor = scopePortAnchor(owner, scope, port, layout);
      portals.push({ ...anchor, owner: owner.id, port: port.name, direction: "input" });
    }
    return portals;
  }


  function graphLabelObstacles(graph, layout) {
    const portLabelObstacles = [];
    for (const [ownerId, owner] of layout.owners) {
      const scope = layout.scopes.get(ownerId);
      if (!scope) continue;
      for (const port of owner.ports || []) {
        const anchor = scopePortAnchor(owner, scope, port, layout);
        portLabelObstacles.push({ ...textLabelBox(
          anchor.x + (port.direction === "input" ? 9 : -9),
          anchor.y - 5,
          port.display_label || port.name,
          port.direction === "input" ? "start" : "end",
        ), portKey: scopePortKey(ownerId, port.direction, port.name) });
      }
    }
    for (const node of graph.nodes) {
      const position = layout.positions.get(node.id);
      // Child-scope port labels are drawn inside its frame, not outside its
      // parent-level stand-in. The child's own drawing reserves their space.
      if (!position || node.kind === "boundary" || node.kind === "scope") continue;
      if (node.kind === "discard") {
        const anchor = portAnchor(node, position, null, "input", layout);
        const label = String(node.label || "Discard").replace(/^Discard · /, "");
        portLabelObstacles.push(textLabelBox(anchor.x - 12, anchor.y - 5, label, "end"));
        continue;
      }
      for (const port of node.ports || []) {
        const anchor = portAnchor(node, position, port.name, port.direction, layout);
        portLabelObstacles.push({ ...textLabelBox(
          anchor.x + (port.direction === "input" ? -8 : 8),
          anchor.y - 5,
          port.display_label || port.name,
          port.direction === "input" ? "end" : "start",
        ), portKey: portKey(node.id, port.direction, port.name) });
      }
    }
    return portLabelObstacles;
  }


  function placeEdgeLabels(routes, graph, layout) {
    const nodeObstacles = graph.nodes
      .filter((node) => node.kind !== "boundary" && node.kind !== "discard")
      .map((node) => expandBox(layout.positions.get(node.id), 6))
      .filter(Boolean);
    const headerObstacles = [...layout.scopes.values()].map((scope) => ({
      x: scope.x,
      y: scope.y,
      width: scope.width,
      height: SCOPE_HEADER,
    }));
    const portLabelObstacles = graphLabelObstacles(graph, layout);
    const routeSamples = new Map([...routes.entries()].map(([index, route]) => [
      index,
      sampleCurvedCorridor(route.presentationPoints, CORNER_RADIUS, 12, 4),
    ]));
    // A label only collides with samples in its immediate neighborhood.
    // Index once instead of rescanning every curve for each candidate label.
    const sampleCells = new Map();
    const cellSize = 32;
    for (const [route, samples] of routeSamples) {
      for (const point of samples) addSample(sampleCells,
        `${Math.floor(point.x / cellSize)}:${Math.floor(point.y / cellSize)}`, { route, point });
    }
    const nearby = (box, route) => {
      const values = [];
      for (let x = Math.floor((box.x - 5) / cellSize); x <= Math.floor((box.x + box.width + 5) / cellSize); x += 1) {
        for (let y = Math.floor((box.y - 5) / cellSize); y <= Math.floor((box.y + box.height + 5) / cellSize); y += 1) {
          for (const sample of sampleCells.get(`${x}:${y}`) || []) if (sample.route !== route) values.push(sample.point);
        }
      }
      return values;
    };
    const fixedObstacles = [...headerObstacles,
      ...portLabelObstacles.map(obstacle => expandBox(obstacle, PORT_LABEL_CLEARANCE))];
    const placed = [];
    const ordered = [...routes.entries()].sort(([left], [right]) => left - right);
    for (const [routeIndex, route] of ordered) {
      const badge = {
        // Four digits fit before admission; queue growth cannot move a label.
        width: Math.max(EDGE_COUNTER_WIDTH, textWidth("≡ 0000") + 8),
        height: 18,
      };
      const samples = routeSamples.get(routeIndex) || [route.start, route.end];
      // A mailbox belongs to the receiving input, not the producer.
      const preferred = 0.82;
      const fractions = Array.from({ length: 61 }, (_, index) => 0.05 + index * 0.015)
        .sort((left, right) => Math.abs(left - preferred) - Math.abs(right - preferred) || left - right);
      const candidates = fractions.map((fraction, order) => ({
          center: pointAtPolylineFraction(samples, fraction),
          order,
        }));
      // In a compact channel a centered badge can cover a neighboring track.
      // It may touch either side of its own vertical run instead, without a
      // leader or a detached counter. Centered candidates win equal scores.
      const vertical = curvedCorridorSegments(route.presentationPoints)
        .filter(segment => segment.line && segment.from.x === segment.to.x);
      for (const { center } of [...candidates]) {
        if (!vertical.some(segment => Math.abs(center.x - segment.from.x) < 0.01
          && center.y >= Math.min(segment.from.y, segment.to.y)
          && center.y <= Math.max(segment.from.y, segment.to.y))) continue;
        for (const direction of [-1, 1]) candidates.push({
          center: { x: center.x + direction * badge.width / 2, y: center.y },
          order: candidates.length,
        });
      }
      const evaluated = candidates.map((candidate, order) => {
        const box = centeredBox(candidate.center, badge.width, badge.height);
        return {
          ...candidate,
          box,
          order,
          score: curveLabelCollisionScore(
            box,
            layout,
            nodeObstacles,
            fixedObstacles,
            nearby(box, routeIndex),
            placed,
          ),
        };
      });
      evaluated.sort((left, right) => left.score - right.score || left.order - right.order);
      const fallbackAnchor = pointAtPolylineFraction(samples, 0.5);
      const selected = evaluated[0] || {
        center: fallbackAnchor,
        box: centeredBox(fallbackAnchor, badge.width, badge.height),
      };
      placed.push(selected.box);
      route.label = {
        x: selected.center.x,
        y: selected.center.y,
        leader: null,
        box: selected.box,
      };
    }
  }

  function growLayoutForRoutes(routes, layout) {
    let maximumX = layout.width;
    let maximumY = layout.height;
    for (const route of routes.values()) {
      for (const point of route.presentationPoints) {
        maximumX = Math.max(maximumX, point.x + 28);
        maximumY = Math.max(maximumY, point.y + 28);
      }
      maximumX = Math.max(maximumX, route.label.x + 34);
      maximumY = Math.max(maximumY, route.label.y + 18);
    }
    layout.width = Math.ceil(maximumX);
    layout.height = Math.ceil(maximumY);
  }

  function resolveScopePort(owner, direction, edge) {
    const ports = (owner.ports || []).filter((port) => port.direction === direction);
    if (!ports.length) return null;
    const names = direction === "input"
      ? [edge.target.port, edge.source.port]
      : [edge.source.port, edge.target.port];
    for (const name of names) {
      if (!name) continue;
      const matches = ports.filter((port) => port.name === name);
      if (matches.length === 1) return matches[0];
    }
    return ports.length === 1 ? ports[0] : null;
  }

  function ownerDescendants(owners, children) {
    const result = new Map();
    function visit(ownerId) {
      if (result.has(ownerId)) return result.get(ownerId);
      const descendants = new Set([ownerId]);
      for (const child of children.get(ownerId) || []) {
        for (const nested of visit(child)) descendants.add(nested);
      }
      result.set(ownerId, descendants);
      return descendants;
    }
    for (const ownerId of owners.keys()) visit(ownerId);
    return result;
  }

  function ownerChain(ownerId, owners) {
    const chain = [];
    const seen = new Set();
    let current = ownerId;
    while (current && owners.has(current) && !seen.has(current)) {
      seen.add(current);
      chain.push(current);
      current = owners.get(current).parent;
    }
    chain.reverse();
    return chain;
  }

  function scopePortAnchor(owner, position, port, layout) {
    const exact = layout?.scopePortAnchors?.get(scopePortKey(owner.id, port.direction, port.name));
    if (exact) return exact;
    const peers = (owner.ports || []).filter((candidate) => candidate.direction === port.direction);
    const index = Math.max(0, peers.findIndex((candidate) => candidate.name === port.name));
    return {
      x: port.direction === "input" ? position.x : position.x + position.width,
      y: centeredSideY(position, index, peers.length),
    };
  }

  function centeredSideY(position, index, count) {
    const center = position.y + SCOPE_HEADER + (position.height - SCOPE_HEADER) / 2;
    return center + (index - (count - 1) / 2) * PORT_SPACING;
  }

  function portAnchor(node, position, portName, expectedDirection, layout) {
    if (node.kind === "boundary" || node.kind === "discard") {
      const direction = node.direction || expectedDirection;
      return {
        x: direction === "input" ? position.x + position.width - 10 : position.x + 10,
        y: position.y + position.height / 2,
      };
    }
    const exact = layout?.portAnchors?.get(portKey(node.id, expectedDirection, portName));
    if (exact) return exact;
    const candidates = (node.ports || []).filter((port) => port.direction === expectedDirection);
    const index = Math.max(0, candidates.findIndex((port) => port.name === portName));
    const count = Math.max(1, candidates.length);
    return {
      x: expectedDirection === "input" ? position.x : position.x + position.width,
      y: position.y + position.height * (index + 1) / (count + 1),
    };
  }

  function portKey(node, direction, port) {
    return `${node}\u0000${direction}\u0000${port || ""}`;
  }

  function scopePortKey(owner, direction, port) {
    return `${owner}\u0000${direction}\u0000${port || ""}`;
  }

  function addSample(samples, key, value) {
    if (!samples.has(key)) samples.set(key, []);
    samples.get(key).push(value);
  }

  function median(values) {
    if (!values?.length) return null;
    const ordered = [...values].sort((left, right) => left - right);
    const middle = Math.floor(ordered.length / 2);
    return ordered.length % 2 === 1
      ? ordered[middle]
      : (ordered[middle - 1] + ordered[middle]) / 2;
  }

  function medianOr(values, fallback) {
    return median(values) ?? fallback;
  }

  function boxCenterY(box) {
    return box.y + box.height / 2;
  }

  function clamp(value, minimum, maximum) {
    return Math.max(minimum, Math.min(value, maximum));
  }

  function appendPoints(target, points) {
    for (const point of points) {
      const previous = target.at(-1);
      if (!previous || previous.x !== point.x || previous.y !== point.y) target.push(point);
    }
  }

  function compactOrthogonalPoints(points) {
    const compact = [];
    for (const point of points) {
      const previous = compact.at(-1);
      if (previous && previous.x === point.x && previous.y === point.y) continue;
      compact.push(point);
      while (compact.length >= 3) {
        const [before, corner, after] = compact.slice(-3);
        const collinear = (before.x === corner.x && corner.x === after.x)
          || (before.y === corner.y && corner.y === after.y);
        if (!collinear) break;
        compact.splice(compact.length - 2, 1);
      }
    }
    return compact;
  }

  function textLabelBox(x, baseline, value, anchor) {
    const width = textWidth(value);
    return {
      x: anchor === "end" ? x - width : x,
      y: baseline - 10 / MINIMUM_READABLE_SCALE,
      width,
      height: 13 / MINIMUM_READABLE_SCALE,
    };
  }

  function textWidth(value) {
    return Math.max(10, String(value || "").length * 6 + 4) / MINIMUM_READABLE_SCALE;
  }

  function orthogonalSegments(points) {
    const segments = [];
    for (let index = 1; index < points.length; index += 1) {
      const a = points[index - 1];
      const b = points[index];
      if (a.x === b.x && a.y === b.y) continue;
      segments.push({ a, b });
    }
    return segments;
  }


  function centeredBox(center, width, height) {
    return {
      x: center.x - width / 2,
      y: center.y - height / 2,
      width,
      height,
    };
  }

  function expandBox(box, padding) {
    if (!box) return null;
    return {
      x: box.x - padding,
      y: box.y - padding,
      width: box.width + padding * 2,
      height: box.height + padding * 2,
    };
  }

  function boxesOverlap(left, right) {
    return left.x < right.x + right.width
      && left.x + left.width > right.x
      && left.y < right.y + right.height
      && left.y + left.height > right.y;
  }

  function overlapArea(left, right) {
    if (!boxesOverlap(left, right)) return 0;
    const width = Math.min(left.x + left.width, right.x + right.width) - Math.max(left.x, right.x);
    const height = Math.min(left.y + left.height, right.y + right.height) - Math.max(left.y, right.y);
    return width * height;
  }


  function curveLabelCollisionScore(box, layout, nodes, headers, otherRouteSamples, placed) {
    let score = 0;
    if (box.x < 4 || box.y < 4 || box.x + box.width > layout.width - 4 || box.y + box.height > layout.height - 4) {
      score += 1_000_000;
    }
    for (const obstacle of [...nodes, ...headers]) {
      score += overlapArea(box, obstacle) * 10_000;
    }
    for (const other of placed) {
      score += overlapArea(box, expandBox(other, 6)) * 20_000;
    }
    for (const sample of otherRouteSamples) {
      if (pointInsideBox(sample, expandBox(box, 5))) score += 100_000;
    }
    return score;
  }

  function nearestPointOnBox(point, box) {
    const clampedX = Math.max(box.x, Math.min(point.x, box.x + box.width));
    const clampedY = Math.max(box.y, Math.min(point.y, box.y + box.height));
    if (point.x < box.x || point.x > box.x + box.width) {
      return { x: clampedX, y: clampedY };
    }
    return {
      x: clampedX,
      y: Math.abs(point.y - box.y) < Math.abs(point.y - (box.y + box.height))
        ? box.y
        : box.y + box.height,
    };
  }

  function roundedOrthogonalPath(points, radius) {
    const compact = compactOrthogonalPoints(points);
    if (compact.length < 2) return "";
    let path = `M ${compact[0].x} ${compact[0].y}`;
    for (let index = 1; index < compact.length - 1; index += 1) {
      const previous = compact[index - 1];
      const corner = compact[index];
      const next = compact[index + 1];
      const incoming = Math.hypot(corner.x - previous.x, corner.y - previous.y);
      const outgoing = Math.hypot(next.x - corner.x, next.y - corner.y);
      const turnRadius = Math.min(radius, incoming / 2, outgoing / 2);
      const before = {
        x: corner.x - Math.sign(corner.x - previous.x) * turnRadius,
        y: corner.y - Math.sign(corner.y - previous.y) * turnRadius,
      };
      const after = {
        x: corner.x + Math.sign(next.x - corner.x) * turnRadius,
        y: corner.y + Math.sign(next.y - corner.y) * turnRadius,
      };
      path += ` L ${before.x} ${before.y} Q ${corner.x} ${corner.y} ${after.x} ${after.y}`;
    }
    const end = compact.at(-1);
    return `${path} L ${end.x} ${end.y}`;
  }

  // Direct links and detours share one presentation: orthogonal tracks with
  // bounded rounded corners. Aligned pins remain a single straight segment.
  function curvedCorridorPath(points, radius = CORNER_RADIUS) {
    return roundedOrthogonalPath(points, radius);
  }

  function curvedCorridorSegments(points, radius = CORNER_RADIUS) {
    const compact = compactOrthogonalPoints(points);
    if (compact.length < 2) return [];
    const segments = [];
    let current = compact[0];
    const lineTo = to => {
      if (current.x === to.x && current.y === to.y) return;
      segments.push({ from: current, c1: current, c2: to, to, line: true });
      current = to;
    };
    for (let index = 1; index < compact.length; index += 1) {
      const previous = compact[index - 1], corner = compact[index], next = compact[index + 1];
      if (next) {
        const turn = Math.min(radius, Math.hypot(corner.x - previous.x, corner.y - previous.y) / 2,
          Math.hypot(next.x - corner.x, next.y - corner.y) / 2);
        const before = { x: corner.x - Math.sign(corner.x - previous.x) * turn,
          y: corner.y - Math.sign(corner.y - previous.y) * turn };
        const after = { x: corner.x + Math.sign(next.x - corner.x) * turn,
          y: corner.y + Math.sign(next.y - corner.y) * turn };
        lineTo(before);
        // Quadratic corner represented as an equivalent cubic for sampling.
        segments.push({ from: before,
          c1: { x: before.x + (corner.x - before.x) * 2 / 3, y: before.y + (corner.y - before.y) * 2 / 3 },
          c2: { x: after.x + (corner.x - after.x) * 2 / 3, y: after.y + (corner.y - after.y) * 2 / 3 }, to: after });
        current = after;
      } else lineTo(corner);
    }
    return segments;
  }

  function cubicPoint(segment, amount) {
    const inverse = 1 - amount;
    return {
      x: inverse ** 3 * segment.from.x
        + 3 * inverse ** 2 * amount * segment.c1.x
        + 3 * inverse * amount ** 2 * segment.c2.x
        + amount ** 3 * segment.to.x,
      y: inverse ** 3 * segment.from.y
        + 3 * inverse ** 2 * amount * segment.c1.y
        + 3 * inverse * amount ** 2 * segment.c2.y
        + amount ** 3 * segment.to.y,
    };
  }

  function sampleCurvedCorridor(points, radius = CORNER_RADIUS, samplesPerSegment = 8, maximumStep = Infinity) {
    const segments = curvedCorridorSegments(points, radius);
    if (!segments.length) return [];
    const samples = [segments[0].from];
    for (const segment of segments) {
      // A cubic's speed is bounded by three times its longest control edge.
      // Label collision checks need a spatial bound even on very long tracks.
      const speedBound = 3 * Math.max(
        Math.hypot(segment.c1.x - segment.from.x, segment.c1.y - segment.from.y),
        Math.hypot(segment.c2.x - segment.c1.x, segment.c2.y - segment.c1.y),
        Math.hypot(segment.to.x - segment.c2.x, segment.to.y - segment.c2.y),
      );
      const count = Math.max(samplesPerSegment, Math.ceil(speedBound / maximumStep));
      for (let index = 1; index <= count; index += 1) {
        samples.push(cubicPoint(segment, index / count));
      }
    }
    return samples;
  }

  function pointAtPolylineFraction(points, fraction) {
    if (!points.length) return { x: 0, y: 0 };
    if (points.length === 1) return points[0];
    const lengths = [];
    let total = 0;
    for (let index = 1; index < points.length; index += 1) {
      const length = Math.hypot(points[index].x - points[index - 1].x, points[index].y - points[index - 1].y);
      lengths.push(length);
      total += length;
    }
    let remaining = total * clamp(fraction, 0, 1);
    for (let index = 0; index < lengths.length; index += 1) {
      if (remaining <= lengths[index] || index === lengths.length - 1) {
        const amount = lengths[index] === 0 ? 0 : remaining / lengths[index];
        return {
          x: points[index].x + (points[index + 1].x - points[index].x) * amount,
          y: points[index].y + (points[index + 1].y - points[index].y) * amount,
        };
      }
      remaining -= lengths[index];
    }
    return points.at(-1);
  }

  function pointInsideBox(point, box) {
    return point.x >= box.x
      && point.x <= box.x + box.width
      && point.y >= box.y
      && point.y <= box.y + box.height;
  }


  function segmentOrientation(segment) {
    return segment.a.y === segment.b.y ? "horizontal" : "vertical";
  }

  function collinearOverlapLength(left, right) {
    if (segmentOrientation(left) !== segmentOrientation(right)) return 0;
    if (segmentOrientation(left) === "horizontal") {
      if (left.a.y !== right.a.y) return 0;
      return Math.max(0,
        Math.min(Math.max(left.a.x, left.b.x), Math.max(right.a.x, right.b.x))
          - Math.max(Math.min(left.a.x, left.b.x), Math.min(right.a.x, right.b.x)));
    }
    if (left.a.x !== right.a.x) return 0;
    return Math.max(0,
      Math.min(Math.max(left.a.y, left.b.y), Math.max(right.a.y, right.b.y))
        - Math.max(Math.min(left.a.y, left.b.y), Math.min(right.a.y, right.b.y)));
  }

  function segmentsCross(left, right) {
    if (segmentOrientation(left) === segmentOrientation(right)) return false;
    const horizontal = segmentOrientation(left) === "horizontal" ? left : right;
    const vertical = segmentOrientation(left) === "vertical" ? left : right;
    const minimumX = Math.min(horizontal.a.x, horizontal.b.x);
    const maximumX = Math.max(horizontal.a.x, horizontal.b.x);
    const minimumY = Math.min(vertical.a.y, vertical.b.y);
    const maximumY = Math.max(vertical.a.y, vertical.b.y);
    return vertical.a.x > minimumX
      && vertical.a.x < maximumX
      && horizontal.a.y > minimumY
      && horizontal.a.y < maximumY;
  }

  function countRouteCrossings(routes) {
    const entries = [...routes.entries()];
    let crossings = 0;
    for (let left = 0; left < entries.length; left += 1) {
      const leftSegments = orthogonalSegments(entries[left][1].points);
      for (let right = left + 1; right < entries.length; right += 1) {
        const rightSegments = orthogonalSegments(entries[right][1].points);
        for (const leftSegment of leftSegments) {
          for (const rightSegment of rightSegments) {
            if (segmentsCross(leftSegment, rightSegment)) crossings += 1;
          }
        }
      }
    }
    return crossings;
  }

  function countRouteOverlaps(routes) {
    const entries = [...routes.entries()];
    let overlaps = 0;
    for (let left = 0; left < entries.length; left += 1) {
      const leftSegments = orthogonalSegments(entries[left][1].points);
      for (let right = left + 1; right < entries.length; right += 1) {
        const rightSegments = orthogonalSegments(entries[right][1].points);
        for (const leftSegment of leftSegments) {
          for (const rightSegment of rightSegments) {
            if (collinearOverlapLength(leftSegment, rightSegment) > 1) overlaps += 1;
          }
        }
      }
    }
    return overlaps;
  }

  return Object.freeze({
    minimumReadableScale: MINIMUM_READABLE_SCALE,
    nodeSubtitle,
    materializeGraph,
    layoutGraph,
    routeGraphEdges,
    scopePortAnchor,
    portAnchor,
    roundedOrthogonalPath,
    curvedCorridorPath,
    curvedCorridorSegments,
    sampleCurvedCorridor,
    orthogonalSegments,
    countLayerCrossings,
    countRouteCrossings,
    countRouteOverlaps,
  });
}));
