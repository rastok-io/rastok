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

async function openTour(t, viewport = { width: 1352, height: 760 }, source = "examples/keyed-retry.rkg") {
  const child = spawn(path.join(root, "target/debug/rastok"), ["simulate", source, "--ui"], { cwd: root });
  let context;
  const errors = [];
  t.after(async () => {
    // Close the SSE consumer before its server so teardown does not manufacture
    // incomplete-response console errors in a page that was otherwise healthy.
    try { await context?.close(); } finally { child.kill("SIGTERM"); }
    assert.deepEqual(errors, [], "no browser exceptions or console errors");
  });
  const url = await new Promise((resolve, reject) => {
    let output = "";
    let errors = "";
    const timeout = setTimeout(() => reject(new Error("UI startup timed out")), 30000);
    child.on("error", reject);
    child.stderr.on("data", chunk => { errors += chunk; });
    child.on("exit", code => { clearTimeout(timeout); reject(new Error(`UI exited: ${code}\n${errors}`)); });
    child.stdout.on("data", chunk => {
      output += chunk;
      const found = output.match(/ui: (http:\/\/[^\s]+)/);
      if (found) { clearTimeout(timeout); resolve(found[1]); }
    });
  });
  context = await browser.newContext({ viewport, colorScheme: "dark" });
  const page = await context.newPage();
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  await page.goto(url);
  await page.locator("#step:enabled").waitFor();
  return page;
}

async function finish(page) {
  await page.locator('[data-command="run-to-completion"]').click();
  await page.locator("#execution-state").filter({ hasText: /^completed$/ }).waitFor();
  return page.locator("#timeline").inputValue();
}

async function seek(page, transition) {
  await page.locator('#timeline').fill(String(transition));
  await page.locator('#timeline').dispatchEvent('change');
  await page.locator('#timeline-position').filter({ hasText: new RegExp(`transition ${transition}$`) }).waitFor();
}

async function capture(page, name) {
  if (!process.env.RASTOK_UI_CAPTURES) return;
  await fs.mkdir(process.env.RASTOK_UI_CAPTURES, { recursive: true });
  await page.screenshot({ path: path.join(process.env.RASTOK_UI_CAPTURES, `${name}.png`) });
}

async function presentResponse(page, release) {
  const response = page.waitForResponse(response => response.url().includes("/api/snapshot?"));
  release();
  await (await response).finished();
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
}

test("transitions have explicit transport names, a first-contact hint and an honest delayed dial", { timeout: 60000 }, async t => {
  const page = await openTour(t);
  for (const [command, label, hint] of [
    ["step", "Next transition", "advance one retained transition"],
    ["next-scenario-action", "Next action", "run one authored action to its effect"],
    ["run-to-completion", "Finish", "run to completion"],
    ["resume", "Replay", "replay retained transitions at the selected speed"],
  ]) {
    const button = page.locator(`[data-command="${command}"]`);
    assert.equal(await button.innerText(), label);
    assert.ok((await button.getAttribute("title")).includes(hint));
  }
  assert.match(await page.locator('#command-status').innerText(), /Paused before admission/);
  assert.match(await page.locator('#timeline-position').innerText(), /^LIVE · Main · transition 0$/);
  await page.locator('[data-command="next-scenario-action"]').click();
  await page.locator('#timeline-position').filter({ hasText: /^LIVE · Main · transition 1$/ }).waitFor();
  assert.equal(await page.locator('#command-status').innerText(), '1 transition · authored action reached its effect');
  await finish(page);
  for (const transition of [21, 22]) {
    await page.locator('#timeline').fill(String(transition));
    await page.locator('#timeline').dispatchEvent('change');
    await page.locator('#timeline-position').filter({ hasText: new RegExp(`transition ${transition}$`) }).waitFor();
    const dial = await page.locator('.graph-loop-dial').evaluate(el => el.firstChild.textContent);
    assert.match(dial, /⟳ 1\/2.*⏱/);
    assert.doesNotMatch(await page.locator('body').innerText(), /\bboundary\b/i);
  }
});

