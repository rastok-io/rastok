/*
 * Deterministic compound-graph layout for the embedded Rastok UI.
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

  const SCOPE_HEADER = 26;
  const SCOPE_PADDING = 12;
  const PORT_SPACING = 20;
  const TRACK_SPACING = 12;
  const PORT_STUB = 8;
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

    // Child scopes already have their own vertical bands. Global DAG ranks
    // order work, but are not horizontal space that must be paid for again in
    // every band (a downstream child is not another screen to the right).
    const ownerRankX = new Map();
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
        const outputLabels = Math.max(0, ...ranked.flatMap(node => (node.ports || [])
          .filter(port => port.direction === "output")
          .map(port => textWidth(port.display_label || port.name))));
        x += width + Math.max(columnGap, outputLabels + 56);
      }
      ownerRankX.set(ownerId, coordinates);
    }

    const rowGap = 20;
    const scopeGap = 16;
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
        const outputLabel = Math.max(0, ...(node.ports || [])
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
      const leftPadding = hasInput
        ? ownerId === rootOwner.id ? ROOT_CONTENT_GUTTER : 64
        : SCOPE_PADDING * 2;
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
      const rightPadding = ownerId === rootOwner.id
        ? Math.max(ROOT_OUTPUT_GUTTER, resultLaneWidth)
        : 64;
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
      owners: ownerMap,
      ownerChildren: children,
      rootOwner: rootOwner.id,
      portAnchors: new Map(),
      scopePortAnchors: new Map(),
    };
    placeMovablePorts(nodes, edges, layout);
    // Once public boundary endpoints have moved, align child scope pins with
    // the same final producer/consumer ports instead of their initial boxes.
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
              layout.portAnchors.get(portKey(target.id, "input", edge.target.port))?.y ?? boxCenterY(targetPosition),
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
              layout.portAnchors.get(portKey(source.id, "output", edge.source.port))?.y ?? boxCenterY(sourcePosition),
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
        start: { x: sourcePort.x, y: sourcePort.y },
        end: { x: targetPort.x, y: targetPort.y },
        portals: routingPortals(edge, source, target, layout),
      });
    }
    const sourceFans = fanOffsets(candidates, true);
    const targetFans = fanOffsets(candidates, false);
    const routes = new Map();
    const occupiedVertical = [];
    const occupiedSegments = [];
    const labelObstacles = graphLabelObstacles(graph, layout);
    const bodies = graph.nodes.filter(node => node.kind !== "boundary" && node.kind !== "discard")
      .map(node => layout.positions.get(node.id));
    const headers = [...layout.scopes.values()].map(scope => ({ ...scope, height: SCOPE_HEADER }));
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
        const forward = { x: candidate.start.x + FAN_STUB, y: candidate.start.y };
        const separated = { x: forward.x, y: candidate.start.y + sourceFan.offset };
        const branch = { x: forward.x + PORT_STUB, y: separated.y };
        appendPoints(points, [forward, separated, branch]);
        current = branch;
      } else {
        current = { x: candidate.start.x + PORT_STUB, y: candidate.start.y };
        appendPoints(points, [current]);
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

      let targetApproach = { x: candidate.end.x - PORT_STUB, y: candidate.end.y };
      const targetTail = [candidate.end];
      if (targetFan.count > 1) {
        const branch = {
          x: candidate.end.x - FAN_STUB - PORT_STUB,
          y: candidate.end.y + targetFan.offset,
        };
        const separated = {
          x: candidate.end.x - FAN_STUB,
          y: candidate.end.y + targetFan.offset,
        };
        const aligned = { x: separated.x, y: candidate.end.y };
        targetApproach = branch;
        targetTail.splice(0, 1, separated, aligned, candidate.end);
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
      const presentationPoints = [candidate.start,
        ...candidate.portals.map(portal => ({ x: portal.x, y: portal.y }))];
      // Multiple pipes keep their identities and join the same short approach
      // to the receiving pin. They do not create a shared semantic mailbox.
      if (targetFan.count > 1) presentationPoints.push({ x: candidate.end.x - FAN_STUB * 2, y: candidate.end.y });
      presentationPoints.push(candidate.end);
      const ownPorts = new Set([
        portKey(candidate.source.id, "output", candidate.edge.source.port),
        portKey(candidate.target.id, "input", candidate.edge.target.port),
        ...candidate.portals.map(portal => scopePortKey(portal.owner, portal.direction, portal.port)),
      ]);
      for (const endpoint of [candidate.source, candidate.target]) {
        if (endpoint.kind === "boundary") ownPorts.add(scopePortKey(endpoint.owner, endpoint.direction, endpoint.ports[0].name));
      }
      const obstacles = [...labelObstacles.filter(box => !ownPorts.has(box.portKey)), ...bodies, ...headers];
      const clearPoints = [];
      for (let index = 1; index < presentationPoints.length; index += 1) {
        appendPoints(clearPoints, routePresentationSpan(presentationPoints[index - 1], presentationPoints[index],
          obstacles, layout.scopes.get(layout.rootOwner)));
      }
      const route = {
        start: candidate.start,
        end: candidate.end,
        points: compact,
        presentationPoints: clearPoints,
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
        offset: (index - (group.length - 1) / 2) * 6,
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
      if (!position || node.kind === "boundary") continue;
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

  function routePresentationSpan(from, to, obstacles, bounds) {
    const score = points => {
      const samples = sampleCurvedCorridor(points, CORNER_RADIUS, 32);
      let collisions = 0;
      let length = 0;
      for (let index = 1; index < samples.length; index += 1) {
        const point = samples[index];
        length += Math.hypot(point.x - samples[index - 1].x, point.y - samples[index - 1].y);
        if (point.x < bounds.x || point.x > bounds.x + bounds.width
          || point.y < bounds.y + SCOPE_HEADER || point.y > bounds.y + bounds.height) collisions += 1;
        for (const box of obstacles) {
          if (point.x > box.x + 0.5 && point.x < box.x + box.width - 0.5
            && point.y > box.y + 0.5 && point.y < box.y + box.height - 0.5) collisions += 1;
        }
      }
      return { points, collisions, cost: collisions * 1_000_000 + length + points.length * 8 };
    };
    const gap = to.x - from.x;
    const direct = score([from, { ...to, curve: true }]);
    if (gap > 0 && !direct.collisions) return direct.points;
    const lanes = [...new Set([
      (from.y + to.y) / 2,
      ...obstacles.flatMap(box => [box.y - CORNER_RADIUS, box.y + box.height + CORNER_RADIUS]),
    ])].filter(y => y >= bounds.y + SCOPE_HEADER && y <= bounds.y + bounds.height)
      .sort((a, b) => a - b);
    let best = null;
    for (const lane of lanes) {
      for (const stub of [24, 48, 72]) {
        const departure = gap > 0 ? Math.min(stub, gap / 3) : stub;
        const arrival = departure;
        const points = compactOrthogonalPoints([
          from,
          { x: from.x + departure, y: from.y },
          { x: from.x + departure, y: lane },
          { x: to.x - arrival, y: lane },
          { x: to.x - arrival, y: to.y },
          to,
        ]);
        const candidate = score(points);
        if (!best || candidate.cost < best.cost) best = candidate;
      }
    }
    return best?.points || [from, to];
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
      sampleCurvedCorridor(route.presentationPoints, CORNER_RADIUS, 12),
    ]));
    const placed = [];
    const ordered = [...routes.entries()].sort(([left], [right]) => left - right);
    for (const [routeIndex, route] of ordered) {
      const edge = graph.edges.find(candidate => candidate.index === routeIndex);
      const badge = {
        width: Math.max(EDGE_COUNTER_WIDTH, textWidth(`≡ ${edge?.queued_count || "0"}`) + 8),
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
            [
              ...headerObstacles,
              ...portLabelObstacles.map((obstacle) => expandBox(obstacle, PORT_LABEL_CLEARANCE)),
            ],
            [...routeSamples.entries()]
              .filter(([index]) => index !== routeIndex)
              .flatMap(([, values]) => values),
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
        if (!collinear || corner.curve || after.curve) break;
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

  // A clear forward span is one cubic with horizontal port tangents. Detours
  // retain straight orthogonal tracks; only their corners are rounded.
  function curvedCorridorPath(points, radius = CORNER_RADIUS) {
    if (!points.some(point => point.curve) && points.length > 2) return roundedOrthogonalPath(points, radius);
    const segments = curvedCorridorSegments(points, radius);
    if (!segments.length) return "";
    const start = segments[0].from;
    return segments.reduce(
      (path, segment) => segment.line ? `${path} L ${segment.to.x} ${segment.to.y}`
        : `${path} C ${segment.c1.x} ${segment.c1.y} ${segment.c2.x} ${segment.c2.y} ${segment.to.x} ${segment.to.y}`,
      `M ${start.x} ${start.y}`,
    );
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
      if (corner.curve || compact.length === 2) {
        const reach = (corner.x - current.x) * 0.42;
        segments.push({ from: current, c1: { x: current.x + reach, y: current.y },
          c2: { x: corner.x - reach, y: corner.y }, to: corner });
        current = corner;
      } else if (next && !next.curve) {
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

  function sampleCurvedCorridor(points, radius = CORNER_RADIUS, samplesPerSegment = 8) {
    const segments = curvedCorridorSegments(points, radius);
    if (!segments.length) return [];
    const samples = [segments[0].from];
    for (const segment of segments) {
      for (let index = 1; index <= samplesPerSegment; index += 1) {
        samples.push(cubicPoint(segment, index / samplesPerSegment));
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
