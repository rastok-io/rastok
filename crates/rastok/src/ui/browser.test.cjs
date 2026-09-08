"use strict";

// Optional real-browser qualification, not a frontend/runtime dependency.
// Build `cargo build -p rastok --features hosted-ui` first. Point
// RASTOK_PLAYWRIGHT at an installed Playwright and RASTOK_CHROME at Chrome.
const assert = require("node:assert/strict");
const { test, before, after } = require("node:test");
const { spawn } = require("node:child_process");
const path = require("node:path");
const fs = require("node:fs/promises");
const { chromium } = require(process.env.RASTOK_PLAYWRIGHT || "playwright");
const root = path.resolve(__dirname, "../../../..");
let browser;

before(async () => {
  browser = await chromium.launch({ headless: true, executablePath: process.env.RASTOK_CHROME });
});
after(async () => { await browser?.close(); });

async function openTour(t, viewport = { width: 1352, height: 760 }) {
  const child = spawn(path.join(root, "target/debug/rastok"), ["simulate", "examples/keyed-retry.rkg", "--ui"], { cwd: root });
  t.after(() => child.kill("SIGTERM"));
  const url = await new Promise((resolve, reject) => {
    let output = "";
    const timeout = setTimeout(() => reject(new Error("UI startup timed out")), 30000);
    child.on("error", reject);
    child.on("exit", code => reject(new Error(`UI exited: ${code}`)));
    child.stdout.on("data", chunk => {
      output += chunk;
      const found = output.match(/ui: (http:\/\/[^\s]+)/);
      if (found) { clearTimeout(timeout); resolve(found[1]); }
    });
  });
  const context = await browser.newContext({ viewport, colorScheme: "dark" });
  t.after(() => context.close());
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  t.after(() => assert.deepEqual(errors, [], "no browser exceptions"));
  await page.goto(url);
  await page.locator("#step:enabled").waitFor();
  return page;
}

async function finish(page) {
  await page.locator('[data-command="run-to-completion"]').click();
  await page.locator("#execution-state").filter({ hasText: /^completed$/ }).waitFor();
  return page.locator("#timeline").inputValue();
}

async function presentResponse(page, release) {
  const response = page.waitForResponse(response => response.url().includes("/api/snapshot?"));
  release();
  await (await response).finished();
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
}

test("Loop dials follow historical admissions and instance selection without synthetic ports", { timeout: 90000 }, async t => {
  const page = await openTour(t);
  const dial = page.locator('.graph-loop-dial[data-owner=""]');
  const dialText = () => dial.evaluate(element => element.firstChild.textContent.trim());
  assert.equal(await dialText(), "⟳ 0");
  const head = Number(await finish(page));
  assert.equal(await dialText(), "⟳ 0");
  const seen = new Set();
  const historicalDials = new Map();
  let multiple;
  let waiting;
  for (let boundary = 0; boundary <= head; boundary++) {
    await page.locator("#timeline").fill(String(boundary));
    await page.locator("#timeline").dispatchEvent("change");
    await page.locator("#timeline-position").filter({ hasText: new RegExp(`B${boundary}$`) }).waitFor();
    const value = await dialText();
    historicalDials.set(boundary, value);
    seen.add(value.match(/^⟳ (\d+)/)[1]);
    if (value.includes("·2") && !value.startsWith("⟳ 0")) multiple = boundary;
    if (value.includes("⏱")) {
      assert.match(value, /^⟳ 1/, "an armed delay has not admitted application 2");
      waiting = boundary;
    }
    assert.equal(await page.locator('[data-port="Loop.expired"]').count(), 0);
  }
  assert.deepEqual([...seen].sort(), ["0", "1", "2"]);
  assert.notEqual(multiple, undefined);
  assert.notEqual(waiting, undefined);
  await page.locator("#timeline").fill(String(waiting - 1));
  await page.locator("#timeline").dispatchEvent("change");
  await page.locator("#timeline-position").filter({ hasText: new RegExp(`B${waiting - 1}$`) }).waitFor();
  await page.locator("#playback-speed").selectOption("1");
  await page.locator('[data-command="resume"]').click();
  await page.locator("#timeline-position").filter({ hasText: new RegExp(`^REPLAY.*B${waiting}$`) }).waitFor();
  assert.equal(await dialText(), historicalDials.get(waiting), "replay projects the same retained fold as history");
  await page.locator('[data-command="pause"]').click();
  await page.locator("#timeline").fill(String(multiple));
  await page.locator("#timeline").dispatchEvent("change");
  await page.locator("#timeline-position").filter({ hasText: new RegExp(`B${multiple}$`) }).waitFor();
  await page.locator('[data-focus-key="instances:owner:0"]').click();
  await page.locator('#state h3 button').first().click();
  assert.doesNotMatch(await dialText(), /·2/);
  assert.equal(await page.locator('#loop-runs tbody tr').count(), 1);
  await page.locator('#state h3 button[aria-pressed="true"]').click();
  assert.match(await dialText(), /·2/);
  assert.equal(await page.locator('#loop-runs tbody tr').count(), 2);
  assert.equal(await page.locator('text="retry ⇢ Loop.next"').count(), 1);
});