test('explanatory hovers expose types, capacity, command disposition, queued data and retained timers', { timeout: 60000 }, async t => {
  const page = await openTour(t);
  await finish(page);
  await seek(page, 8);
  const pin = page.locator('.graph-port[aria-label*="decide.retry"]');
  await pin.focus();
  assert.match(await page.locator('#graph-tooltip').innerText(), /output.*\n.*Loop.next/s);
  await page.locator('.graph-edge-path[aria-label*="capacity"]').first().focus();
  assert.match(await page.locator('#graph-tooltip').innerText(), /capacity 1024.*shared instance mailbox/);
  const mailbox = page.locator('.edge-counter').filter({ hasText: /≡ 1/ }).first();
  const inspector = await page.locator('[data-inspector-tab][aria-selected="true"]').getAttribute('id');
  await mailbox.hover();
  assert.equal(await page.locator('#graph-tooltip').isVisible(), true);
  assert.match(await page.locator('#graph-tooltip').innerText(), /messages waiting.*(alice|bob)/s);
  assert.equal(await page.locator('[data-inspector-tab][aria-selected="true"]').getAttribute('id'), inspector, 'hover previews the mailbox without selecting it');
  assert.equal(await page.locator('#timeline').inputValue(), '8', 'hover leaves the selected transition unchanged');
  await capture(page, 'mailbox-preview');
  await mailbox.focus();
  assert.match(await page.locator('#graph-tooltip').innerText(), /messages waiting.*(alice|bob)/s);
  await page.locator('#future summary').click();
  assert.match(await page.locator('#future-items').innerText(), /l:timeout · model · due 1.0 s/);
  await page.locator('#future summary').click();
  await page.locator('.graph-scope-type[aria-label*="Loop"]').first().focus();
  assert.match(await page.locator('#graph-tooltip').innerText(), /Commands:.*Loop.next.*Fuel: 2/s);
  await page.locator('.graph-node-name').filter({ hasText: /^model$/ }).focus();
  assert.match(await page.locator('#graph-tooltip').innerText(), /connector.*rastok\/process@2.*open instances/);
  await seek(page, 21);
  await page.locator('.graph-loop-dial').focus();
  assert.match(await page.locator('#graph-tooltip').innerText(), /applications remaining.*waiting until.*exhaustion: failed/s);
  await page.locator('#future summary').click();
  assert.match(await page.locator('#future-items').innerText(), /l:admit/);
  assert.doesNotMatch(await page.locator('#future-items').innerText(), /l:timeout/, 'a closed Call has no future timeout');
  await capture(page, 'B21-waiting');
});

test('Operator decisions highlight the exact authored arm and ordered predicate results', { timeout: 60000 }, async t => {
  const page = await openTour(t);
  await finish(page);
  const records = await page.evaluate(() => currentSnapshot.history.records.filter(record => record.owner === 'decide' && record.decision));
  for (const expected of ['TimeoutExpired', 'Exited']) {
    const record = records.find(record => record.decision.steps[0].case === expected);
    const transition = await page.evaluate(index => boundaryForRecord(index), record.record_index);
    await seek(page, transition);
    const node = page.locator('.graph-node-name').filter({ hasText: /^decide$/ });
    if (await node.locator('..').getAttribute('aria-pressed') !== 'true') await node.click();
    await page.locator('#tab-definition').click();
    assert.match(await page.locator('.decision-explanation').innerText(), new RegExp(`Selected case: ${expected}`));
    assert.ok((await page.locator('#definition .source-line.fired').allTextContents()).some(line => line.includes(`${expected} =>`)));
    await capture(page, `decision-${expected}`);
  }
  const sample = await openTour(t, undefined, 'crates/rastok/tests/fixtures/ui-explanation.rkg');
  await finish(sample);
  await sample.evaluate(() => { selectedTopologyItem = currentSnapshot.topology.nodes.find(node => node.path === 'classify').id; renderRelatedPanels(currentSnapshot); activateInspectorTab('definition'); });
  assert.match(await sample.locator('.decision-explanation').innerText(), /data < 0 = false.*seen == 0 = true/s);
  const fired = (await sample.locator('#definition .source-line.fired').allTextContents()).join('\n');
  assert.match(fired, /true =>/);
  assert.doesNotMatch(fired, /false =>/, 'an unselected nested arm must not inherit its parent highlight');
  assert.equal(await sample.locator('.graph-edge.case-selected[data-selected-case="Kept"]').count(), 1);
  await capture(sample, 'open-match');
});

