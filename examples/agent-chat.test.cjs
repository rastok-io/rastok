// Exercise the inline, dependency-free client without a browser or a model.
// The DOM/socket doubles expose user actions and received protocol frames;
// they deliberately do not reach into the client's private state.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const html = fs.readFileSync(path.join(__dirname, "agent-chat.html"), "utf8");
const script = html.match(/<script>([\s\S]*?)<\/script>/)[1];

test("the gateway title and introduction invite more than two conversations", () => {
  assert.equal(html.match(/<title>([^<]+)<\/title>/)[1], "Rastok Codex Gateway");
  assert.equal(html.match(/<h1>([^<]+)<\/h1>/)[1], "Rastok Codex Gateway");
  assert.ok(html.includes("Open as many tabs as you want. Each nickname gets its own Codex process."));
});

function client() {
  class Element {
    constructor() {
      this.children = [];
      this.listeners = new Map();
      this.value = "";
      this.disabled = false;
      this.text = "";
    }
    get textContent() { return this.text + this.children.map(child => child.textContent).join(""); }
    set textContent(text) { this.text = String(text); this.children = []; }
    append(...children) { this.children.push(...children); }
    replaceChildren(...children) { this.text = ""; this.children = children; }
    addEventListener(name, listener) { this.listeners.set(name, listener); }
    focus() { document.activeElement = this; }
    fire(name) { this.listeners.get(name)?.({ preventDefault() {} }); }
  }
  const elements = new Map();
  for (const match of html.matchAll(/<\w+\b[^>]*\bid="([^"]+)"[^>]*>/g)) {
    const element = new Element();
    element.disabled = /\bdisabled\b/.test(match[0]);
    elements.set(match[1], element);
  }
  const document = {
    getElementById: id => elements.get(id),
    createElement: () => new Element(),
    activeElement: null,
  };
  const sockets = [];
  class Socket {
    static OPEN = 1;
    constructor(url) {
      this.url = url;
      this.readyState = 0;
      this.sent = [];
      this.closes = [];
      sockets.push(this);
    }
    send(text) {
      assert.equal(this.readyState, Socket.OPEN, "only an open socket accepts writes");
      this.sent.push(JSON.parse(text));
    }
    open() { this.readyState = Socket.OPEN; this.onopen(); }
    receive(frame) { this.onmessage({ data: JSON.stringify(frame) }); }
    rpc(message) { this.receive({ kind: "rpc", rpc: message }); }
    close(code, reason) { this.closes.push({ code, reason }); this.readyState = 2; }
    finishClose() { this.readyState = 3; this.onclose(); }
  }
  vm.runInNewContext(script, { document, WebSocket: Socket }, { filename: "agent-chat.html" });
  const get = id => elements.get(id);
  function connect(nickname = "alice") {
    get("nickname").value = nickname;
    get("connect-form").fire("submit");
    return sockets.at(-1);
  }
  function ready(nickname = "alice") {
    const socket = connect(nickname);
    socket.open();
    socket.rpc({ id: 0, result: { userAgent: "fixture" } });
    socket.rpc({ id: 1, result: { thread: { id: "thread-1" } } });
    return socket;
  }
  return { get, connect, ready, sockets, document };
}

test("the page is standalone and the handshake waits for each response", () => {
  assert.doesNotMatch(html, /<script\b[^>]*\bsrc=|<link\b[^>]*rel="stylesheet"/);
  const { get, connect } = client();
  assert.equal(get("send").disabled, true);
  assert.equal(get("quit").disabled, true);
  const socket = connect();
  assert.equal(socket.sent.length, 0);
  socket.open();
  assert.deepEqual(socket.sent, [{
    nickname: "alice", kind: "rpc",
    rpc: { id: 0, method: "initialize", params: { clientInfo: { name: "rastok_chat", version: "1" } } },
  }]);
  assert.equal(get("send").disabled, true);
  socket.rpc({ id: 0, result: {} });
  assert.deepEqual(socket.sent.slice(1).map(frame => frame.rpc), [
    { method: "initialized", params: {} },
    { id: 1, method: "thread/start", params: { approvalPolicy: "never", sandbox: "read-only" } },
  ]);
  socket.rpc({ id: 1, result: { thread: { id: "thread-1" } } });
  assert.equal(get("send").disabled, false);
});

