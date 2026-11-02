"use strict";

// Publication assets from the real debugger, not a mockup. Optional tooling
// only: reuse an installed Playwright/Chrome and build rastok with hosted-ui.
// Run from the repository root after `cargo build --features hosted-ui`:
//   RASTOK_UI_ASSETS=<output dir> node tools/debugger-tour-capture.cjs
// Optional: RASTOK_BIN (binary), RASTOK_PLAYWRIGHT (installed Playwright
// package), RASTOK_CHROME (browser executable), RASTOK_FFMPEG (encoder).
// Copy webhook-tour.png and the three stills into docs/site/assets when the
// produced webhook-capture.json is identical to the committed manifest.
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const { chromium } = require(process.env.RASTOK_PLAYWRIGHT || "playwright");
const root = path.resolve(__dirname, "..");
const output = process.env.RASTOK_UI_ASSETS;
assert.ok(output, "RASTOK_UI_ASSETS must name the output directory");
const destination = path.resolve(output);
const rasterSize = { width: 1352, height: 760 };

(async () => {
  await fs.mkdir(destination, { recursive: true });
  const frames = path.join(destination, "frames");
  await fs.mkdir(frames, { recursive: true });
  const guide = await fs.readFile(path.join(root, "docs/site/examples/04-webhook-to-partner.md"), "utf8");
  const fixture = guide.match(/RASTOK_SHOWCASE_PARTNER_TOKEN=([^\s]+)/)[1];
  const child = spawn(process.env.RASTOK_BIN || path.join(root, "target/debug/rastok"), [
    "simulate", "examples/webhook-to-partner.rkg", "--ui",
    "--deployment", "examples/webhook-to-partner.deployment.json",
  ], { cwd: root, env: { ...process.env, RASTOK_SHOWCASE_PARTNER_TOKEN: fixture, RASTOK_SHOWCASE_PARTNER_TOKEN_VERSION: "fixture-v1" } });
  let browser;
  let errors = "";
  child.stderr.on("data", chunk => { errors += chunk; });
  try {
    const url = await new Promise((resolve, reject) => {
      let stdout = "";
      const timeout = setTimeout(() => reject(new Error(`startup timeout: ${errors}`)), 30000);
      child.once("error", reject);
      child.once("exit", code => { clearTimeout(timeout); reject(new Error(`rastok exited ${code}: ${errors}`)); });
      child.stdout.on("data", chunk => {
        stdout += chunk;
        const match = stdout.match(/ui: (http:\/\/[^\s]+)/);
        if (match) { clearTimeout(timeout); resolve(match[1]); }
      });
    });
    browser = await chromium.launch({ headless: true, executablePath: process.env.RASTOK_CHROME });
    const page = await browser.newPage({ viewport: rasterSize, colorScheme: "dark" });
    const consoleErrors = [];
    page.on("pageerror", error => consoleErrors.push(error.message));
    page.on("console", message => { if (message.type() === "error") consoleErrors.push(message.text()); });
    await page.goto(url);
    await page.locator("#step:enabled").waitFor();
    await page.locator("#scenario-select").selectOption("webhook\u0000timeout_then_success");
    await page.locator("#timeline-position").filter({ hasText: /^LIVE · Session 1 · transition 0$/ }).waitFor();
    await page.locator("#step:enabled").waitFor();
    const render = async () => page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    await render();
    await page.screenshot({ path: path.join(destination, "webhook-before.png") });
    await page.locator('[data-command="run-to-completion"]').click();
    await page.locator("#execution-state").filter({ hasText: /^completed$/ }).waitFor();
    assert.deepEqual(await page.evaluate(() => currentSnapshot.scenario.current.failures), []);
    const head = Number(await page.locator("#timeline").inputValue());
    const seek = async transition => {
      await page.locator("#timeline").fill(String(transition));
      await page.locator("#timeline").dispatchEvent("change");
      await page.locator("#timeline-position").filter({ hasText: new RegExp(`transition ${transition}$`) }).waitFor();
      await render();
    };
    // Save a real recorded waiting state; no invented deadline or pasted label.
    let waiting;
    for (let transition = 1; transition < head; transition++) {
      await seek(transition);
      const awaitingRetry = await page.evaluate(() => currentSnapshot.retry_operations.some(operation =>
        operation.phase === "AwaitingAdmission" && operation.due_at_ns !== null));
      if (awaitingRetry) {
        waiting = transition;
        await page.locator(".graph-node-name").filter({ hasText: /^partner$/ }).click();
        await page.locator("#tab-instances").click();
        if (await page.locator("#future").getAttribute("open") === null) {
          await page.locator("#future summary").click();
        }
        assert.match(await page.locator("#future-items").innerText(), /Retry attempt.*due/);
        assert.match(await page.locator("#retry-operations").innerText(), /held by Retry/);
        await page.locator("#retry-operations").scrollIntoViewIfNeeded();
        await render();
        await page.screenshot({ path: path.join(destination, "webhook-backoff.png") });
        break;
      }
    }
    assert.ok(waiting, "the retry scenario must expose its retained operation deadline");
    await seek(head);
    await page.locator(".graph-node-name").filter({ hasText: /^partner$/ }).click();
    await page.locator("#tab-envelope").click();
    await page.locator("#future summary").click();
    await render();
    await page.screenshot({ path: path.join(destination, "webhook-completed.png") });
    // Thirty seconds at four frames/s. A frame is an actual selected journal
    // prefix. Repeated positions hold still; no interpolation invents events.
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.evaluate(() => selectTopologyItem(null));
    await page.locator('[data-source-view="0"]').click();
    const transitions = [];
    for (let index = 0; index < 120; index++) {
      const transition = index < 8 ? 0 : index >= 104 ? head : Math.floor((index - 8) * head / 95);
      transitions.push(transition);
      await seek(transition);
      const visible = await page.locator("body").innerText();
      assert.doesNotMatch(visible, /alice@example\.test/);
      assert.ok(!visible.includes(fixture), "fixture material is absent from the page");
      await page.screenshot({ path: path.join(frames, `${String(index).padStart(3, "0")}.png`) });
    }
    assert.deepEqual(consoleErrors, []);
    const ffmpeg = spawn(process.env.RASTOK_FFMPEG || "ffmpeg", [
      "-v", "error", "-n", "-framerate", "4", "-i", path.join(frames, "%03d.png"),
      "-c:v", "apng", "-pix_fmt", "rgba", "-pred", "mixed", "-compression_level", "9",
      "-plays", "0", "-final_delay", "1/4", "-f", "apng", path.join(destination, "webhook-tour.png"),
    ], { stdio: "inherit" });
    await new Promise((resolve, reject) => {
      ffmpeg.once("error", reject);
      ffmpeg.once("exit", code => code === 0 ? resolve() : reject(new Error(`ffmpeg exited ${code}`)));
    });
    const manifest = { example: "examples/webhook-to-partner.rkg", scenario: "webhook.timeout_then_success", viewport: rasterSize, screenshots: { before: 0, backoff: waiting, completed: head }, animation: { format: "apng", seconds: 30, framesPerSecond: 4, transitions }, evidence: "Real headless Chromium rendering of retained snapshots; no external audit or model call." };
    await fs.writeFile(path.join(destination, "webhook-capture.json"), `${JSON.stringify(manifest, null, 2)}\n`);
    console.log(JSON.stringify({ screenshots: manifest.screenshots, animation: "30 seconds, APNG", output: destination }));
  } finally {
    await browser?.close();
    child.kill("SIGTERM");
    // Retain the captured frames as local qualification evidence.
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