test('causal selection links opening admission, manager command and readmission without changing the head', { timeout: 60000 }, async t => {
  const page = await openTour(t);
  const head = await finish(page);
  await page.keyboard.press('/');
  await page.locator('#search').fill('decide.done');
  await page.locator('#search-results button').press('Enter');
  await page.locator('#tab-history').click();
  const related = await page.locator('.causal-history-table .causal-related').allTextContents();
  assert.ok(related.some(text => text.includes('Input admitted')));
  assert.ok(related.some(text => text.includes('Loop application admitted')));
  assert.ok(related.some(text => text.includes('Loop recurrence scheduled')));
  assert.equal(await page.locator('#timeline').inputValue(), head);
  await page.locator('#tab-envelope').click();
  assert.ok(await page.locator('#envelope .identifier button').count() >= 4);
  assert.match(await page.locator('.causal-command').filter({ hasText: 'Next command' }).innerText(), /CommandProduced.*CommandConsumed.*Admit/s);
  assert.doesNotMatch(await page.locator('.causal-command').filter({ hasText: 'Done command' }).innerText(), /Admit/);
});

test('scenario actions select their retained transition and all navigation is keyboard reachable', { timeout: 60000 }, async t => {
  const page = await openTour(t);
  assert.match(await page.locator('#timeline-position').innerText(), /^LIVE · Main · transition 0$/);
  assert.match(await page.locator('#command-status').innerText(), /Paused before admission/);
  await page.keyboard.press('a');
  await page.locator('#timeline-position').filter({ hasText: /^LIVE · Main · transition 1$/ }).waitFor();
  assert.equal(await page.locator('#command-status').innerText(), '1 transition · authored action reached its effect');
  await capture(page, 'fresh-shortcut-a');
  await page.keyboard.press('ArrowLeft');
  await page.locator('#timeline-position').filter({ hasText: /transition 0$/ }).waitFor();
  await page.keyboard.press('ArrowRight');
  await page.locator('#timeline-position').filter({ hasText: /transition 1$/ }).waitFor();
  await page.keyboard.press('n');
  await page.locator('#timeline-position').filter({ hasText: /^LIVE · Main · transition 2$/ }).waitFor();
  await capture(page, 'fresh-shortcut-n');
  await finish(page);
  await page.locator('#scenario-definition').press('Enter');
  const action = page.locator('.scenario-definition-code [data-action="2"]');
  await action.press('Enter');
  await page.locator('.scenario-definition-code [data-action="2"][aria-current=step]').waitFor();
  assert.equal(await page.locator('.scenario-definition-code [aria-current=step]').getAttribute('data-action'), '2');
  assert.match(await page.locator('#timeline-position').innerText(), /^HISTORY/);
  await page.keyboard.press('Escape');
  await page.keyboard.press(' ');
  await page.locator('#timeline-position').filter({ hasText: /^REPLAY/ }).waitFor();
  await page.keyboard.press(' ');
  assert.match(await page.locator('#execution-state').innerText(), /replay paused/);
});

