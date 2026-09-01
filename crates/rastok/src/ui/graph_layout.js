/*
 * Deterministic compound-DAG layout for the embedded Rastok UI.
 *
 * This is an original, dependency-free implementation. It combines standard
 * barycentric layer sweeps with movable side-constrained ports and interval
 * track allocation. Geometry is disposable presentation state: semantic
 * node, port and edge identities always come from the validated snapshot.
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

  const SCOPE_HEADER = 34;
  const SCOPE_PADDING = 20;
  const PORT_SPACING = 20;
  const TRACK_SPACING = 14;
  const PORT_STUB = 10;

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
    const maximumRank = Math.max(0, ...groups.keys());

    const orderByNeighbors = (level, neighbors) => {
      const group = groups.get(level);
      if (!group || group.length < 2) return;
      const order = layerOrder(groups);
      const barycenter = (node) => {
        const adjacent = neighbors.get(node.id) || [];
        if (!adjacent.length) return null;
        const coordinates = adjacent
          .filter((id) => order.has(id))
          .map((id) => order.get(id));
        return coordinates.length
          ? coordinates.reduce((sum, coordinate) => sum + coordinate, 0) / coordinates.length
          : null;
      };
      group.sort((left, right) => {
        const leftCenter = barycenter(left);
        const rightCenter = barycenter(right);
        if (leftCenter !== null && rightCenter !== null && leftCenter !== rightCenter) {
          return leftCenter - rightCenter;
        }
        return byId.get(left.id).index - byId.get(right.id).index;
      });
    };

    // Repeated deterministic sweeps are cheap for the small inspection graph
    // and keep source order as the final tie-break. The local swap pass accepts
    // only a strict reduction in crossings, so equivalent layouts never flap.
    for (let pass = 0; pass < 6; pass += 1) {
      for (let level = 1; level <= maximumRank; level += 1) {
        orderByNeighbors(level, predecessors);
      }
      for (let level = maximumRank - 1; level >= 0; level -= 1) {
        orderByNeighbors(level, outgoing);
      }
    }
    minimizeAdjacentCrossings(groups, edges, rank);

    const rankOrder = new Map();
    for (const [level, group] of groups) {
      group.forEach((node, index) => rankOrder.set(node.id, [level, index]));
    }
    const dimensions = new Map(nodes.map((node) => {
      if (node.kind === "boundary" || node.kind === "discard") {
        return [node.id, { width: 104, height: 34 }];
      }
      const inputs = (node.ports || []).filter((port) => port.direction === "input").length;
      const outputs = (node.ports || []).filter((port) => port.direction === "output").length;
      return [node.id, {
        width: 150,
        height: Math.max(50, (Math.max(inputs, outputs) + 1) * 16),
      }];
    }));

    const columnGap = 142;
    const rankWidths = new Map();
    for (const [level, group] of groups) {
      rankWidths.set(level, Math.max(...group.map((node) => dimensions.get(node.id).width)));
    }
    const rankX = new Map();
    let nextX = 76;
    for (let level = 0; level <= maximumRank; level += 1) {
      rankX.set(level, nextX);
      nextX += (rankWidths.get(level) || 150) + columnGap;
    }

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

    const rowGap = 28;
    const scopeGap = 24;
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
      const ownerPorts = ownerMap.get(ownerId)?.ports || [];
      const portBand = Math.max(
        ownerPorts.filter((port) => port.direction === "input").length,
        ownerPorts.filter((port) => port.direction === "output").length,
      ) * PORT_SPACING;
      const height = SCOPE_HEADER + SCOPE_PADDING * 2
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
      let cursorY = top + SCOPE_HEADER + SCOPE_PADDING;
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
      let left = components.length ? Math.min(...components.map((component) => component.left)) - 30 : 48;
      let right = components.length ? Math.max(...components.map((component) => component.right)) + 30 : left + 220;
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

    const layout = {
      width: Math.max(...[...scopes.values()].map((scope) => scope.x + scope.width), nextX) + 28,
      height: rootScope.y + rootScope.height + 28,
      positions,
      scopes,
      rank,
      owners: ownerMap,
      ownerChildren: children,
      rootOwner: rootOwner.id,
      portAnchors: new Map(),
      scopePortAnchors: new Map(),
    };
    placeMovablePorts(nodes, edges, layout);
    return layout;
  }

  function layerOrder(groups) {
    const order = new Map();
    for (const [level, ranked] of groups) {
      ranked.forEach((node, index) => order.set(node.id, level * 1_000_000 + index));
    }
    return order;
  }

  function minimizeAdjacentCrossings(groups, edges, rank) {
    let best = countLayerCrossings(groups, edges, rank);
    const levels = [...groups.keys()].sort((left, right) => left - right);
    for (let pass = 0; pass < 4; pass += 1) {
      let changed = false;
      for (const level of levels) {
        const group = groups.get(level);
        for (let index = 0; index + 1 < group.length; index += 1) {
          [group[index], group[index + 1]] = [group[index + 1], group[index]];
          const candidate = countLayerCrossings(groups, edges, rank);
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

  function countLayerCrossings(groups, edges, rank) {
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
        source: order.get(edge.source.id),
        target: order.get(edge.target.id),
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
    const nodeSamples = new Map();
    for (const edge of edges) {
      const sourcePosition = layout.positions.get(edge.source.id);
      const targetPosition = layout.positions.get(edge.target.id);
      if (!sourcePosition || !targetPosition) continue;
      addSample(
        nodeSamples,
        portKey(edge.source.id, "output", edge.source.port),
        boxCenterY(targetPosition),
      );
      addSample(
        nodeSamples,
        portKey(edge.target.id, "input", edge.target.port),
        boxCenterY(sourcePosition),
      );
    }
    allocateNodePorts(nodes, layout, nodeSamples);

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
            addSample(
              ownerSamples,
              scopePortKey(ownerId, direction, port.name),
              boxCenterY(targetPosition),
            );
          }
        }
        if (target.kind === "boundary" && (target.owner || layout.rootOwner) === ownerId) {
          const direction = target.direction || "output";
          const port = resolveScopePort(owner, direction, edge);
          const sourcePosition = layout.positions.get(source.id);
          if (port && sourcePosition) {
            addSample(
              ownerSamples,
              scopePortKey(ownerId, direction, port.name),
              boxCenterY(sourcePosition),
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
        addSample(
          ownerSamples,
          scopePortKey(ownerId, direction, port.name),
          median([boxCenterY(sourcePosition), boxCenterY(targetPosition)]),
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
        );
        for (const [port, y] of anchors) {
          layout.scopePortAnchors.set(scopePortKey(ownerId, direction, port.name), {
            x: direction === "input" ? scope.x : scope.x + scope.width,
            y,
          });
        }
      }
    }

    // Root boundary pseudo-nodes are the hit targets for the root Graph's
    // real ports. Align them with the newly selected movable pin positions.
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

    // Discards are presentation sinks rather than Graph ports. They may move
    // beside their producer, but retain stable edge-index ordering.
    const discards = nodes.filter((node) => node.kind === "discard");
    const discardGroups = new Map();
    for (const node of discards) {
      const ownerId = node.owner || layout.rootOwner;
      if (!discardGroups.has(ownerId)) discardGroups.set(ownerId, []);
      discardGroups.get(ownerId).push(node);
    }
    for (const [ownerId, group] of discardGroups) {
      const scope = layout.scopes.get(ownerId) || layout.scopes.get(layout.rootOwner);
      const anchors = allocateSideItems(
        group,
        scope.y + SCOPE_HEADER + 18,
        scope.y + scope.height - 18,
        (node, index) => medianOr(
          nodeSamples.get(portKey(node.id, "input", null)),
          centeredSideY(scope, index, group.length),
        ),
        (node) => node.id,
      );
      for (const [node, y] of anchors) {
        const position = layout.positions.get(node.id);
        position.x = scope.x + scope.width - 10;
        position.y = y - position.height / 2;
      }
    }

    // Boundary movement changes the useful target of adjacent Node ports.
    // Re-derive once; this converges because only side positions, never ranks,
    // are mutable in this phase.
    const refreshedSamples = new Map();
    for (const edge of edges) {
      const sourcePosition = layout.positions.get(edge.source.id);
      const targetPosition = layout.positions.get(edge.target.id);
      if (!sourcePosition || !targetPosition) continue;
      addSample(refreshedSamples, portKey(edge.source.id, "output", edge.source.port), boxCenterY(targetPosition));
      addSample(refreshedSamples, portKey(edge.target.id, "input", edge.target.port), boxCenterY(sourcePosition));
    }
    allocateNodePorts(nodes, layout, refreshedSamples);
  }

  function allocateNodePorts(nodes, layout, samples) {
    layout.portAnchors.clear();
    for (const node of nodes) {
      if (node.kind === "boundary" || node.kind === "discard") continue;
      const position = layout.positions.get(node.id);
      if (!position) continue;
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

  function allocateSideItems(items, minimum, maximum, desired, stableKey) {
    if (!items.length) return [];
    const ranked = items.map((item, index) => ({
      item,
      index,
      desired: clamp(desired(item, index), minimum, maximum),
      key: stableKey(item),
    }));
    ranked.sort((left, right) => left.desired - right.desired
      || String(left.key).localeCompare(String(right.key))
      || left.index - right.index);
    const spacing = Math.min(PORT_SPACING, (maximum - minimum) / Math.max(1, ranked.length - 1));
    const values = ranked.map((entry) => entry.desired);
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
        start: { x: sourcePort.x + 7, y: sourcePort.y },
        end: { x: targetPort.x - 7, y: targetPort.y },
        portals: routingPortals(edge, source, target, layout),
      });
    }
    const sourceFans = fanOffsets(candidates, true);
    const targetFans = fanOffsets(candidates, false);
    const routes = new Map();
    const occupiedVertical = [];
    const occupiedSegments = [];
    const ordered = [...candidates].sort((left, right) =>
      left.portals.length - right.portals.length
      || left.start.x - right.start.x
      || left.end.x - right.end.x
      || left.edge.index - right.edge.index);

    for (const candidate of ordered) {
      const sourceFan = sourceFans.get(candidate.edge.index) || { offset: 0, branch: 0, count: 1 };
      const targetFan = targetFans.get(candidate.edge.index) || { offset: 0, branch: 0, count: 1 };
      const points = [candidate.start];
      let current = candidate.start;
      if (sourceFan.count > 1) {
        const separated = { x: candidate.start.x, y: candidate.start.y + sourceFan.offset };
        const forward = { x: candidate.start.x + PORT_STUB, y: separated.y };
        appendPoints(points, [separated, forward]);
        current = forward;
      }

      for (const portal of candidate.portals) {
        const before = { x: portal.x - PORT_STUB, y: portal.y };
        appendPoints(points, routeSpan(
          current,
          before,
          candidate.edge.index,
          occupiedVertical,
          [...occupiedSegments, ...orthogonalSegments(points)],
        ));
        appendPoints(points, [portal, { x: portal.x + PORT_STUB, y: portal.y }]);
        current = points.at(-1);
      }

      let targetApproach = candidate.end;
      const targetTail = [];
      if (targetFan.count > 1) {
        const separated = {
          x: candidate.end.x - PORT_STUB,
          y: candidate.end.y + targetFan.offset,
        };
        const branch = { x: candidate.end.x, y: separated.y };
        targetApproach = separated;
        targetTail.push(branch, candidate.end);
      }
      appendPoints(points, routeSpan(
        current,
        targetApproach,
        candidate.edge.index,
        occupiedVertical,
        [...occupiedSegments, ...orthogonalSegments(points)],
      ));
      appendPoints(points, targetTail);
      const compact = compactOrthogonalPoints(points);
      const route = {
        start: candidate.start,
        end: candidate.end,
        points: compact,
        portals: candidate.portals,
      };
      routes.set(candidate.edge.index, route);
      occupiedSegments.push(...orthogonalSegments(compact).map((segment) => ({
        ...segment,
        edge: candidate.edge.index,
      })));
    }
    placeEdgeLabels(routes, graph, layout);
    growLayoutForRoutes(routes, layout);
    return routes;
  }

  function fanOffsets(candidates, source) {
    const groups = new Map();
    for (const candidate of candidates) {
      const endpoint = source ? candidate.edge.source : candidate.edge.target;
      const key = `${endpoint.id}\u0000${endpoint.port || ""}`;
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key).push(candidate);
    }
    const offsets = new Map();
    for (const group of groups.values()) {
      group.sort((left, right) => {
        const leftCoordinate = source ? left.end.y : left.start.y;
        const rightCoordinate = source ? right.end.y : right.start.y;
        return leftCoordinate - rightCoordinate || left.edge.index - right.edge.index;
      });
      group.forEach((candidate, index) => offsets.set(candidate.edge.index, {
        offset: (index - (group.length - 1) / 2) * 8,
        count: group.length,
      }));
    }
    return offsets;
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

  function routeSpan(from, to, edge, occupiedVertical, occupiedSegments) {
    const forward = to.x - from.x >= 2 * PORT_STUB + 8;
    const paths = [];
    let order = 0;
    if (from.x === to.x || from.y === to.y) {
      paths.push({ points: [from, to], order: order++ });
    }
    paths.push(
      { points: [from, { x: to.x, y: from.y }, to], order: order++ },
      { points: [from, { x: from.x, y: to.y }, to], order: order++ },
    );

    const verticalLanes = [];
    if (forward) {
      const minimum = from.x + PORT_STUB + 4;
      const maximum = to.x - PORT_STUB - 4;
      const middle = (minimum + maximum) / 2;
      verticalLanes.push(middle);
      for (let index = 1; index <= 12; index += 1) {
        verticalLanes.push(middle - index * TRACK_SPACING, middle + index * TRACK_SPACING);
      }
      verticalLanes.push(minimum, maximum);
    } else {
      const base = Math.max(from.x, to.x) + 30;
      for (let index = 0; index <= 12; index += 1) {
        verticalLanes.push(base + index * TRACK_SPACING);
      }
    }
    for (const lane of verticalLanes) {
      if (forward && (lane < from.x + PORT_STUB || lane > to.x - PORT_STUB)) continue;
      paths.push({
        points: [from, { x: lane, y: from.y }, { x: lane, y: to.y }, to],
        order: order++,
      });
    }

    // A vertical-lane route still has horizontal entry and exit runs. When
    // either would overlap an existing pipe, a horizontal corridor creates
    // two short doglegs and gives the long segment its own Y track.
    const occupiedY = occupiedSegments.flatMap((segment) => [segment.a.y, segment.b.y]);
    const minimumY = Math.min(from.y, to.y, ...occupiedY);
    const maximumY = Math.max(from.y, to.y, ...occupiedY);
    const middleY = (from.y + to.y) / 2;
    const horizontalLanes = [minimumY - 24, maximumY + 24, middleY];
    for (let index = 1; index <= 12; index += 1) {
      horizontalLanes.push(
        middleY - index * TRACK_SPACING,
        middleY + index * TRACK_SPACING,
        minimumY - 24 - index * TRACK_SPACING,
        maximumY + 24 + index * TRACK_SPACING,
      );
    }
    const direction = Math.sign(to.x - from.x) || 1;
    const gap = Math.abs(to.x - from.x);
    const corridorStubs = [0, 4, 7, 10, 14, 21, 28, 35];
    for (const sourceStub of corridorStubs) {
      for (const targetStub of corridorStubs) {
        if (to.x > from.x && sourceStub + targetStub >= gap - 2) continue;
        const fromStub = from.x + direction * sourceStub;
        const toStub = to.x - direction * targetStub;
        for (const lane of horizontalLanes) {
          paths.push({
            points: [
              from,
              { x: fromStub, y: from.y },
              { x: fromStub, y: lane },
              { x: toStub, y: lane },
              { x: toStub, y: to.y },
              to,
            ],
            order: order++,
          });
        }
      }
    }

    const evaluated = paths.map((candidate) => {
      const points = compactOrthogonalPoints(candidate.points);
      const segments = orthogonalSegments(points);
      let score = segments.reduce((sum, segment) => sum + segmentLength(segment), 0)
        + Math.max(0, segments.length - 1) * 6;
      for (const segment of segments) {
        if (segmentOrientation(segment) === "vertical") {
          const segmentMinimum = Math.min(segment.a.y, segment.b.y);
          const segmentMaximum = Math.max(segment.a.y, segment.b.y);
          for (const reservation of occupiedVertical) {
            if (Math.abs(reservation.x - segment.a.x) < 7
              && intervalsOverlap(
                segmentMinimum,
                segmentMaximum,
                reservation.minimum,
                reservation.maximum,
                7,
              )) {
              score += 1_000_000;
            }
          }
        }
        for (const occupied of occupiedSegments) {
          if (collinearOverlapLength(segment, occupied) > 1) score += 1_000_000_000_000;
          if (segmentsCross(segment, occupied)) score += 1_000_000_000;
        }
      }
      return { ...candidate, points, score };
    });
    evaluated.sort((left, right) => left.score - right.score || left.order - right.order);
    const selected = evaluated[0] || { points: [from, to] };
    for (const segment of orthogonalSegments(selected.points)) {
      if (segmentOrientation(segment) !== "vertical") continue;
      occupiedVertical.push({
        x: segment.a.x,
        minimum: Math.min(segment.a.y, segment.b.y),
        maximum: Math.max(segment.a.y, segment.b.y),
        edge,
      });
    }
    return selected.points.slice(1);
  }

  function placeEdgeLabels(routes, graph, layout) {
    const badge = { width: 50, height: 22 };
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
    const portLabelObstacles = [];
    for (const [ownerId, owner] of layout.owners) {
      const scope = layout.scopes.get(ownerId);
      if (!scope) continue;
      for (const port of owner.ports || []) {
        const anchor = scopePortAnchor(owner, scope, port, layout);
        portLabelObstacles.push(textLabelBox(
          anchor.x + (port.direction === "input" ? 9 : -9),
          anchor.y - 8,
          port.name,
          port.direction === "input" ? "start" : "end",
        ));
      }
    }
    for (const node of graph.nodes) {
      if (node.kind === "boundary" || node.kind === "discard") continue;
      const position = layout.positions.get(node.id);
      if (!position) continue;
      for (const port of node.ports || []) {
        const anchor = portAnchor(node, position, port.name, port.direction, layout);
        portLabelObstacles.push(textLabelBox(
          anchor.x + (port.direction === "input" ? -8 : 8),
          anchor.y - 8,
          port.name,
          port.direction === "input" ? "end" : "start",
        ));
      }
    }
    const pathSegments = [...routes.values()].flatMap((route) => orthogonalSegments(route.points));
    const placed = [];
    const ordered = [...routes.entries()].sort(([left], [right]) => left - right);
    for (const [, route] of ordered) {
      const ownSegments = orthogonalSegments(route.points)
        .sort((left, right) => segmentLength(right) - segmentLength(left));
      const candidates = [];
      for (const segment of ownSegments) {
        const anchor = segmentMidpoint(segment);
        if (segment.a.y === segment.b.y) {
          for (const offset of [-22, 22, -38, 38, -54, 54]) {
            candidates.push({ center: { x: anchor.x, y: anchor.y + offset }, anchor });
          }
        } else {
          for (const offset of [34, -34, 50, -50, 66, -66]) {
            candidates.push({ center: { x: anchor.x + offset, y: anchor.y }, anchor });
          }
        }
      }
      const evaluated = candidates.map((candidate, order) => {
        const box = centeredBox(candidate.center, badge.width, badge.height);
        return {
          ...candidate,
          box,
          order,
          score: labelCollisionScore(
            box,
            layout,
            nodeObstacles,
            [...headerObstacles, ...portLabelObstacles],
            pathSegments,
            placed,
          ),
        };
      });
      evaluated.sort((left, right) => left.score - right.score || left.order - right.order);
      const fallbackAnchor = ownSegments[0] ? segmentMidpoint(ownSegments[0]) : route.start;
      const selected = evaluated[0] || {
        center: fallbackAnchor,
        anchor: fallbackAnchor,
        box: centeredBox(fallbackAnchor, badge.width, badge.height),
      };
      placed.push(selected.box);
      route.label = {
        x: selected.center.x,
        y: selected.center.y,
        leader: {
          from: selected.anchor,
          to: nearestPointOnBox(selected.anchor, selected.box),
        },
      };
    }
  }

  function growLayoutForRoutes(routes, layout) {
    let maximumX = layout.width;
    let maximumY = layout.height;
    for (const route of routes.values()) {
      for (const point of route.points) {
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
    const width = Math.max(10, String(value || "").length * 6 + 4);
    return {
      x: anchor === "end" ? x - width : x,
      y: baseline - 10,
      width,
      height: 13,
    };
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

  function segmentLength(segment) {
    return Math.abs(segment.b.x - segment.a.x) + Math.abs(segment.b.y - segment.a.y);
  }

  function segmentMidpoint(segment) {
    return {
      x: (segment.a.x + segment.b.x) / 2,
      y: (segment.a.y + segment.b.y) / 2,
    };
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

  function segmentCrossesBox(segment, box, padding) {
    const expanded = expandBox(box, padding);
    const minimumX = Math.min(segment.a.x, segment.b.x);
    const maximumX = Math.max(segment.a.x, segment.b.x);
    const minimumY = Math.min(segment.a.y, segment.b.y);
    const maximumY = Math.max(segment.a.y, segment.b.y);
    if (segment.a.y === segment.b.y) {
      return segment.a.y >= expanded.y
        && segment.a.y <= expanded.y + expanded.height
        && maximumX >= expanded.x
        && minimumX <= expanded.x + expanded.width;
    }
    return segment.a.x >= expanded.x
      && segment.a.x <= expanded.x + expanded.width
      && maximumY >= expanded.y
      && minimumY <= expanded.y + expanded.height;
  }

  function labelCollisionScore(box, layout, nodes, headers, segments, placed) {
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
    for (const segment of segments) {
      if (segmentCrossesBox(segment, box, 5)) score += 100_000;
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

  function intervalsOverlap(leftMinimum, leftMaximum, rightMinimum, rightMaximum, clearance = 0) {
    return leftMinimum < rightMaximum + clearance && leftMaximum > rightMinimum - clearance;
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
    materializeGraph,
    layoutGraph,
    routeGraphEdges,
    scopePortAnchor,
    portAnchor,
    roundedOrthogonalPath,
    orthogonalSegments,
    countLayerCrossings,
    countRouteCrossings,
    countRouteOverlaps,
  });
}));