test("a stale historical response cannot replace Return live", { timeout: 60000 }, async t => {
  const page = await openTour(t);
  const head = await finish(page);
  let release;
  let requested;
  const requestStarted = new Promise(resolve => { requested = resolve; });
  await page.route("**/api/snapshot?session=*&boundary=*", async route => {
    if (release) return route.continue();
    const response = await route.fetch();
    requested();
    await new Promise(resolve => { release = resolve; });
    await route.fulfill({ response });
  });
  await page.locator("#back-boundary").click();
  await requestStarted;
  // A second inspection request wins even when the first one returns last.
  await page.locator("#timeline").fill("1");
  await page.locator("#timeline").dispatchEvent("change");
  await page.locator("#timeline-position").filter({ hasText: /B1$/ }).waitFor();
  await page.locator("#return-live").click();
  await presentResponse(page, release);
  assert.equal(await page.locator("#timeline").inputValue(), head);
  assert.match(await page.locator("#timeline-position").innerText(), /^LIVE/);
});

test("Pause cancels a buffered frame already being fetched", { timeout: 60000 }, async t => {
  const page = await openTour(t);
  await finish(page);
  await page.locator("#timeline").fill("0");
  await page.locator("#timeline").dispatchEvent("change");
  await page.locator("#timeline-position").filter({ hasText: /B0$/ }).waitFor();
  let release;
  let requested;
  const requestStarted = new Promise(resolve => { requested = resolve; });
  await page.route("**/api/snapshot?session=*&boundary=*", async route => {
    const response = await route.fetch();
    requested();
    await new Promise(resolve => { release = resolve; });
    await route.fulfill({ response });
  });
  await page.locator('[data-command="resume"]').click();
  await requestStarted;
  await page.locator('[data-command="pause"]').click();
  await presentResponse(page, release);
  assert.equal(await page.locator("#timeline").inputValue(), "0");
  assert.match(await page.locator("#timeline-position").innerText(), /PAUSED/);
});

test("visible buffered Play honors both selected speeds", { timeout: 60000 }, async t => {
  const page = await openTour(t);
  await finish(page);
  assert.equal(await page.evaluate(() => document.visibilityState), "visible");
  const intervalAt = async speed => {
    await page.locator("#timeline").fill("0");
    await page.locator("#timeline").dispatchEvent("change");
    await page.locator("#timeline-position").filter({ hasText: /B0$/ }).waitFor();
    await page.locator("#playback-speed").selectOption(speed);
    const times = [];
    const record = request => {
      if (request.url().includes("/api/snapshot?")) times.push(performance.now());
    };
    page.on("request", record);
    const sixth = page.waitForRequest(request => new URL(request.url()).searchParams.get("boundary") === "6");
    await page.locator('[data-command="resume"]').click();
    await sixth;
    await page.locator('[data-command="pause"]').click();
    page.off("request", record);
    const intervals = times.slice(1).map((time, index) => time - times[index]);
    intervals.sort((left, right) => left - right);
    assert.ok(intervals.length >= 4);
    return intervals[Math.floor(intervals.length / 2)];
  };
  const slow = await intervalAt("1");
  const fast = await intervalAt("4");
  t.diagnostic(`Median intervals in the visible page: ${Math.round(slow)} ms at 1/s, ${Math.round(fast)} ms at 4/s`);
  assert.ok(slow >= 800, `1 event/s interval was ${slow} ms`);
  assert.ok(fast >= 180 && fast < slow * 0.6, `4 events/s interval ${fast} ms, 1 event/s ${slow} ms`);
});