test("only turn/completed unlocks Send; streaming output stays literal text", () => {
  const { get, ready } = client();
  const socket = ready();
  get("prompt").value = "Hello";
  get("message-form").fire("submit");
  assert.deepEqual(socket.sent.at(-1).rpc, {
    id: 2, method: "turn/start",
    params: { threadId: "thread-1", input: [{ type: "text", text: "Hello" }] },
  });
  socket.rpc({ id: 2, result: { turn: { id: "turn-1", status: "inProgress" } } });
  assert.equal(get("send").disabled, true);
  for (const delta of ["<script>", "literal text</script>"]) {
    socket.rpc({ method: "item/agentMessage/delta", params: {
      threadId: "thread-1", turnId: "turn-1", itemId: "item-1", delta,
    } });
  }
  socket.rpc({ method: "item/completed", params: {
    threadId: "thread-1", turnId: "turn-1",
    item: { id: "item-1", type: "agentMessage", text: "duplicate final text" },
  } });
  const messages = get("messages").children;
  assert.equal(messages.length, 2);
  assert.equal(messages[1].children[1].textContent, "<script>literal text</script>");
  assert.equal(messages[1].children[1].children.length, 0);
  socket.rpc({ method: "turn/completed", params: {
    threadId: "thread-1", turn: { id: "turn-1", status: "completed" },
  } });
  assert.equal(get("send").disabled, false);
});

test("a completed item also works without deltas and another thread is ignored", () => {
  const { get, ready } = client();
  const socket = ready();
  socket.rpc({ method: "item/completed", params: {
    threadId: "another-thread", turnId: "turn-1",
    item: { id: "item-1", type: "agentMessage", text: "not ours" },
  } });
  assert.equal(get("messages").children.length, 0);
  socket.rpc({ method: "item/completed", params: {
    threadId: "thread-1", turnId: "turn-1",
    item: { id: "item-1", type: "agentMessage", text: "whole answer" },
  } });
  assert.match(get("messages").textContent, /whole answer/);
});

test("two tabs may reuse RPC ids without sharing a connection or nickname", () => {
  const alice = client();
  const bob = client();
  const aliceSocket = alice.ready("alice");
  const bobSocket = bob.ready("bob");
  assert.equal(aliceSocket.sent[0].rpc.id, bobSocket.sent[0].rpc.id);
  assert.equal(aliceSocket.sent[0].nickname, "alice");
  assert.equal(bobSocket.sent[0].nickname, "bob");
  alice.get("quit").fire("click");
  assert.deepEqual(aliceSocket.sent.at(-1), { nickname: "alice", kind: "quit" });
  assert.equal(bob.get("send").disabled, false);
  aliceSocket.receive({ kind: "closing" });
  assert.match(alice.get("status").textContent, /closure requested/);
  aliceSocket.finishClose();
  assert.equal(alice.get("connect").disabled, false);
});

test("Quit while connecting cannot initialize a child after cancellation", () => {
  const { get, connect } = client();
  const socket = connect();
  get("quit").fire("click");
  // A late connection callback must not resurrect the cancelled action.
  socket.open();
  assert.deepEqual(socket.sent, []);
});

test("a rejected nickname disconnects without sending Quit to its owner", () => {
  const { get, connect } = client();
  const socket = connect();
  socket.open();
  socket.receive({ kind: "error", message: "Nickname already in use. Choose another one." });
  assert.equal(socket.closes.length, 1);
  assert.equal(socket.sent.length, 1, "no quit command for the rejected name");
  socket.finishClose();
  assert.equal(get("nickname").disabled, false);
  assert.match(get("messages").textContent, /Nickname already in use/);
});

test("Quit during initialization does not send initialized or start a thread", () => {
  const { get, connect } = client();
  const socket = connect();
  socket.open();
  get("quit").fire("click");
  socket.rpc({ id: 0, result: {} });
  assert.equal(socket.sent.length, 2);
  assert.deepEqual(socket.sent.at(-1), { nickname: "alice", kind: "quit" });
  assert.equal(get("send").disabled, true);
});

test("unsupported server requests receive an explicit JSON-RPC refusal", () => {
  const { get, ready } = client();
  const socket = ready();
  socket.rpc({ id: 99, method: "item/commandExecution/requestApproval", params: {} });
  assert.deepEqual(socket.sent.at(-1).rpc, {
    id: 99, error: { code: -32601, message: "Unsupported by this teaching client." },
  });
  assert.equal(get("send").disabled, true);
  assert.match(get("messages").textContent, /unsupported interaction/);
});

test("network and malformed protocol failures remain readable", () => {
  const { get, ready } = client();
  const socket = ready();
  socket.onmessage({ data: "not JSON" });
  assert.match(get("messages").textContent, /Invalid message/);
  socket.onerror();
  assert.match(get("messages").textContent, /WebSocket connection failed/);
  socket.finishClose();
  assert.equal(get("connect").disabled, false);
  assert.equal(get("send").disabled, true);
});
