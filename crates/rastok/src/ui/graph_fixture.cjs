"use strict";

// Seven ranks, five nodes each, adjacent links and seven links spanning ranks.
// Deliberately generated: no application source or private graph is embedded.
function syntheticGraph() {
  const ports = [{ name: "in", direction: "input" }, { name: "out", direction: "output" }];
  const nodes = Array.from({ length: 35 }, (_, index) => ({
    id: `node:${index}`, owner: "owner:0", kind: "operator", type: "Operator",
    label: `stage_${String(index).padStart(2, "0")}`, path: `stage_${index}`,
    ports: ports.map(port => ({ ...port })), extensions: [], instances: [],
  }));
  const edges = [];
  const connect = (source, target) => {
    const index = edges.length;
    edges.push({ index, route: `stage_${source}.out>stage_${target}.in`,
      source: { id: `node:${source}`, port: "out", kind: "node" },
      target: { id: `node:${target}`, port: "in", kind: "node" }, queued_count: "0", mailbox: [] });
  };
  for (let rank = 0; rank < 6; rank += 1) {
    for (let lane = 0; lane < 5; lane += 1) {
      connect(rank * 5 + lane, (rank + 1) * 5 + lane);
      connect(rank * 5 + lane, (rank + 1) * 5 + (lane + 1) % 5);
    }
  }
  for (let index = 0; index < 7; index += 1) connect(index, index + 20);
  return { owners: [{ id: "owner:0", parent: null, label: "App", path: "", ports: [], extensions: [], instances: [] }], nodes, edges, commands: [] };
}

module.exports = { syntheticGraph };