test("keyboard node selection retains focus and does not force Definition", { timeout: 60000 }, async t => {
  const page = await openTour(t);
  const node = page.locator('.graph-node[role="button"]').first();
  await node.focus();
  await page.keyboard.press("Enter");
  assert.equal(await node.getAttribute("aria-pressed"), "true");
  assert.equal(await node.evaluate(element => element === document.activeElement), true);
  assert.equal(await page.locator("#tab-envelope").getAttribute("aria-selected"), "true");
});

test("empty canvas layers clear node selection", { timeout: 60000 }, async t => {
  const page = await openTour(t);
  const node = page.locator('.graph-node[role="button"]').first();
  await node.click();
  assert.equal(await node.getAttribute("aria-pressed"), "true");
  await page.locator(".graph-pan-layer").dispatchEvent("click");
  assert.equal(await node.getAttribute("aria-pressed"), "false");
});

test("history navigation clears replay messages and keeps transport positions stable", { timeout: 60000 }, async t => {
  const page = await openTour(t);
  const reset = page.locator('[data-command="reset-scenario"]');
  const initial = await reset.boundingBox();
  await finish(page);
  await page.locator("#timeline").fill("0");
  await page.locator("#timeline").dispatchEvent("change");
  await page.locator("#timeline-position").filter({ hasText: /B0$/ }).waitFor();
  await page.locator('[data-command="resume"]').click();
  await page.locator('[data-command="pause"]').click();
  assert.match(await page.locator("#command-status").innerText(), /Replay paused/);
  const paused = await reset.boundingBox();
  const visible = await reset.evaluate(element => {
    const button = element.getBoundingClientRect();
    const parent = element.parentElement.getBoundingClientRect();
    return button.right <= parent.right && button.right <= innerWidth;
  });
  assert.equal(visible, true, "Reset is not clipped while replay is paused");
  assert.ok(Math.abs(initial.x - paused.x) < 1, "transport controls do not shift with the state label");
  await page.locator("#timeline").fill("1");
  await page.locator("#timeline").dispatchEvent("change");
  await page.locator("#timeline-position").filter({ hasText: /B1$/ }).waitFor();
  assert.equal(await page.locator("#command-status").isVisible(), false);
  assert.equal(await page.locator("#execution-state").innerText(), "history");
  assert.match(await page.locator("#timeline-head").innerText(), /engine completed/);
});

test("an admission without an observed delivery does not claim no boundary is selected", { timeout: 60000 }, async t => {
  const page = await openTour(t);
  await page.locator("#step").click();
  await page.locator("#timeline-position").filter({ hasText: /B1$/ }).waitFor();
  assert.doesNotMatch(await page.locator("#envelope").innerText(), /No boundary selected/);
});

test("narrow history keeps Return live and session navigation reachable", { timeout: 60000 }, async t => {
  const page = await openTour(t, { width: 700, height: 760 });
  await finish(page);
  await page.locator("#back-boundary").click();
  await page.locator("#timeline-position").filter({ hasText: /^HISTORY/ }).waitFor();
  assert.equal(await page.locator("#return-live").isVisible(), true);
  assert.equal(await page.locator("#timeline-session").isVisible(), true);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
});