test('the explanatory path is reachable with Tab and keyboard alone', { timeout: 90000 }, async t => {
  const page = await openTour(t);
  const tabTo = async locator => {
    const target = locator.first();
    for (let count = 0; count < 240; count += 1) {
      if (await target.evaluate(element => element === document.activeElement)) return;
      await page.keyboard.press('Tab');
    }
    assert.fail('the requested explanation was not reachable in the Tab order');
  };
  const at = transition => page.locator('#timeline-position').filter({ hasText: new RegExp(`transition ${transition}$`) }).waitFor();
  const inspector = async name => {
    await tabTo(page.locator('[data-inspector-tab][aria-selected="true"]'));
    for (let count = 0; count < 7; count += 1) {
      if (await page.locator(`#tab-${name}`).evaluate(element => element === document.activeElement)) return;
      await page.keyboard.press('ArrowRight');
    }
    assert.fail('the requested inspector tab was not reachable with arrow keys');
  };
  const moveTo = async transition => {
    await tabTo(page.locator('#timeline'));
    await page.keyboard.press('Home');
    await at(0);
    for (let index = 1; index <= transition; index += 1) {
      await page.keyboard.press('ArrowRight');
      await at(index);
    }
  };
  const search = async query => {
    await page.keyboard.press('Escape');
    await page.keyboard.press('/');
    assert.equal(await page.locator('#search-panel').isVisible(), true, 'Escape must leave the timeline input before / opens search');
    await page.keyboard.press('ControlOrMeta+A');
    await page.keyboard.type(query);
    await tabTo(page.locator('#search-results button').filter({ hasText: new RegExp(`^${query.replaceAll('.', '\\.')}$`) }));
    await page.keyboard.press('Enter');
  };
  assert.match(await page.locator('#command-status').innerText(), /Paused before admission/);
  await page.keyboard.press('a');
  await at(1);
  await page.keyboard.press('n');
  await at(2);
  await tabTo(page.locator('[data-command="run-to-completion"]'));
  await page.keyboard.press('Enter');
  await page.locator('#execution-state').filter({ hasText: /^completed$/ }).waitFor();
  await moveTo(8);
  await tabTo(page.locator('#future summary'));
  await page.keyboard.press('Enter');
  assert.match(await page.locator('#future-items').innerText(), /l:timeout · model · due 1.0 s/);
  await page.keyboard.press('Escape');
  for (const [target, explanation] of [
    [page.locator('.graph-port[aria-label*="decide.retry"]'), /Loop.next/],
    [page.locator('.graph-edge-path[aria-label*="capacity"]'), /capacity 1024/],
    [page.locator('.edge-counter').filter({ hasText: /≡ 1/ }), /messages waiting/],
    [page.locator('.graph-scope-type[aria-label*="Loop"]'), /Commands:.*Loop.next/s],
    [page.locator('.graph-node-name').filter({ hasText: /^model$/ }), /rastok\/process@2/],
  ]) {
    await tabTo(target);
    assert.match(await page.locator('#graph-tooltip').innerText(), explanation);
  }
  await moveTo(21);
  await tabTo(page.locator('.graph-loop-dial'));
  assert.match(await page.locator('#graph-tooltip').innerText(), /waiting until/);
  assert.match(await page.locator('.graph-loop-dial').evaluate(element => element.firstChild.textContent), /⟳ 1\/2.*⏱/);
  await search('decide');
  await inspector('definition');
  await page.keyboard.press('Enter');
  assert.match(await page.locator('.decision-explanation').innerText(), /TimeoutExpired/);
  await tabTo(page.locator('#timeline'));
  await page.keyboard.press('End');
  await page.locator('#timeline-position').filter({ hasText: /^LIVE/ }).waitFor();
  assert.match(await page.locator('.decision-explanation').innerText(), /Exited/);
  await search('decide.done');
  await inspector('history');
  await page.keyboard.press('Enter');
  const lineage = (await page.locator('.causal-history-table .causal-related').allTextContents()).join('\n');
  assert.match(lineage, /Input admitted/);
  assert.match(lineage, /Loop application admitted/);
  await inspector('envelope');
  await page.keyboard.press('Enter');
  await tabTo(page.locator('#envelope .identifier > button').filter({ hasText: /^origin / }));
  await page.keyboard.press('Enter');
  assert.doesNotMatch(await page.locator('body').innerText(), /[0-9a-f]{32}/i);
  await tabTo(page.locator('#scenario-definition'));
  await page.keyboard.press('Enter');
  await tabTo(page.locator('.scenario-definition-code [data-action="2"]'));
  await page.keyboard.press('Enter');
  await page.locator('.scenario-definition-code [data-action="2"][aria-current=step]').waitFor();
  await page.keyboard.press('Escape');
  await page.keyboard.press(' ');
  await page.locator('#timeline-position').filter({ hasText: /^REPLAY/ }).waitFor();
  await page.keyboard.press(' ');
  assert.match(await page.locator('#execution-state').innerText(), /replay paused/);
});

