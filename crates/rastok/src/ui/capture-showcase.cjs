"use strict";

// Publication assets from the real debugger, not a mockup. Optional tooling
// only: reuse an installed Playwright/Chrome and build rastok with hosted-ui.
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const { chromium } = require(process.env.RASTOK_PLAYWRIGHT || "playwright");
const root = path.resolve(__dirname, "../../../..");
const output = process.env.RASTOK_UI_ASSETS;
assert.ok(output, "RASTOK_UI_ASSETS must name the output directory");
const destination = path.resolve(output);
const rasterSize = { width: 1352, height: 760 };

(async () => {
  await fs.mkdir(destination, { recursive: true });
  const frames = await fs.mkdtemp(path.join(os.tmpdir(), "rastok-showcase-frames-"));
  const child = spawn(process.env.RASTOK_BIN || path.join(root, "target/debug/rastok"), [
    "simulate", "examples/webhook-to-partner.rkg", "--ui",
    "--deployment", "examples/webhook-to-partner.deployment.json",
  ], { cwd: root, env: { ...process.env, RASTOK_SHOWCASE_PARTNER_TOKEN: "capture-only-synthetic-token", RASTOK_SHOWCASE_PARTNER_TOKEN_VERSION: "fixture-v1" } });
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
    await page.locator("#select-scenario").click();
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
    // Save a real retained waiting state; no invented timer or pasted label.
    let waiting;
    for (let transition = 1; transition < head; transition++) {
      await seek(transition);
      if ((await page.locator(".graph-svg").textContent()).includes("⏱")) {
        waiting = transition;
        await page.screenshot({ path: path.join(destination, "webhook-backoff.png") });
        break;
      }
    }
    assert.ok(waiting, "the retry scenario must expose an armed Loop delay");
    await seek(head);
    await page.screenshot({ path: path.join(destination, "webhook-completed.png") });
    // Thirty seconds at four frames/s. A frame is an actual selected journal
    // prefix. Repeated positions hold still; no interpolation invents events.
    const transitions = [];
    for (let index = 0; index < 120; index++) {
      const transition = index < 8 ? 0 : index >= 104 ? head : Math.floor((index - 8) * head / 95);
      transitions.push(transition);
      await seek(transition);
      const visible = await page.locator("body").innerText();
      assert.doesNotMatch(visible, /alice@example\.test|capture-only-synthetic-token/);
      await page.screenshot({ path: path.join(frames, `${String(index).padStart(3, "0")}.png`) });
    }
    assert.deepEqual(consoleErrors, []);
    const ffmpeg = spawn(process.env.RASTOK_FFMPEG || "ffmpeg", [
      "-v", "error", "-n", "-framerate", "4", "-i", path.join(frames, "%03d.png"),
      "-filter_complex", "split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer",
      "-loop", "0", path.join(destination, "webhook-tour.gif"),
    ], { stdio: "inherit" });
    await new Promise((resolve, reject) => {
      ffmpeg.once("error", reject);
      ffmpeg.once("exit", code => code === 0 ? resolve() : reject(new Error(`ffmpeg exited ${code}`)));
    });
    const manifest = { example: "examples/webhook-to-partner.rkg", scenario: "webhook.timeout_then_success", viewport: rasterSize, screenshots: { before: 0, backoff: waiting, completed: head }, gif: { seconds: 30, framesPerSecond: 4, transitions }, evidence: "Real headless Chromium rendering of retained snapshots; no external audit or model call." };
    await fs.writeFile(path.join(destination, "webhook-capture.json"), `${JSON.stringify(manifest, null, 2)}\n`);
    console.log(JSON.stringify({ screenshots: manifest.screenshots, gif: "30 seconds", output: destination }));
  } finally {
    await browser?.close();
    child.kill("SIGTERM");
    await fs.rm(frames, { recursive: true });
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