test("scenario Definition shows authored DSL and reset selects a fresh B0", { timeout: 60000 }, async t => {
  const page = await openTour(t);
  await page.locator("#scenario-definition").click();
  assert.match(await page.locator("#definition").innerText(), /app.in = \{ tenant = "alice"/);
  assert.match(await page.locator('.source-line[aria-current="step"]').innerText(), /app.in/);
  const head = await finish(page);
  await page.locator('[data-command="reset-scenario"]').click();
  await page.locator("#timeline-position").filter({ hasText: /^LIVE · Session 1 · B0$/ }).waitFor();
  assert.equal(await page.locator("#command-status").isVisible(), false);
  assert.equal(await page.locator("#step").isEnabled(), true);
  await page.locator("#timeline-session").selectOption("0");
  await page.locator("#timeline-position").filter({ hasText: new RegExp(`HISTORY · Main · B${head}$`) }).waitFor();
  assert.equal(await page.locator("#timeline").inputValue(), head, "reset preserved the previous history");
});

test("the complete tour stays inspectable by mouse and keyboard", { timeout: 60000 }, async t => {
  const page = await openTour(t);
  const capture = async name => {
    if (!process.env.RASTOK_UI_CAPTURES) return;
    await fs.mkdir(process.env.RASTOK_UI_CAPTURES, { recursive: true });
    await page.screenshot({ path: path.join(process.env.RASTOK_UI_CAPTURES, `${name}.png`) });
  };
  await capture("B0");
  const boundaries = [0];
  for (let count = 0; count < 200; count += 1) {
    const previous = await page.locator("#timeline-position").innerText();
    await page.locator("#step:enabled").click();
    await page.locator("#timeline-position").filter({ hasNotText: previous }).waitFor();
    const boundary = Number(await page.locator("#timeline").inputValue());
    boundaries.push(boundary);
    if ([1, 8].includes(boundary)) await capture(`B${boundary}`);
    if (await page.locator("#execution-state").innerText() === "completed") break;
  }
  assert.equal(await page.locator("#execution-state").innerText(), "completed");
  assert.deepEqual(boundaries, boundaries.map((_, index) => index), "Step never skips a retained boundary");
  await capture(`B${boundaries.at(-1)}`);
  assert.match(await page.locator("#scenario-status").innerText(), /0 failures/);
  const readSnapshot = async () => (await page.request.get(new URL("/api/snapshot", page.url()).href)).json();
  const stepped = await readSnapshot();
  await page.locator("#tab-history").click();
  assert.ok(await page.locator(".timeline-record").count() >= 10);
  await capture("completed-history");
  const graph = await page.locator(".graph-canvas").boundingBox();
  const scope = await page.locator(".graph-scope.root .graph-scope-body").boundingBox();
  assert.ok(scope.x >= graph.x && scope.x + scope.width <= graph.x + graph.width);
  assert.ok(scope.y >= graph.y && scope.y + scope.height <= graph.y + graph.height);
  await page.getByRole("button", { name: "Zoom in", exact: true }).click();
  assert.equal(await page.locator(".graph-zoom-value").innerText(), "120%");
  await page.getByRole("button", { name: "Fit graph", exact: true }).click();
  const splitter = page.locator("#workspace-splitter");
  await splitter.focus();
  const width = Number(await splitter.getAttribute("aria-valuenow"));
  await page.keyboard.press("ArrowLeft");
  assert.equal(Number(await splitter.getAttribute("aria-valuenow")), width + 24);
  await page.locator("#back-boundary").click();
  await page.locator("#timeline-position").filter({ hasText: /^HISTORY/ }).waitFor();
  await page.locator(".graph-instance-badge").first().click();
  assert.equal(await page.locator("#tab-instances").getAttribute("aria-selected"), "true");
  await page.locator(".edge-counter").first().click();
  assert.equal(await page.locator("#tab-mailbox").getAttribute("aria-selected"), "true");
  await page.setViewportSize({ width: 700, height: 760 });
  await capture("narrow-history");
  assert.equal(await page.locator("#return-live").isVisible(), true);
  await page.emulateMedia({ colorScheme: "light" });
  await page.setViewportSize({ width: 1352, height: 760 });
  await capture("light-history");
  await page.locator("#return-live").click();
  await page.locator('[data-command="reset-scenario"]').click();
  await page.locator("#timeline-position").filter({ hasText: /^LIVE · Session 1 · B0$/ }).waitFor();
  assert.equal(Number(await finish(page)), boundaries.at(-1), "Continue and Step have the same boundary count");
  const continuous = await readSnapshot();
  assert.deepEqual(continuous.history, stepped.history, "the causal records and observed envelopes are identical");
});