test('failed expect shows the observed field difference and expect nothing shows the offender', { timeout: 60000 }, async t => {
  const page = await openTour(t, undefined, 'crates/rastok/tests/fixtures/ui-explanation.rkg');
  await finish(page);
  assert.equal(await page.locator('#tab-assertion').getAttribute('aria-selected'), 'true');
  assert.match(await page.locator('#assertion').innerText(), /expect app.out where data.value == 8/);
  assert.match(await page.locator('.assertion-difference').innerText(), /data.value.*differs.*7.*8/s);
  await capture(page, 'assertion-difference');
  await page.locator('#scenario-select').selectOption('inspect\u0000unwanted');
  await page.locator('#select-scenario').click();
  await page.locator('#timeline-position').filter({ hasText: /Session 1 · transition 0$/ }).waitFor();
  await finish(page);
  assert.match(await page.locator('#assertion').innerText(), /expect nothing app.out.*1 observed · 1 matching/s);
  assert.equal(await page.locator('.assertion-envelope').count(), 1);
  await capture(page, 'assertion-unwanted');
});

test('comparison aligns two keyed Loop instances and two read-only sessions at the selected transition', { timeout: 60000 }, async t => {
  const page = await openTour(t);
  await finish(page);
  await seek(page, 21);
  await page.locator('#tab-compare').click();
  assert.match(await page.locator('#compare-left').innerText(), /alice/);
  assert.match(await page.locator('#compare-right').innerText(), /bob/);
  assert.match(await page.locator('#comparison').innerText(), /Aligned at transition 21/);
  assert.equal(await page.locator('.comparison-row').count(), 3);
  await capture(page, 'comparison-instances');
  await page.locator('#return-live').click();
  await page.locator('[data-command="reset-scenario"]').click();
  await page.locator('#timeline-position').filter({ hasText: /Session 1 · transition 0$/ }).waitFor();
  await finish(page);
  await page.locator('#compare-mode').selectOption('sessions');
  await page.locator('#comparison').filter({ hasText: /Aligned at transition/ }).waitFor();
  assert.match(await page.locator('#compare-left').innerText(), /Main.*Session 1/s);
  assert.equal(await page.locator('.comparison-row.different').count(), 0);
});

for (const example of ['echo', 'jsonl', 'gauge', 'countdown', 'keyed-retry']) {
  test(`${example}: live, history and replay have no visible full ids or browser exception`, { timeout: 60000 }, async t => {
    const page = await openTour(t, undefined, `examples/${example}.rkg`);
    const assertText = async () => {
      assert.doesNotMatch(await page.locator('body').innerText(), /\b[0-9a-f]{32,}\b/i);
      assert.doesNotMatch(await page.locator('body').innerText(), /\bboundary\b/i);
    };
    await assertText();
    await finish(page);
    for (const tab of ['envelope', 'instances', 'history', 'definition', 'mailbox']) { await page.locator(`#tab-${tab}`).click(); await assertText(); }
    await seek(page, 0);
    await assertText();
    await page.locator('[data-command="resume"]').click();
    await page.locator('#timeline-position').filter({ hasText: /^REPLAY/ }).waitFor();
    await assertText();
    await page.locator('[data-command="pause"]').click();
    await capture(page, `${example}-replay`);
  });
}

