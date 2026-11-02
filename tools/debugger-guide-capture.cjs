"use strict";

// Capture the running debugger, including its own source and assertion views.
// Requires a hosted-ui binary and an already installed Playwright/Chrome.
// RASTOK_UI_ASSETS names the output directory; optional RASTOK_BIN selects
// the binary. No capability URL is written to the manifest or console.
const assert = require("node:assert/strict");
const { spawn, execFileSync } = require("node:child_process");
const { createHash } = require("node:crypto");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const { chromium } = require(process.env.RASTOK_PLAYWRIGHT || "playwright");
const root = path.resolve(__dirname, "..");
const binary = path.resolve(process.env.RASTOK_BIN || path.join(root, "target/debug/rastok"));
const viewport = { width: 1352, height: 760 };
assert.ok(process.env.RASTOK_UI_ASSETS, "RASTOK_UI_ASSETS must name the output directory");
const destination = path.resolve(process.env.RASTOK_UI_ASSETS);
const children = [];
const digest = bytes => createHash("sha256").update(bytes).digest("hex");
const safeMessage = error => String(error.message || error).replace(/([?&]token=)[^\s"'&)]+/g, "$1[REDACTED]");

async function start(args, cwd, env = {}) {
  const child = spawn(binary, args, { cwd, env: { ...process.env, ...env }, stdio: ["ignore", "pipe", "pipe"] });
  children.push(child);
  // Startup output may contain capabilities. Only parse it in memory.
  child.stderr.resume();
  return new Promise((resolve, reject) => {
    let output = "";
    const timer = setTimeout(() => reject(new Error("debugger did not start within 30 seconds")), 30000);
    child.once("error", error => { clearTimeout(timer); reject(error); });
    child.once("exit", code => { clearTimeout(timer); reject(new Error(`debugger exited ${code}`)); });
    child.stdout.on("data", chunk => {
      output += chunk;
      const match = output.match(/ui: (http:\/\/[^\s]+)/);
      if (match) { clearTimeout(timer); resolve(match[1]); }
    });
  });
}

async function settle(page) {
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
}

async function finish(page) {
  await page.locator("#step:enabled").waitFor();
  await page.locator('[data-command="run-to-completion"]').click();
  await page.locator("#execution-state").filter({ hasText: /^completed$/ }).waitFor();
  await settle(page);
}

async function seek(page, transition) {
  await page.locator("#timeline").fill(String(transition));
  await page.locator("#timeline").dispatchEvent("change");
  await page.locator("#timeline-position").filter({ hasText: new RegExp(`transition ${transition}$`) }).waitFor();
  await settle(page);
}

(async () => {
  await fs.mkdir(destination, { recursive: true });
  const scratch = await fs.mkdtemp(path.join(os.tmpdir(), "rastok-debugger-guide-"));
  // Use the shipped starter generator, not a separately authored lookalike.
  execFileSync(binary, ["new", "hello"], { cwd: scratch, stdio: "pipe" });
  const starter = await fs.readFile(path.join(scratch, "hello.rkg"), "utf8");
  assert.ok(starter.includes('app.in = "world"'));
  await fs.writeFile(path.join(scratch, "assertion.rkg"), starter.replace('app.in = "world"', 'app.in = "monde"'));
  const guide = await fs.readFile(path.join(root, "docs/site/examples/04-webhook-to-partner.md"), "utf8");
  const fixture = guide.match(/RASTOK_SHOWCASE_PARTNER_TOKEN=([^\s]+)/)[1];
  const screenshots = {};
  let browser;
  try {
    browser = await chromium.launch({ headless: true, executablePath: process.env.RASTOK_CHROME });
    const errors = [];
    const open = async url => {
      const page = await browser.newPage({ viewport, colorScheme: "dark", reducedMotion: "reduce" });
      page.on("pageerror", error => errors.push(safeMessage(error)));
      await page.goto(url);
      await page.locator("#step:enabled").waitFor();
      return page;
    };
    const capture = async (page, name, source, scenario, extra = {}) => {
      const visible = await page.locator("body").innerText();
      assert.ok(!visible.includes(fixture), "fixture credentials stay redacted");
      assert.doesNotMatch(visible, /alice@example\.test/);
      await settle(page);
      const bytes = await page.screenshot({ path: path.join(destination, `${name}.png`) });
      screenshots[name] = { source, scenario, transition: Number(await page.locator("#timeline").inputValue()), sha256: digest(bytes), ...extra };
    };

    const webhook = await open(await start([
      "simulate", "examples/webhook-to-partner.rkg", "--ui",
      "--deployment", "examples/webhook-to-partner.deployment.json",
    ], root, { RASTOK_SHOWCASE_PARTNER_TOKEN: fixture, RASTOK_SHOWCASE_PARTNER_TOKEN_VERSION: "fixture-v1" }));
    await webhook.locator("#scenario-select").selectOption("webhook\u0000timeout_then_success");
    await webhook.locator("#timeline-position").filter({ hasText: /^LIVE · Session 1 · transition 0$/ }).waitFor();
    await finish(webhook);
    assert.deepEqual(await webhook.evaluate(() => currentSnapshot.scenario.current.failures), []);
    const head = Number(await webhook.locator("#timeline").inputValue());
    let backoff;
    for (let transition = 1; transition < head; transition++) {
      await seek(webhook, transition);
      if (await webhook.evaluate(() => currentSnapshot.retry_operations.some(operation => operation.phase === "AwaitingAdmission" && operation.due_at_ns !== null))) {
        backoff = transition;
        break;
      }
    }
    assert.ok(backoff, "the recorded scenario contains a retry backoff");
    await webhook.locator(".graph-node-name").filter({ hasText: /^partner$/ }).click();
    await webhook.locator("#tab-instances").click();
    const badges = [".brand-block", ".transport", ".timeline-bar", "#source-section", "#inspector", "#graph-section", ".bar-status"];
    await webhook.evaluate(selectors => {
      selectors.forEach((selector, index) => {
        const target = document.querySelector(selector);
        if (!target) throw new Error(`missing guide region ${selector}`);
        const rect = target.getBoundingClientRect();
        const badge = document.createElement("span");
        badge.textContent = String(index + 1);
        badge.dataset.guideBadge = "true";
        Object.assign(badge.style, { position: "fixed", left: `${Math.max(2, rect.left + 2)}px`, top: `${Math.max(2, rect.top + 2)}px`, width: "24px", height: "24px", lineHeight: "24px", textAlign: "center", borderRadius: "50%", color: "#111", background: "#ffe18a", border: "2px solid #111", font: "bold 16px/24px sans-serif", zIndex: "9999", pointerEvents: "none" });
        document.body.append(badge);
      });
    }, badges);
    await capture(webhook, "debugger-overview", "examples/webhook-to-partner.rkg", "webhook.timeout_then_success", { badges: badges.length });
    await webhook.close();

    const normal = await open(await start(["simulate", "hello.rkg", "--ui"], scratch));
    await finish(normal);
    await normal.locator(".graph-node-name").filter({ hasText: /^greet$/ }).click();
    await normal.locator('[data-source-view="0"]').click();
    assert.ok(await normal.locator("#source-code .selected-definition").count());
    await capture(normal, "debugger-selection", "rastok new hello", "smoke.roundtrip", { selected: "greet" });
    const identifier = normal.locator("#source-code [data-source-token]").filter({ hasText: /^greet$/ }).first();
    await identifier.hover();
    await normal.locator("#source-tooltip").waitFor({ state: "visible" });
    await capture(normal, "debugger-hover", "rastok new hello", "smoke.roundtrip", { identifier: "greet" });
    await normal.close();

    const assertion = await open(await start(["simulate", "assertion.rkg", "--ui"], scratch));
    await finish(assertion);
    await assertion.locator("#tab-assertion").click();
    const evidence = await assertion.locator("#panel-assertion").innerText();
    assert.match(evidence, /Hello, monde/);
    assert.match(evidence, /Hello, world/);
    await capture(assertion, "debugger-assertion", "rastok new hello; app.in changed to monde", "smoke.roundtrip");
    assert.deepEqual(errors, []);
    const manifest = {
      revision: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
      binary_sha256: digest(await fs.readFile(binary)),
      viewport, screenshots,
      capture: "RASTOK_UI_ASSETS=docs/site/assets node tools/debugger-guide-capture.cjs",
      evidence: "Running debugger captured with installed Chrome and Playwright; numbered badges inserted into the DOM before the overview capture. No image editing or model call.",
    };
    await fs.writeFile(path.join(destination, "debugger-capture.json"), `${JSON.stringify(manifest, null, 2)}\n`);
    console.log(JSON.stringify({ output: destination, screenshots, scratch }, null, 2));
  } finally {
    await browser?.close();
    for (const child of children) child.kill("SIGTERM");
  }
})().catch(error => { console.error(safeMessage(error)); process.exitCode = 1; });
