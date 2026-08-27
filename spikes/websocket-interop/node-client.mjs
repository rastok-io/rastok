import WebSocket from "ws";

const [url, expected = "hello-from-node"] = process.argv.slice(2);
if (!url) {
  throw new Error("usage: node node-client.mjs ws://host/path [message]");
}

const socket = new WebSocket(url, { perMessageDeflate: false });
const timeout = setTimeout(() => {
  socket.terminate();
  throw new Error("the independent Node peer timed out");
}, 5000);

socket.on("open", () => socket.send(expected));
socket.on("message", (data, binary) => {
  const actual = binary ? Buffer.from(data).toString("utf8") : data.toString();
  if (actual !== expected) {
    throw new Error(`crossed or changed reply: ${JSON.stringify(actual)}`);
  }
  clearTimeout(timeout);
  socket.close(1000, "proved");
});
socket.on("close", (code) => {
  clearTimeout(timeout);
  if (code !== 1000) {
    throw new Error(`unexpected close code ${code}`);
  }
  console.log("NODE_WS_OK");
});
socket.on("error", (error) => {
  clearTimeout(timeout);
  throw error;
});