test("Loop dials follow historical admissions and instance selection without synthetic ports", { timeout: 90000 }, async t => {
  const page = await openTour(t);
  const dial = page.locator('.graph-loop-dial[data-owner=""]');
  const dialText = () => dial.evaluate(element => element.firstChild.textContent.trim());
  assert.equal(await dialText(), "⟳ 0/2");
  const head = Number(await finish(page));
  assert.equal(await dialText(), "⟳ 0/2");
  const seen = new Set();
  const historicalDials = new Map();
  let multiple;
  let waiting;
  for (let boundary = 0; boundary <= head; boundary++) {
    await page.locator("#timeline").fill(String(boundary));
    await page.locator("#timeline").dispatchEvent("change");
    await page.locator("#timeline-position").filter({ hasText: new RegExp(`transition ${boundary}$`) }).waitFor();
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
  await page.locator("#timeline-position").filter({ hasText: new RegExp(`transition ${waiting - 1}$`) }).waitFor();
  await page.locator("#playback-speed").selectOption("1");
  await page.locator('[data-command="resume"]').click();
  await page.locator("#timeline-position").filter({ hasText: new RegExp(`^REPLAY.*transition ${waiting}$`) }).waitFor();
  assert.equal(await dialText(), historicalDials.get(waiting), "replay projects the same retained fold as history");
  await page.locator('[data-command="pause"]').click();
  await page.locator("#timeline").fill(String(multiple));
  await page.locator("#timeline").dispatchEvent("change");
  await page.locator("#timeline-position").filter({ hasText: new RegExp(`transition ${multiple}$`) }).waitFor();
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
  await page.locator("#timeline-position").filter({ hasText: /transition 1$/ }).waitFor();
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
  await page.locator("#timeline-position").filter({ hasText: /transition 0$/ }).waitFor();
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
    await page.locator("#timeline-position").filter({ hasText: /transition 0$/ }).waitFor();
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
  await page.locator("#timeline-position").filter({ hasText: /transition 0$/ }).waitFor();
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
  await page.locator("#timeline-position").filter({ hasText: /transition 1$/ }).waitFor();
  assert.equal(await page.locator("#command-status").isVisible(), false);
  assert.equal(await page.locator("#execution-state").innerText(), "history");
  assert.match(await page.locator("#timeline-head").innerText(), /engine completed/);
});

test("an admission without an observed delivery does not claim no boundary is selected", { timeout: 60000 }, async t => {
  const page = await openTour(t);
  await page.locator("#step").click();
  await page.locator("#timeline-position").filter({ hasText: /transition 1$/ }).waitFor();
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
  await page.locator("#timeline-position").filter({ hasText: /^LIVE · Session 1 · transition 0$/ }).waitFor();
  assert.equal(await page.locator("#command-status").isVisible(), false);
  assert.equal(await page.locator("#step").isEnabled(), true);
  await page.locator("#timeline-session").selectOption("0");
  await page.locator("#timeline-position").filter({ hasText: new RegExp(`HISTORY · Main · transition ${head}$`) }).waitFor();
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
  await capture(`transition ${boundaries.at(-1)}`);
  assert.match(await page.locator("#scenario-status").innerText(), /0 failures/);
  const readSnapshot = async () => (await page.request.get(new URL("/api/snapshot", page.url()).href)).json();
  const stepped = await readSnapshot();
  await page.locator("#tab-history").click();
  assert.ok(await page.locator(".timeline-record").count() >= 10);
  assert.equal(await page.locator('.causal-history-table td').first().evaluate(cell => cell.scrollWidth <= cell.clientWidth), true, 'record and transition positions are never clipped');
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
  await page.locator("#timeline-position").filter({ hasText: /^LIVE · Session 1 · transition 0$/ }).waitFor();
  assert.equal(Number(await finish(page)), boundaries.at(-1), "Continue and Step have the same boundary count");
  const continuous = await readSnapshot();
  assert.deepEqual(continuous.history, stepped.history, "the causal records and observed envelopes are identical");
});
