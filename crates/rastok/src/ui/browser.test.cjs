"use strict";

// Optional real-browser qualification, not a frontend/runtime dependency.
// Build `cargo build -p rastok --features hosted-ui` first. Point
// RASTOK_PLAYWRIGHT at an installed Playwright and RASTOK_CHROME at Chrome.
// RASTOK_BIN can pin an isolated binary while other feature builds run.
const assert = require("node:assert/strict");
const { test, before, after } = require("node:test");
const { spawn } = require("node:child_process");
const path = require("node:path");
const fs = require("node:fs/promises");
const os = require("node:os");
const { chromium } = require(process.env.RASTOK_PLAYWRIGHT || "playwright");
const root = path.resolve(__dirname, "../../../..");
const rastok = process.env.RASTOK_BIN || path.join(root, "target/debug/rastok");
let browser;

before(async () => {
  browser = await chromium.launch({ headless: true, executablePath: process.env.RASTOK_CHROME });
});
after(async () => { await browser?.close(); });

test('source ligatures use embedded JetBrains Mono and change glyphs without changing text', { timeout: 60000 }, async t => {
  const page = await openTour(t, { width: 1440, height: 760 }, 'examples/echo.rkg');
  const faces = await page.evaluate(async () => {
    const loaded = await document.fonts.load('13px "Rastok Source"', '-> != ==');
    return loaded.map(face => ({ family: face.family, status: face.status }));
  });
  assert.ok(faces.length > 0 && faces.every(face => face.status === 'loaded'), 'source fonts load from embedded assets, independently of installed fonts');
  const client = await page.context().newCDPSession(page);
  await client.send('DOM.enable');
  await client.send('CSS.enable');
  const { root: documentNode } = await client.send('DOM.getDocument');
  const { nodeId } = await client.send('DOM.querySelector', { nodeId: documentNode.nodeId, selector: '#source-code .syntax-punctuation' });
  const { fonts } = await client.send('CSS.getPlatformFontsForNode', { nodeId });
  assert.ok(fonts.some(font => font.isCustomFont && font.familyName === 'JetBrains Mono' && font.glyphCount > 0), 'the rendered source uses the bundled face');
  for (const [name, spelling] of [['arrow', '->'], ['not-equal', '!='], ['equal', '==']]) {
    const operator = page.locator('#source-code .syntax-punctuation').filter({ hasText: new RegExp(`^${spelling}$`) }).first();
    const enabled = await operator.screenshot({ animations: 'disabled' });
    await operator.evaluate(element => { element.style.fontFeatureSettings = '"calt" 0, "liga" 0'; });
    const disabled = await operator.screenshot({ animations: 'disabled' });
    assert.ok(!enabled.equals(disabled), `${spelling} changes its drawn glyphs when ligatures are enabled`);
    assert.equal(await operator.textContent(), spelling, 'the original operator text stays intact');
    await operator.evaluate(element => element.style.removeProperty('font-feature-settings'));
    if (process.env.RASTOK_UI_CAPTURES) {
      await fs.mkdir(process.env.RASTOK_UI_CAPTURES, { recursive: true });
      await fs.writeFile(path.join(process.env.RASTOK_UI_CAPTURES, `ligature-${name}-on.png`), enabled);
      await fs.writeFile(path.join(process.env.RASTOK_UI_CAPTURES, `ligature-${name}-off.png`), disabled);
    }
  }
  assert.equal(await page.locator('#timeline').inputValue(), '0');
  await capture(page, 'ligature-echo-jetbrains');
});

test('source regions fold with the keyboard and reveal the actual transition', { timeout: 60000 }, async t => {
  const page = await openTour(t, { width: 1440, height: 760 }, 'examples/echo.rkg');
  const fold = line => page.locator(`.source-line[data-line="${line}"] .source-fold`);
  assert.equal(await fold(5).count(), 1, 'the graph has a folding control');
  const commands = [];
  page.on('request', request => { if (request.url().includes('/api/command/')) commands.push(new URL(request.url()).pathname); });
  const typography = await page.locator('#source-code').evaluate(element => {
    const style = getComputedStyle(element);
    return { ligatures: style.fontVariantLigatures, features: style.fontFeatureSettings };
  });
  assert.match(typography.ligatures, /contextual/);
  assert.match(typography.features, /"calt"(?: 1)?(?:,|$)/);
  await fold(13).press('Space');
  assert.equal(await fold(13).getAttribute('aria-expanded'), 'false');
  assert.equal(await page.locator('.source-line[data-line="14"]').isVisible(), false);
  await fold(5).press('Enter');
  assert.equal(await page.locator('.source-line[data-line="13"]').isVisible(), false);
  await capture(page, 'fold-echo-graph-dark');
  await fold(5).press('Space');
  assert.equal(await fold(13).getAttribute('aria-expanded'), 'false', 'nested folding survives expanding a parent');
  await page.reload();
  await page.locator('#step:enabled').waitFor();
  assert.equal(await fold(13).getAttribute('aria-expanded'), 'false', 'folding survives a reload in this tab');
  assert.deepEqual(commands, [], 'folding never sends an execution command');
  await fold(5).click();
  await page.locator('#step').click();
  await page.waitForFunction(() => Number(currentSnapshot.timeline.boundary) === 1 && pendingCommandRevision === null);
  assert.equal(await page.locator('#source-code [aria-current="step"]').isVisible(), true);
  assert.equal(await fold(5).getAttribute('aria-expanded'), 'false', 'an unrelated fold stays closed');
  await fold(24).click();
  assert.equal(await page.locator('#source-code [aria-current="step"]').getAttribute('data-line'), '24', 'a folded region indicates a current transition inside it');
  await page.locator('#step').click();
  await page.waitForFunction(() => Number(currentSnapshot.timeline.boundary) === 2 && pendingCommandRevision === null);
  assert.equal(await fold(5).getAttribute('aria-expanded'), 'true');
  for (let remaining = 8; remaining > 0 && await page.evaluate(() => currentSnapshot.current_position.node_path !== 'relay'); remaining -= 1) {
    const revision = await page.evaluate(() => currentSnapshot.revision);
    await page.locator('#step').click();
    await page.waitForFunction(previous => currentSnapshot.revision > previous && pendingCommandRevision === null, revision);
  }
  assert.equal(await page.evaluate(() => currentSnapshot.current_position.node_path), 'relay');
  assert.equal(await fold(13).getAttribute('aria-expanded'), 'true', 'the transition reveals the nested operator');
  await fold(5).click();
  await page.locator('.graph-node-name').filter({ hasText: /^relay$/ }).click();
  assert.equal(await fold(5).getAttribute('aria-expanded'), 'true', 'graph selection reveals its source definition');
  await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'reduce' });
  await page.setViewportSize({ width: 1024, height: 760 });
  await capture(page, 'fold-echo-marker-light');
});

test('scenario gutter selects a fresh session and advances exactly one transition', { timeout: 60000 }, async t => {
  const page = await openTour(t, { width: 1440, height: 760 }, 'examples/echo.rkg');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const action = page.locator('.source-scenario[data-simulation="smoke"][data-scenario="filtered"]');
  assert.equal(await action.count(), 1, 'scenario declarations have gutter actions');
  const commands = [];
  page.on('request', request => { if (request.url().includes('/api/command/')) commands.push(new URL(request.url()).pathname); });
  await action.press('Enter');
  await page.locator('#timeline-position').filter({ hasText: /^LIVE · Session 1 · transition 1$/ }).waitFor();
  assert.equal(await page.locator('#scenario-select').inputValue(), 'smoke\u0000filtered');
  assert.deepEqual(commands, ['/api/command/select-scenario', '/api/command/step']);
  await capture(page, 'fold-echo-scenario-selected');
  await action.press('Space');
  await page.locator('#timeline-position').filter({ hasText: /^LIVE · Session 1 · transition 2$/ }).waitFor();
  assert.deepEqual(commands, ['/api/command/select-scenario', '/api/command/step', '/api/command/step']);
  await capture(page, 'fold-echo-scenario-step');
  await seek(page, 1);
  assert.equal(await action.isDisabled(), true, 'historical inspection cannot execute a scenario');
  assert.deepEqual(commands, ['/api/command/select-scenario', '/api/command/step', '/api/command/step']);
});

test('fold regions ignore comment and string braces and preserve shared closing lines', { timeout: 60000 }, async t => {
  const directory = await fs.mkdtemp(path.join(root, 'target/source-fold-browser-'));
  t.after(() => fs.rm(directory, { recursive: true }));
  const source = path.join(directory, 'fold.rkg');
  await fs.writeFile(source, `// { unmatched comment brace
graph fold : App {
  input in: text; output out: text
  operator relay {
    on in { out = data }
  } in -> relay -> out
}
lab fold { simulation smoke {
  scenario first { // a { comment is not a scope
    app.in = "}"
    expect app.out // }
  } scenario second { app.in = "{" }
} }
`);
  const page = await openTour(t, undefined, source);
  assert.deepEqual(await page.locator('button.source-fold').evaluateAll(buttons => buttons.map(button => Number(button.closest('[data-line]').dataset.line))), [2, 4, 8, 9]);
  await page.getByRole('button', { name: 'Collapse block at line 4', exact: true }).click();
  assert.equal(await page.locator('[data-line="5"]').isVisible(), false);
  assert.equal(await page.locator('[data-line="6"]').isVisible(), true, 'the pipe following the closing brace stays visible');
  await page.getByRole('button', { name: 'Collapse block at line 9', exact: true }).click();
  assert.equal(await page.locator('[data-line="12"]').isVisible(), true, 'the next scenario on the closing line stays visible');
  assert.equal(await page.locator('.source-scenario[data-scenario="second"]').isVisible(), true);
  await capture(page, 'fold-shared-lines');
});

test('scenario gutter waits for the selected snapshot and ignores repeated activation', { timeout: 60000 }, async t => {
  const page = await openTour(t, undefined, 'examples/echo.rkg');
  const commands = [];
  page.on('request', request => { if (request.url().includes('/api/command/')) commands.push(new URL(request.url()).pathname); });
  let release;
  let requested;
  const started = new Promise(resolve => { requested = resolve; });
  const gate = new Promise(resolve => { release = resolve; });
  await page.route('**/api/snapshot', async route => {
    const response = await route.fetch();
    const snapshot = await response.json();
    if (snapshot.timeline.session === 1 && Number(snapshot.timeline.boundary) === 0) {
      requested();
      await gate;
    }
    await route.fulfill({ response });
  });
  const action = page.locator('.source-scenario[data-scenario="filtered"]');
  await action.click();
  await started;
  assert.equal(await action.isDisabled(), true);
  assert.equal(await page.locator('#step').isDisabled(), true);
  await action.dispatchEvent('click');
  assert.deepEqual(commands, ['/api/command/select-scenario'], 'an acknowledgment does not authorize a step on the old snapshot');
  release();
  await page.locator('#timeline-position').filter({ hasText: /^LIVE · Session 1 · transition 1$/ }).waitFor();
  assert.deepEqual(commands, ['/api/command/select-scenario', '/api/command/step']);
});

test('historical navigation cancels a pending scenario gutter step', { timeout: 60000 }, async t => {
  const page = await openTour(t, undefined, 'examples/echo.rkg');
  await page.locator('#step').click();
  await page.locator('#timeline-position').filter({ hasText: /transition 1$/ }).waitFor();
  const commands = [];
  page.on('request', request => { if (request.url().includes('/api/command/')) commands.push(new URL(request.url()).pathname); });
  let release;
  let requested;
  const started = new Promise(resolve => { requested = resolve; });
  const gate = new Promise(resolve => { release = resolve; });
  await page.route('**/api/snapshot', async route => {
    const response = await route.fetch();
    const snapshot = await response.json();
    if (snapshot.timeline.session === 1) { requested(); await gate; }
    await route.fulfill({ response });
  });
  await page.locator('.source-scenario[data-scenario="filtered"]').click();
  await started;
  await page.locator('#back-boundary').click();
  await page.locator('#timeline-position').filter({ hasText: /Main · transition 0$/ }).waitFor();
  release();
  await page.waitForFunction(() => Number(liveSnapshot.timeline.session) === 1);
  assert.deepEqual(commands, ['/api/command/select-scenario']);
  assert.equal(await page.locator('#timeline').inputValue(), '0');
  await page.locator('#return-live').click();
  assert.match(await page.locator('#timeline-position').innerText(), /^LIVE · Session 1 · transition 0$/);
});

test('a refused scenario selection clears its pending gutter step', { timeout: 30000 }, async t => {
  const page = await openTour(t, undefined, 'examples/echo.rkg');
  const commands = [];
  page.on('request', request => { if (request.url().includes('/api/command/')) commands.push(new URL(request.url()).pathname); });
  // Exercise the real controller's refusal after accepting a queued command.
  await page.route('**/api/command/select-scenario?*', route => {
    const url = new URL(route.request().url());
    url.searchParams.set('scenario', 'missing');
    return route.continue({ url: url.toString() });
  });
  await page.locator('.source-scenario[data-scenario="filtered"]').click();
  await page.locator('#fault-panel').waitFor();
  await page.waitForFunction(() => pendingScenarioStep === null, null, { timeout: 3000 });
  assert.equal(await page.locator('#scenario-select').isEnabled(), true, 'selection can be corrected without reloading');
  assert.equal(await page.locator('#timeline').inputValue(), '0');
  assert.deepEqual(commands, ['/api/command/select-scenario']);
});

test('an observer can fold source but cannot execute its scenario gutter actions', { timeout: 30000 }, async t => {
  const page = await openTour(t, undefined, 'examples/echo.rkg');
  const observer = await page.context().newPage();
  const commands = [];
  const errors = [];
  observer.on('pageerror', error => errors.push(error.message));
  observer.on('request', request => { if (request.url().includes('/api/command/')) commands.push(new URL(request.url()).pathname); });
  await observer.goto(page.url());
  await observer.locator('#role').filter({ hasText: /^observer$/ }).waitFor();
  await observer.locator('.source-scenario').first().waitFor();
  assert.equal(await observer.locator('.source-scenario').evaluateAll(buttons => buttons.every(button => button.disabled)), true);
  await observer.locator('.source-scenario').first().dispatchEvent('click');
  await observer.getByRole('button', { name: 'Collapse block at line 5', exact: true }).click();
  assert.equal(await observer.locator('.source-line[data-line="13"]').isVisible(), false);
  assert.deepEqual(commands, []);
  assert.deepEqual(errors, []);
});

// The two-simultaneous-retries fixture pins intermediate transition regressions;
// the public asymmetric gallery example is also exercised in the example matrix.
async function openTour(t, viewport = { width: 1352, height: 760 }, source = "crates/rastok/tests/fixtures/ui-two-retries.rkg", commandArgs, environment = {}) {
  const child = spawn(rastok, commandArgs || ["simulate", source, "--ui"], { cwd: root, env: { ...process.env, ...environment } });
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
  assert.equal(await page.locator("#source-pane").isVisible(), true, "source is the initial primary view");
  assert.equal(await page.locator("#graph").isVisible(), true, "graph remains visible beside the source");
  return page;
}

async function finish(page) {
  await page.locator('[data-command="run-to-completion"]').click();
  await page.locator("#execution-state").filter({ hasText: /^completed$/ }).waitFor();
  return page.locator("#timeline").inputValue();
}

test('an authored advance retains each transaction and App output pins inspect the original emission', { timeout: 90000 }, async t => {
  const page = await openTour(t);
  let previousRecords = 0;
  let transactionsDuringAdvance = 0;
  for (let count = 0; count < 100; count += 1) {
    if (await page.locator('#execution-state').innerText() === 'completed') break;
    const revision = await page.evaluate(() => currentSnapshot.revision);
    await page.locator('#step').click();
    await page.waitForFunction(previous => currentSnapshot.revision > previous && pendingCommandRevision === null, revision);
    const state = await page.evaluate(previous => ({
      records: currentSnapshot.history.records.length,
      transactions: currentSnapshot.history.records.slice(previous).filter(record => record.record_type === 'TRANSACTION').length,
      action: currentSnapshot.scenario.current.last_action,
    }), previousRecords);
    assert.ok(state.transactions <= 1, `one transition hid ${state.transactions} transactions`);
    if (state.action === 'advance') transactionsDuringAdvance += state.transactions;
    previousRecords = state.records;
  }
  assert.equal(await page.locator('#execution-state').innerText(), 'completed');
  assert.ok(transactionsDuringAdvance > 1, 'several time-driven transactions were inspected separately');
  const output = await page.evaluate(() => currentSnapshot.history.outputs.findLast(output => output.port === 'done'));
  await page.locator('.graph-scope.root > .graph-port[data-port="done"]').press('Enter');
  assert.equal(await page.evaluate(() => selectedEnvelopeId), output.envelope_id);
  assert.equal(await page.locator('#tab-envelope').getAttribute('aria-selected'), 'true');
  assert.match(await page.locator('#envelope').innerText(), /attempts/);
  const keyLines = await page.locator('#envelope .data-tree-key').filter({ hasText: /^graph_version$/ }).evaluate(key => {
    const range = document.createRange();
    range.selectNodeContents(key);
    return range.getClientRects().length;
  });
  assert.equal(keyLines, 1, 'ordinary META names remain readable without splitting graph_version');
  await page.locator('#tab-history').click();
  await page.locator('#history > details').filter({ hasText: 'Observed envelopes' }).locator(':scope > summary').click();
  const rows = page.locator('.delivery-history-table tbody tr');
  assert.ok(await rows.count() > 10);
  assert.ok(await rows.first().evaluate(row => row.getBoundingClientRect().height < 48));
  assert.equal(await rows.first().locator('td > details[open]').count(), 0, 'payload trees start folded, without losing data');
});

test('the resident gateway explains independent Process Sessions without starting a child', { timeout: 60000 }, async t => {
  const page = await openTour(t, undefined, 'examples/agent-gateway.rkg');
  await finish(page);
  assert.deepEqual(await page.evaluate(() => currentSnapshot.scenario.current.failures), []);
  await page.locator('#tab-instances').click();
  assert.match(await page.locator('#panel-instances').innerText(), /conversation/);
  assert.match(await page.locator('#timeline-position').innerText(), /^LIVE/);
  await seek(page, 1);
  assert.match(await page.locator('#timeline-position').innerText(), /^HISTORY/);
  await page.locator('[data-command="resume"]').click();
  await page.locator('#timeline-position').filter({ hasText: /^REPLAY/ }).waitFor();
  await page.locator('[data-command="pause"]').click();
});

test('the Responses adapter preserves typed evidence and hides the bearer in LIVE, HISTORY and REPLAY', { timeout: 120000 }, async t => {
  const source = 'examples/agent-http.rkg';
  const credential = 'browser-responses-fixture-not-a-real-credential';
  const page = await openTour(t, undefined, source, [
    'simulate', source, '--ui', '--deployment', 'examples/agent-http.deployment.json',
  ], {
    RASTOK_SHOWCASE_MODEL_TOKEN: credential,
    RASTOK_SHOWCASE_MODEL_TOKEN_VERSION: 'fixture-v1',
  });
  const assertPrivate = async () => {
    assert.ok(!(await page.locator('body').innerText()).includes(credential));
    assert.ok(!(await page.evaluate(() => JSON.stringify(currentSnapshot))).includes(credential));
    assert.equal(await page.locator('#cost').count(), 0, 'no report or manufactured total');
  };
  const scenarios = ['two_sessions', 'missing_usage', 'observed_zero', 'schema_failure', 'transport_failure', 'http_errors'];
  for (const [index, scenario] of scenarios.entries()) {
    if (index) {
      await page.locator('#return-live').click();
      await page.locator('#scenario-select').selectOption(`responses\u0000${scenario}`);
      await page.locator('#timeline-position').filter({ hasText: new RegExp(`^LIVE · Session ${index} · transition 0$`) }).waitFor();
    }
    const head = Number(await finish(page));
    assert.deepEqual(await page.evaluate(() => currentSnapshot.scenario.current.failures), [], scenario);
    await assertPrivate();
    if (scenario === 'two_sessions' || scenario === 'observed_zero') {
      await page.locator('.graph-port[aria-label*="model.out"]').press('Enter');
      assert.equal(await page.locator('#tab-envelope').getAttribute('aria-selected'), 'true');
      const usage = page.locator('#envelope summary').filter({ hasText: /^usage/ });
      await usage.click();
      await usage.locator('..').locator(':scope > .data-tree-children > details > summary').filter({ hasText: /^value/ }).click();
      assert.match(await page.locator('#envelope').innerText(), /input_tokens/);
      const inputTokens = page.locator('#envelope .data-tree-key').filter({ hasText: /^input_tokens$/ }).locator('..');
      assert.match(await inputTokens.innerText(), scenario === 'observed_zero' ? /input_tokens\s+0$/ : /input_tokens\s+12$/);
    }
    await seek(page, Math.min(8, head - 1));
    await assertPrivate();
    await page.locator('[data-command="resume"]').click();
    await page.locator('#timeline-position').filter({ hasText: /^REPLAY/ }).waitFor();
    await page.locator('[data-command="pause"]').click();
    await assertPrivate();
  }
});

test('Retry keeps held attempts inspectable without drawing Loop state or downstream delivery', { timeout: 90000 }, async t => {
  const page = await openTour(t, undefined, 'crates/rastok/tests/fixtures/ui-retry.rkg');
  let held;
  let waiting;
  for (let i = 0; i < 100; i += 1) {
    if (await page.locator('#execution-state').innerText() === 'completed') break;
    const revision = await page.evaluate(() => currentSnapshot.revision);
    await page.locator('#step').click();
    await page.waitForFunction(previous => currentSnapshot.revision > previous && pendingCommandRevision === null, revision);
    const frame = await page.evaluate(() => currentSnapshot);
    const operation = frame.retry_operations[0];
    if (operation?.phase === 'Held') {
      held = frame.timeline.boundary;
      assert.deepEqual(frame.history.outputs, []);
      assert.ok(frame.topology.activity.routes.every(route => route.phase === 'consumed'));
    }
    if (operation?.phase === 'AwaitingAdmission') waiting = frame.timeline.boundary;
  }
  assert.notEqual(held, undefined);
  assert.notEqual(waiting, undefined);
  assert.equal(await page.locator('.graph-loop-dial').count(), 0);
  await page.locator('#tab-instances').click();
  assert.match(await page.locator('#retry-operations').innerText(), /Attempt 2\/2 · final result published/);
  assert.equal(await page.locator('#retry-operations tbody tr').count(), 2);
  assert.match(await page.locator('#retry-operations tbody tr').first().innerText(), /ServerError.*held by Retry/s);
  assert.match(await page.locator('#retry-operations tbody tr').last().innerText(), /Completed.*published/s);
  assert.match(await page.locator('#history-summary').textContent(), /envelope observations/);
  assert.doesNotMatch(await page.locator('#history-summary').textContent(), /deliver/);
  assert.doesNotMatch(await page.locator('body').innerText(), /private-(held|final)-response|[a-f0-9]{32}/);
  await seek(page, held);
  assert.equal(await page.locator('#retry-operations tbody tr').count(), 1);
  await page.locator('#retry-operations summary').click();
  await page.locator('#retry-operations details .identifier > button').first().click();
  assert.equal(await page.locator('#tab-envelope').getAttribute('aria-selected'), 'true');
  assert.match(await page.locator('#envelope').innerText(), /tokens.*10/s);
  assert.doesNotMatch(await page.locator('#envelope').innerText(), /private-held-response/);
  await seek(page, waiting);
  await page.locator('#future summary').click();
  assert.match(await page.locator('#future-items').innerText(), /Retry attempt.*due 0.1 s/s);
});

test('injection history names the recorded routing key, not a guessed tenant field', { timeout: 60000 }, async t => {
  const page = await openTour(t, undefined, 'crates/rastok/tests/fixtures/ui-instance-keys.rkg');
  await finish(page);
  await page.locator('#tab-instances').click();
  const history = await page.locator('#injections').innerText();
  assert.match(history, /Routed instance.*actual-account/is);
  assert.doesNotMatch(history, /not-the-instance-key/);
  await page.locator('#injections tbody tr').press('Enter');
  await page.locator('#timeline-position').filter({ hasText: /^HISTORY.*transition 1$/ }).waitFor();
  assert.match(await page.locator('#injections').innerText(), /actual-account/);
});

test('the flagship keeps readable graph text clear of badges, ports and pipes', { timeout: 60000 }, async t => {
  const page = await openTour(t, undefined, undefined, [
    'simulate', 'examples/webhook-to-partner.rkg', '--ui',
    '--deployment', 'examples/webhook-to-partner.deployment.json',
  ], {
    RASTOK_SHOWCASE_PARTNER_TOKEN: 'browser-fixture-not-a-real-credential',
    RASTOK_SHOWCASE_PARTNER_TOKEN_VERSION: 'fixture-v1',
  });
  // Use the dividers to give the complete drawing room above its readability floor.
  await page.locator('#workspace-splitter').press('Home');
  await page.locator('#details-splitter').press('Home');
  for (const transition of ['initial', 1, 'completed', 8, 'narrow']) {
    if (transition === 1) {
      await page.locator('#step').click();
      await page.locator('#timeline-position').filter({ hasText: /transition 1$/ }).waitFor();
    }
    if (transition === 'completed') await finish(page);
    if (transition === 8) await seek(page, 8);
    if (transition === 'narrow') await page.setViewportSize({ width: 700, height: 760 });
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    await page.getByRole('button', { name: 'Readable size', exact: true }).click();
    await capture(page, `webhook-geometry-${transition}`);
    const collisions = await page.evaluate(() => {
      const labels = [...document.querySelectorAll('.graph-svg text')].map(element => ({
        text: element.firstChild?.textContent,
        box: element.getBoundingClientRect(),
      }));
      const overlap = (a, b) => Math.min(a.right, b.right) - Math.max(a.left, b.left) > 0.5
        && Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top) > 0.5;
      const collisions = labels.flatMap((a, index) => labels.slice(index + 1)
        .filter(b => overlap(a.box, b.box))
        .map(b => `${a.text} overlaps ${b.text}`));
      for (const node of document.querySelectorAll('.graph-node')) {
        const title = node.querySelector('.graph-node-name');
        if (!title) continue;
        const body = node.querySelector('.graph-node-body').getBoundingClientRect();
        const box = title.getBoundingClientRect();
        if (box.left < body.left || box.right > body.right || box.top < body.top || box.bottom > body.bottom) {
          collisions.push(`${title.firstChild.textContent} exceeds its box`);
        }
        const scale = title.getScreenCTM().a;
        if (Math.abs(parseFloat(getComputedStyle(title).fontSize) * scale - 12) > 0.1) {
          collisions.push(`${title.firstChild.textContent} is not at interface text size`);
        }
      }
      for (const path of document.querySelectorAll('.graph-edge-path')) {
        const transform = path.getScreenCTM();
        const samples = [];
        for (let distance = 0; distance <= path.getTotalLength(); distance += 2) {
          samples.push(path.getPointAtLength(distance).matrixTransform(transform));
        }
        for (const label of document.querySelectorAll('.graph-scope-name, .graph-scope-type, .graph-node-name, .graph-node-type, .graph-node-port-label, .graph-scope-port-label')) {
          const box = label.getBoundingClientRect();
          if (samples.some(point => point.x > box.left && point.x < box.right && point.y > box.top && point.y < box.bottom)) {
            collisions.push(`${path.closest('.graph-edge').dataset.route} crosses ${label.firstChild.textContent}`);
          }
        }
      }
      return collisions;
    });
    assert.deepEqual(collisions, [], String(transition));
    await page.getByRole('button', { name: 'Fit graph', exact: true }).click();
    if (transition !== 'narrow') {
      const canvas = await page.locator('.graph-canvas').boundingBox();
      const scope = await page.locator('.graph-scope.root .graph-scope-body').boundingBox();
      assert.ok(scope.x >= canvas.x && scope.x + scope.width <= canvas.x + canvas.width, 'the complete flagship fits horizontally');
      assert.ok(scope.y >= canvas.y && scope.y + scope.height <= canvas.y + canvas.height, 'the complete flagship fits vertically');
    }
    if (transition === 8) {
      assert.equal(await page.locator('body.historical-view').count(), 1);
      assert.equal(await page.locator('#return-live').isVisible(), true);
    }
  }
});

test('a decoded HttpServer source is inspectable without an artificial App input', { timeout: 60000 }, async t => {
  const page = await openTour(t, undefined, 'crates/rastok/tests/fixtures/lab-http-source.rkg');
  assert.match(await page.locator('.envelope-inspector').innerText(), /listener\.out/);
  assert.match(await page.locator('.envelope-inspector').innerText(), /first/);
  assert.equal(await page.evaluate(() => currentSnapshot.scenario.current.next_injection.position), 'before-source-output');
  await page.locator('#step').click();
  await page.waitForFunction(() => currentSnapshot.history.sources?.length === 1);
  assert.equal(await page.evaluate(() => currentSnapshot.history.sources[0].node), 'listener');
  assert.equal(await page.evaluate(() => currentSnapshot.history.sources[0].data.content), 'first');
  await finish(page);
  assert.deepEqual(await page.evaluate(() => currentSnapshot.scenario.current.failures), []);
  assert.equal(await page.evaluate(() => currentSnapshot.history.sources.length), 2);
});

test('the webhook gallery remains classified in LIVE, HISTORY and REPLAY without a measurement report', { timeout: 120000 }, async t => {
  const source = 'examples/webhook-to-partner.rkg';
  const credential = 'browser-fixture-not-a-real-credential';
  const page = await openTour(t, undefined, source, [
    'simulate', source, '--ui', '--deployment', 'examples/webhook-to-partner.deployment.json',
  ], {
    RASTOK_SHOWCASE_PARTNER_TOKEN: credential,
    RASTOK_SHOWCASE_PARTNER_TOKEN_VERSION: 'fixture-v1',
  });
  const assertClassified = async () => {
    assert.doesNotMatch(await page.locator('body').innerText(), /alice@example\.test|browser-fixture-not-a-real-credential/);
    const snapshot = await page.evaluate(() => JSON.stringify(currentSnapshot));
    assert.ok(!snapshot.includes('alice@example.test'), 'no raw email in the browser snapshot');
    assert.ok(!snapshot.includes(credential), 'the physical secret never reaches the browser');
    assert.equal(await page.evaluate(() => Object.hasOwn(currentSnapshot, 'cost_report')), false);
    assert.equal(await page.locator('#cost').count(), 0);
    assert.equal(await page.locator('.source-scenario').count(), 0, 'withheld scenario source has no gutter actions');
  };
  await assertClassified();
  await capture(page, 'webhook-B0');
  await page.locator('#step').click();
  await page.locator('#timeline-position').filter({ hasText: /transition 1$/ }).waitFor();
  await assertClassified();
  await capture(page, 'webhook-B1');
  const scenarios = ['nominal', 'timeout_then_success', 'partner_dead', 'duplicate', 'invalid_payload', 'http_500'];
  for (const [index, scenario] of scenarios.entries()) {
    if (index) {
      await page.locator('#return-live').click();
      await page.locator('#scenario-select').selectOption(`webhook\u0000${scenario}`);
      await page.locator('#timeline-position').filter({ hasText: new RegExp(`^LIVE · Session ${index} · transition 0$`) }).waitFor();
    }
    const head = Number(await finish(page));
    assert.deepEqual(await page.evaluate(() => currentSnapshot.scenario.current.failures), [], scenario);
    await assertClassified();
    if (!index) {
      await page.locator('#tab-envelope').click();
      await capture(page, 'webhook-completed');
    }
    await seek(page, Math.min(8, head - 1));
    await assertClassified();
    if (!index) await capture(page, 'webhook-B8');
    await page.locator('[data-command="resume"]').click();
    await page.locator('#timeline-position').filter({ hasText: /^REPLAY/ }).waitFor();
    await page.locator('[data-command="pause"]').click();
    await assertClassified();
  }
});

test('a real recording is navigable without any execution command or Driver', { timeout: 60000 }, async t => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'rastok-recording-browser-'));
  t.after(() => fs.rm(directory, { recursive: true }));
  const recording = path.join(directory, 'echo.rkrec');
  const run = spawn(rastok, ['run', 'examples/echo.rkg', '--record', recording], { cwd: root });
  let stderr = '';
  run.stderr.on('data', chunk => { stderr += chunk; });
  run.stdout.resume();
  const ended = new Promise((resolve, reject) => { run.on('error', reject); run.on('exit', resolve); });
  run.stdin.end('recorded hello\n');
  assert.equal(await ended, 0, stderr);
  const page = await openTour(t, undefined, undefined, ['replay', recording, '--ui']);
  const commands = [];
  page.on('request', request => { if (request.url().includes('/api/command/')) commands.push(request.url()); });
  assert.equal(await page.evaluate(() => currentSnapshot.mode), 'replay');
  assert.equal(await page.locator('.source-scenario').count(), 0, 'recordings do not expose execution gutter actions');
  assert.match(await page.locator('#command-status').innerText(), /Recorded run.*read-only/);
  await page.locator('[data-source-view="0"]').click();
  assert.match(await page.locator('#source-status').innerText(), /recorded excerpts/);
  assert.equal(await page.evaluate(() => currentSnapshot.sources[0].complete), false);
  assert.equal(await page.locator('#timeline').inputValue(), '0');
  for (const command of ['reset-scenario', 'next-scenario-action', 'run-until-expect', 'retry-step']) {
    const button = page.locator(`[data-command="${command}"]`);
    if (await button.count()) assert.equal(await button.isDisabled(), true);
  }
  await page.locator('#step').click();
  await page.locator('#timeline-position').filter({ hasText: /transition 1$/ }).waitFor();
  await page.keyboard.press('ArrowLeft');
  await page.locator('#timeline-position').filter({ hasText: /transition 0$/ }).waitFor();
  await page.locator('[data-command="resume"]').click();
  await page.locator('#timeline-position').filter({ hasText: /^REPLAY/ }).waitFor();
  await page.locator('[data-command="pause"]').click();
  assert.match(await page.locator('#execution-state').innerText(), /playback paused/);
  const paused = await page.locator('#timeline').inputValue();
  await page.waitForTimeout(1200);
  assert.equal(await page.locator('#timeline').inputValue(), paused);
  const head = await page.evaluate(() => liveSnapshot.timeline.boundary);
  await page.locator('[data-command="run-to-completion"]').click();
  await page.locator('#timeline-position').filter({ hasText: new RegExp(`^RECORDED · Main · transition ${head}$`) }).waitFor();
  assert.equal(await page.locator('#execution-state').innerText(), 'recorded end');
  await page.locator('#tab-envelope').click();
  assert.match(await page.locator('#envelope').innerText(), /shell.exit.*Exited/s);
  await page.locator('#tab-history').click();
  await page.locator('#history summary').filter({ hasText: /Observed envelopes/ }).click();
  await page.locator('.delivery-history-table tbody tr').first().locator('td:last-child > details > summary').click();
  assert.match(await page.locator('#history').innerText(), /recorded hello/);
  assert.deepEqual(commands, [], 'navigation must not ask the server to execute');
});

async function seek(page, transition) {
  await page.locator('#timeline').fill(String(transition));
  await page.locator('#timeline').dispatchEvent('change');
  await page.locator('#timeline-position').filter({ hasText: new RegExp(`transition ${transition}$`) }).waitFor();
}

// Time advancement retains real intermediate commits. Locate the waiting
// interval by the Loop fold, not by an incidental presentation ordinal.
async function bothRunsWaiting(page) {
  const transitions = await page.evaluate(async () => {
    const session = currentSnapshot.timeline.session;
    const result = [];
    for (const frame of timelineFrames(session)) {
      const response = await fetch(`/api/snapshot?session=${session}&boundary=${frame.boundary}`);
      const snapshot = await response.json();
      const runs = snapshot.loop_runs.filter(run => run.active);
      if (runs.length === 2 && runs.every(run => run.waiting && Number(run.application) === 1)) {
        result.push(Number(frame.boundary));
      }
    }
    return result;
  });
  assert.ok(transitions.length > 0, 'both first applications have recorded delayed recurrences');
  return transitions;
}

async function capture(page, name) {
  if (!process.env.RASTOK_UI_CAPTURES) return;
  await fs.mkdir(process.env.RASTOK_UI_CAPTURES, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
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
    ["step", "Next transition F10", "advance one recorded transition"],
    ["next-scenario-action", "Next action", "run one authored action to its effect"],
    ["run-to-completion", "Finish", "run to completion"],
    ["resume", "Play F8", "advance through recorded transitions at the selected speed"],
  ]) {
    const button = page.locator(`[data-command="${command}"]`);
    assert.equal((await button.innerText()).replace(/\s+/g, ' '), label);
    assert.ok((await button.getAttribute("title")).includes(hint));
  }
  assert.equal(await page.locator('.timeline-bar').getAttribute('aria-label'), 'Recorded execution timeline');
  assert.equal(await page.locator('#playback-speed').getAttribute('aria-label'), 'Playback speed');
  assert.equal(await page.locator('#timeline').getAttribute('aria-label'), 'Recorded transition');
  assert.match(await page.locator('#command-status').innerText(), /Paused before admission/);
  assert.match(await page.locator('#timeline-position').innerText(), /^LIVE · Main · transition 0$/);
  await page.locator('[data-command="next-scenario-action"]').click();
  await page.waitForFunction(() => commandOrigin === null && currentSnapshot.scenario.current.quiescent && currentSnapshot.execution.state === 'paused');
  const transitions = Number(await page.locator('#timeline').inputValue());
  assert.ok(transitions > 1, 'admission includes its intermediate Execution transitions');
  assert.equal(await page.locator('#command-status').innerText(), `${transitions} transitions · authored action reached its effect`);
  await page.locator('[data-command="next-scenario-action"]').click();
  await page.waitForFunction(previous => Number(currentSnapshot.timeline.boundary) === previous + 1 && commandOrigin === null, transitions);
  assert.equal(await page.locator('#command-status').innerText(), '1 transition · authored action reached its effect');
  const beforeStep = await page.evaluate(() => currentSnapshot.revision);
  await page.locator('[data-command="step"]').click();
  await page.waitForFunction(previous => currentSnapshot.revision > previous && commandOrigin === null, beforeStep);
  const action = await page.evaluate(() => currentSnapshot.scenario.current.next_action_index);
  assert.equal(await page.locator('#command-status').innerText(), `no execution work ready · advanced to A${action}`);
  await finish(page);
  for (const transition of await bothRunsWaiting(page)) {
    await page.locator('#timeline').fill(String(transition));
    await page.locator('#timeline').dispatchEvent('change');
    await page.locator('#timeline-position').filter({ hasText: new RegExp(`transition ${transition}$`) }).waitFor();
    const dial = await page.locator('.graph-loop-dial').evaluate(el => el.firstChild.textContent);
    assert.match(dial, /⟳ 1\/2.*⏱/);
    const interfaceText = (await page.locator('.app-bar, #command-status, #source-status, #details-pane').allInnerTexts()).join('\n');
    assert.doesNotMatch(interfaceText, /\bboundary\b/i);
  }
});

test('explanatory hovers expose types, capacity, command disposition, queued data and recorded deadlines', { timeout: 60000 }, async t => {
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
  assert.match(await page.locator('#graph-tooltip').innerText(), /connector.*std\/process@6.*open instances/);
  await seek(page, (await bothRunsWaiting(page))[0]);
  await page.locator('.graph-loop-dial').focus();
  assert.match(await page.locator('#graph-tooltip').innerText(), /applications remaining.*waiting until.*exhaustion: failed/s);
  await page.locator('#future summary').click();
  assert.match(await page.locator('#future-items').innerText(), /admitted/);
  assert.doesNotMatch(await page.locator('#future-items').innerText(), /l:timeout/, 'a closed Call has no future timeout');
  await capture(page, 'both-runs-waiting');
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
    await page.locator('#tab-instances').click();
    assert.match(await page.locator('.decision-explanation').innerText(), new RegExp(`Selected case: ${expected}`));
    assert.match(await page.locator('#node-info').innerText(), /Manager commands.*Loop.next/s);
    assert.ok((await page.locator('#source-code .source-line.fired').allTextContents()).some(line => line.includes(`${expected} =>`)));
    await capture(page, `decision-${expected}`);
  }
  const sample = await openTour(t, undefined, 'crates/rastok/tests/fixtures/ui-explanation.rkg');
  await finish(sample);
  const decision = await sample.evaluate(() => boundaryForRecord(currentSnapshot.history.records.findLast(record => record.owner === 'classify' && record.decision).record_index));
  await seek(sample, decision);
  await sample.evaluate(() => { selectTopologyItem(currentSnapshot.topology.nodes.find(node => node.path === 'classify').id); activateInspectorTab('instances'); });
  assert.match(await sample.locator('.decision-explanation').innerText(), /data < 0 = false.*seen == 0 = true/s);
  const fired = (await sample.locator('#source-code .source-line.fired').allTextContents()).join('\n');
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

test('scenario actions select their recorded transition and all navigation is keyboard reachable', { timeout: 60000 }, async t => {
  const page = await openTour(t);
  assert.match(await page.locator('#timeline-position').innerText(), /^LIVE · Main · transition 0$/);
  assert.match(await page.locator('#command-status').innerText(), /Paused before admission/);
  await page.keyboard.press('a');
  await page.waitForFunction(() => commandOrigin === null && currentSnapshot.scenario.current.quiescent && currentSnapshot.execution.state === 'paused');
  const actionHead = Number(await page.locator('#timeline').inputValue());
  assert.ok(actionHead > 1, 'Next action retains its intermediate Execution transitions');
  assert.equal(await page.locator('#command-status').innerText(), `${actionHead} transitions · authored action reached its effect`);
  await capture(page, 'fresh-shortcut-a');
  await page.keyboard.press('ArrowLeft');
  await page.locator('#timeline-position').filter({ hasText: new RegExp(`transition ${actionHead - 1}$`) }).waitFor();
  await page.keyboard.press('ArrowRight');
  await page.locator('#timeline-position').filter({ hasText: new RegExp(`transition ${actionHead}$`) }).waitFor();
  await page.keyboard.press('n');
  await page.locator('#timeline-position').filter({ hasText: new RegExp(`^LIVE · Main · transition ${actionHead + 1}$`) }).waitFor();
  await capture(page, 'fresh-shortcut-n');
  await finish(page);
  await page.locator('#scenario-status').press('Enter');
  const action = page.locator('#source-code [data-action="2"]');
  await action.press('Enter');
  await page.locator('#source-code [data-action="2"][aria-current=step]').waitFor();
  assert.equal(await page.locator('#source-code [aria-current=step]').getAttribute('data-action'), '2');
  assert.match(await page.locator('#timeline-position').innerText(), /^HISTORY/);
  await page.keyboard.press('Escape');
  await page.keyboard.press(' ');
  await page.locator('#timeline-position').filter({ hasText: /^REPLAY/ }).waitFor();
  await page.keyboard.press(' ');
  assert.match(await page.locator('#execution-state').innerText(), /playback paused/);
});

test('the explanatory path is reachable with Tab and keyboard alone', { timeout: 90000 }, async t => {
  const page = await openTour(t);
  const tabTo = async locator => {
    const target = locator.first();
    const limit = await page.locator('button, input, select, a[href], summary, [tabindex="0"]').count()
      + await page.locator('#source-code [data-source-token]').count();
    for (let count = 0; count <= limit; count += 1) {
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
  await page.waitForFunction(() => commandOrigin === null && currentSnapshot.scenario.current.quiescent && currentSnapshot.execution.state === 'paused');
  const actionHead = Number(await page.locator('#timeline').inputValue());
  await page.keyboard.press('n');
  await at(actionHead + 1);
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
    [page.locator('.graph-node-name').filter({ hasText: /^model$/ }), /std\/process@6/],
  ]) {
    await tabTo(target);
    assert.match(await page.locator('#graph-tooltip').innerText(), explanation);
  }
  await moveTo((await bothRunsWaiting(page))[0]);
  await tabTo(page.locator('.graph-loop-dial'));
  assert.match(await page.locator('#graph-tooltip').innerText(), /waiting until/);
  assert.match(await page.locator('.graph-loop-dial').evaluate(element => element.firstChild.textContent), /⟳ 1\/2.*⏱/);
  await search('decide');
  await inspector('instances');
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
  await tabTo(page.locator('#scenario-status'));
  await page.keyboard.press('Enter');
  await tabTo(page.locator('#source-code [data-action="2"]'));
  await page.keyboard.press('Enter');
  await page.locator('#source-code [data-action="2"][aria-current=step]').waitFor();
  await page.keyboard.press('Escape');
  await page.keyboard.press(' ');
  await page.locator('#timeline-position').filter({ hasText: /^REPLAY/ }).waitFor();
  await page.keyboard.press(' ');
  assert.match(await page.locator('#execution-state').innerText(), /playback paused/);
});

test('failed expect shows the observed field difference and absence shows the offender', { timeout: 60000 }, async t => {
  const page = await openTour(t, undefined, 'crates/rastok/tests/fixtures/ui-explanation.rkg');
  await finish(page);
  assert.equal(await page.locator('#tab-assertion').getAttribute('aria-selected'), 'true');
  assert.match(await page.locator('#assertion').innerText(), /expect app.out where data.value == 8/);
  assert.match(await page.locator('.assertion-difference').innerText(), /data.value.*differs.*7.*8/s);
  await capture(page, 'assertion-difference');
  await page.locator('#scenario-select').selectOption('inspect\u0000unwanted');
  await page.locator('#timeline-position').filter({ hasText: /Session 1 · transition 0$/ }).waitFor();
  await finish(page);
  assert.match(await page.locator('#assertion').innerText(), /expect app\.out == nothing.*1 observed · 1 matching/s);
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

for (const example of ['echo', 'jsonl', 'gauge', 'countdown', 'keyed-retry', 'file-pipeline', 'sensor-alarm']) {
  test(`${example}: live, history and replay have no visible full ids or browser exception`, { timeout: 60000 }, async t => {
    const page = await openTour(t, undefined, `examples/${example}.rkg`);
    const assertText = async () => {
      assert.doesNotMatch(await page.locator('body').innerText(), /\b[0-9a-f]{32,}\b/i);
      assert.doesNotMatch(await page.locator('body').innerText(), /\bboundary\b/i);
    };
    await assertText();
    await finish(page);
    for (const tab of ['envelope', 'instances', 'history', 'mailbox']) { await page.locator(`#tab-${tab}`).click(); await assertText(); }
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
      const waitingRuns = await page.evaluate(() => currentSnapshot.loop_runs.filter(run => run.active && run.waiting));
      assert.ok(waitingRuns.length > 0);
      assert.ok(waitingRuns.every(run => Number(run.application) === 1), "a waiting run has not admitted its second application; another run may have");
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
  assert.equal(await dialText(), historicalDials.get(waiting), "replay projects the same recorded fold as history");
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

test("cyclic chat layout separates forward work, quiet mailboxes and focused connections", { timeout: 60000 }, async t => {
  const page = await openTour(t, { width: 1440, height: 760 }, 'examples/agent-chat.rkg');
  await capture(page, 'graph-chat-overview');
  const positions = await page.evaluate(() => {
    const graph = graphLayout.materializeGraph(currentSnapshot.topology);
    const layout = graphLayout.layoutGraph(graph.nodes, graph.edges, currentSnapshot.topology.owners);
    return Object.fromEntries(graph.nodes.filter(node => node.path).map(node => [node.path, layout.positions.get(node.id)]));
  });
  for (const [from, to] of [['page', 'html'], ['clients', 'validate'],
    ['conversation.admit', 'conversation.model'], ['conversation.model', 'conversation.address_reply']]) {
    assert.ok(positions[from].x + positions[from].width < positions[to].x, `${from} precedes ${to}`);
  }
  const commands = [];
  page.on('request', request => { if (request.method() === 'POST') commands.push(new URL(request.url()).pathname); });
  const empty = page.locator('.edge-counter.empty-mailbox').first();
  assert.equal(await empty.locator('rect').evaluate(rect => getComputedStyle(rect).fill), 'rgba(0, 0, 0, 0)');
  const node = page.locator('.graph-node').filter({ has: page.locator('.graph-node-name').filter({ hasText: /^validate$/ }) });
  await node.hover();
  assert.ok(await page.locator('.graph-edge.connection-related').count());
  assert.ok(await page.locator('.graph-edge.connection-muted').count());
  assert.equal(await page.locator('.graph-edge.connection-muted').first().evaluate(edge => getComputedStyle(edge).opacity), '0.2');
  await capture(page, 'graph-chat-hover');
  await page.locator('#source-tabs').hover();
  assert.equal(await page.locator('.graph-edge.connection-muted').count(), 0);
  await node.focus();
  assert.ok(await page.locator('.graph-edge.connection-muted').count(), 'keyboard focus exposes the same neighborhood');
  await empty.focus();
  assert.equal(await page.locator('.graph-edge.connection-related').count(), 1);
  await empty.press('Enter');
  assert.equal(await page.locator('#tab-mailbox').getAttribute('aria-selected'), 'true');
  assert.match(await page.locator('#mailbox').innerText(), /0 messages waiting/);
  assert.deepEqual(commands, []);
  await page.locator('#step').click();
  await page.waitForFunction(() => Number(currentSnapshot.timeline.boundary) === 1 && pendingCommandRevision === null);
  const occupied = page.locator('.edge-counter:not(.empty-mailbox)').first();
  assert.ok(await occupied.isVisible());
  assert.notEqual(await occupied.locator('rect').evaluate(rect => getComputedStyle(rect).fill), 'rgba(0, 0, 0, 0)');
  await page.setViewportSize({ width: 1024, height: 760 });
  await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'reduce' });
  await capture(page, 'graph-chat-1024-light');
});

test("chat route details preserve separate tracks and legible arrivals", { timeout: 60000 }, async t => {
  const page = await openTour(t, { width: 1440, height: 760 }, 'examples/agent-chat.rkg');
  await capture(page, 'graph-chat-track-details');
  const routes = await page.evaluate(() => {
    const graph = graphLayout.materializeGraph(currentSnapshot.topology);
    const layout = graphLayout.layoutGraph(graph.nodes, graph.edges, currentSnapshot.topology.owners);
    const routed = graphLayout.routeGraphEdges(graph, layout);
    return Object.fromEntries(graph.edges.map(edge => {
      const route = routed.get(edge.index);
      return [edge.route, { ...route, segments: graphLayout.curvedCorridorSegments(route.presentationPoints) }];
    }));
  });
  await t.test('unrelated horizontal tracks do not merge', () => {
    for (const [left, right] of [
      ['.peer>validate.in', 'validate.accepted>conversation.admit.in'],
      ['validate.rejected>clients.in', 'conversation.address_reply.out>clients.in'],
    ]) {
      for (const a of routes[left].segments.filter(s => s.from.y === s.to.y)) {
        for (const b of routes[right].segments.filter(s => s.from.y === s.to.y)) {
          const overlap = Math.min(Math.max(a.from.x, a.to.x), Math.max(b.from.x, b.to.x))
            - Math.max(Math.min(a.from.x, a.to.x), Math.min(b.from.x, b.to.x));
          const sharedArrival = left.endsWith('>clients.in') && right.endsWith('>clients.in')
            && Math.max(Math.min(a.from.x, a.to.x), Math.min(b.from.x, b.to.x)) >= routes[left].end.x - 32
            && a.from.y === routes[left].end.y;
          if (!sharedArrival && overlap > 1) assert.ok(Math.abs(a.from.y - b.from.y) >= 10,
            `${left} and ${right} need distinct horizontal tracks`);
        }
      }
    }
  });
  await t.test('direct requests leave above the bypass to remember', () => {
    const request = routes['conversation.admit.request>conversation.model.in'];
    const remember = routes['conversation.admit.remember>conversation.address_reply.remember'];
    assert.ok(request.start.y < remember.start.y, 'the short request precedes the lower bypass');
    assert.ok(request.end.y < remember.end.y, 'the real target anchors preserve the same order');
  });
  await t.test('the close scope output reaches reply without vertical overshoot', () => {
    const route = routes['conversation.address_reply.out>.reply'];
    const portal = route.portals.at(-1);
    assert.ok(portal, 'the retained conversation.out portal exists');
    const tail = route.segments.filter(segment => segment.from.x >= portal.x);
    assert.ok(tail.length);
    for (const segment of tail) {
      for (const point of [segment.from, segment.c1, segment.c2, segment.to]) {
        assert.ok(point.y >= Math.min(portal.y, route.end.y) - 0.01
          && point.y <= Math.max(portal.y, route.end.y) + 0.01,
        'the final approach stays between the scope output and parent pin');
      }
    }
  });
  await t.test('empty counters mask the wire behind their text', async () => {
    const style = await page.locator('.edge-counter.empty-mailbox text').first().evaluate(text => {
      const style = getComputedStyle(text);
      return { stroke: style.stroke, width: parseFloat(style.strokeWidth), order: style.paintOrder };
    });
    assert.notEqual(style.stroke, 'none');
    assert.ok(style.width >= 4);
    assert.match(style.order, /^stroke/);
  });
  if (process.env.RASTOK_UI_CAPTURES) {
    await page.setViewportSize({ width: 2560, height: 1200 });
    await page.getByRole('button', { name: 'Fit graph', exact: true }).click();
    await page.locator('.graph-canvas').screenshot({ path: path.join(process.env.RASTOK_UI_CAPTURES, 'graph-chat-tracks-detail-dark.png') });
    await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'reduce' });
    await page.locator('.graph-canvas').screenshot({ path: path.join(process.env.RASTOK_UI_CAPTURES, 'graph-chat-tracks-detail-light.png') });
  }
});

test("chat return tracks clear frames and keep rounded turns", { timeout: 60000 }, async t => {
  const page = await openTour(t, { width: 1440, height: 760 }, 'examples/agent-chat.rkg');
  await capture(page, 'graph-chat-clearance');
  const drawing = await page.evaluate(() => {
    const graph = graphLayout.materializeGraph(currentSnapshot.topology);
    const layout = graphLayout.layoutGraph(graph.nodes, graph.edges, currentSnapshot.topology.owners);
    const routes = graphLayout.routeGraphEdges(graph, layout);
    return {
      boxes: [...layout.scopes.values(), ...graph.nodes.filter(node => !['boundary', 'discard'].includes(node.kind))
        .map(node => layout.positions.get(node.id))],
      routes: graph.edges.map(edge => ({ name: edge.route, ...routes.get(edge.index),
        segments: graphLayout.curvedCorridorSegments(routes.get(edge.index).presentationPoints) })),
    };
  });
  await t.test('straight pipes have a visible gap from parallel box borders', () => {
    for (const route of drawing.routes) for (const segment of route.segments) {
      for (const [along, across, length] of [['x', 'y', 'width'], ['y', 'x', 'height']]) {
        if (![segment.c1, segment.c2, segment.to].every(point => point[across] === segment.from[across])) continue;
        for (const box of drawing.boxes) {
          const overlap = Math.min(Math.max(segment.from[along], segment.to[along]), box[along] + box[length])
            - Math.max(Math.min(segment.from[along], segment.to[along]), box[along]);
          if (overlap <= 1) continue;
          const thickness = across === 'x' ? box.width : box.height;
          for (const border of [box[across], box[across] + thickness]) {
            assert.ok(Math.abs(segment.from[across] - border) >= 5.99,
              `${route.name} needs clearance from the ${across}=${border} frame`);
          }
        }
      }
    }
  });
  await t.test('return bends have continuous tangents instead of reversing into a curve', () => {
    for (const route of drawing.routes.filter(route => (route.portals.at(-1)?.x ?? route.start.x) > route.end.x)) {
      const tangent = (segment, start) => {
        const dx = start ? segment.c1.x - segment.from.x : segment.to.x - segment.c2.x;
        const dy = start ? segment.c1.y - segment.from.y : segment.to.y - segment.c2.y;
        return Math.hypot(dx, dy) > 0.001 ? [dx, dy] : [segment.to.x - segment.from.x, segment.to.y - segment.from.y];
      };
      for (let index = 1; index < route.segments.length; index += 1) {
        const a = tangent(route.segments[index - 1], false), b = tangent(route.segments[index], true);
        const cosine = (a[0] * b[0] + a[1] * b[1]) / (Math.hypot(...a) * Math.hypot(...b));
        assert.ok(cosine > 0.999, `${route.name} has an abrupt turn at segment ${index}`);
      }
    }
  });
});

test("graph selection reveals the source block without a Definition tab or execution", { timeout: 60000 }, async t => {
  const page = await openTour(t, undefined, 'examples/agent-chat.rkg');
  const commands = [];
  page.on('request', request => { if (request.method() === 'POST') commands.push(new URL(request.url()).pathname); });
  assert.equal(await page.locator('#tab-definition, #panel-definition').count(), 0);
  assert.deepEqual(await page.locator('[data-inspector-tab]').evaluateAll(tabs => tabs.map(tab => tab.dataset.inspectorTab)),
    ['envelope', 'mailbox', 'instances', 'history', 'assertion', 'compare']);
  await page.locator('.graph-node-name').filter({ hasText: /^validate$/ }).click();
  const definition = await page.evaluate(() => selectedTopologyItemView(currentSnapshot.topology).definition);
  const selected = await page.locator('#source-code .selected-definition').evaluateAll(lines => lines.map(line => Number(line.dataset.line)));
  assert.deepEqual(selected, Array.from({ length: definition.last_line - definition.first_line + 1 }, (_, index) => definition.first_line + index));
  assert.equal(await page.locator('#tab-envelope').getAttribute('aria-selected'), 'true');
  assert.equal(await page.locator('#timeline').inputValue(), '0');
  await page.locator('#tab-instances').click();
  assert.ok(await page.locator('#node-context').isVisible());
  assert.match(await page.locator('#node-info').innerText(), /validate/);
  await page.waitForFunction(line => {
    const source = document.querySelector('#source-code').getBoundingClientRect();
    const selected = document.querySelector(`#source-code .source-line[data-line="${line}"]`).getBoundingClientRect();
    return selected.top >= source.top && selected.bottom <= source.bottom;
  }, definition.first_line);
  assert.equal(await page.evaluate(() => scrollY), 0, 'selection scrolls only the source pane');
  await capture(page, 'node-chat-source-selection');
  await page.locator('#node-context > summary').click();
  await page.locator('#scenario-status').click();
  assert.equal(await page.locator('#tab-instances').getAttribute('aria-selected'), 'true');
  assert.ok(await page.locator('#source-code .selected-definition').count());
  assert.equal(await page.locator('#node-context').isVisible(), false);
  assert.deepEqual(commands, [], 'source and inspector navigation never execute a command');
});

test("keyboard node selection retains focus and the inspector tab", { timeout: 60000 }, async t => {
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
  assert.match(await page.locator("#command-status").innerText(), /Playback paused/);
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
  assert.equal(await page.locator("#timeline-session").isVisible(), false, "a single session needs no selector");
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
});

test("scenario selection shows authored DSL and reset selects a fresh B0", { timeout: 60000 }, async t => {
  const page = await openTour(t);
  await page.locator("#scenario-status").click();
  assert.match(await page.locator("#source-code .selected-definition").allTextContents().then(lines => lines.join('\n')), /app.in = \{ tenant = "alice"/);
  assert.equal(await page.locator('#source-code .current-position').count(), 0, 'before admission has no current transition');
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
  assert.deepEqual(boundaries, boundaries.map((_, index) => index), "Step never skips a recorded boundary");
  await capture(`transition ${boundaries.at(-1)}`);
  assert.match(await page.locator("#scenario-status").innerText(), /0 failures/);
  const readSnapshot = async () => (await page.request.get(new URL("/api/snapshot", page.url()).href)).json();
  const stepped = await readSnapshot();
  await page.locator("#tab-history").click();
  assert.ok(await page.locator(".timeline-record").count() >= 10);
  assert.equal(await page.locator('.causal-history-table td').first().evaluate(cell => cell.scrollWidth <= cell.clientWidth), true, 'record and transition positions are never clipped');
  await capture("completed-history");
  await page.locator('#workspace-splitter').press('Home');
  await page.locator('#workspace-splitter').press('ArrowRight');
  await page.locator('#details-splitter').press('Home');
  const graph = await page.locator(".graph-canvas").boundingBox();
  const scope = await page.locator(".graph-scope.root .graph-scope-body").boundingBox();
  assert.ok(scope.x >= graph.x && scope.x + scope.width <= graph.x + graph.width);
  assert.ok(scope.y >= graph.y && scope.y + scope.height <= graph.y + graph.height);
  const graphPointAtCentre = () => page.locator('.graph-svg').evaluate(svg => {
    const canvas = svg.closest('.graph-canvas').getBoundingClientRect();
    const point = new DOMPoint(canvas.x + canvas.width / 2, canvas.y + canvas.height / 2).matrixTransform(svg.getScreenCTM().inverse());
    return { x: point.x, y: point.y };
  });
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const centre = await graphPointAtCentre();
  await page.getByRole("button", { name: "Zoom in", exact: true }).click();
  assert.equal(await page.locator(".graph-zoom-value").innerText(), "120%");
  assert.match(await page.locator('.graph-zoom-value').getAttribute('title'), /relative to Fit graph.*canvas center/);
  const zoomed = await graphPointAtCentre();
  assert.ok(Math.abs(centre.x - zoomed.x) < 0.1 && Math.abs(centre.y - zoomed.y) < 0.1, `toolbar zoom keeps the canvas center fixed in graph coordinates: ${JSON.stringify({ centre, zoomed })}`);
  await page.getByRole("button", { name: "Fit graph", exact: true }).click();
  assert.equal(await page.locator('.graph-zoom-value').innerText(), '100%');
  const splitter = page.locator("#workspace-splitter");
  await splitter.focus();
  const width = Number(await splitter.getAttribute("aria-valuenow"));
  await page.keyboard.press("ArrowLeft");
  assert.equal(Number(await splitter.getAttribute("aria-valuenow")), width - 24);
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


test('source positions, keyboard summaries and graph selection share the selected transition', { timeout: 60000 }, async t => {
  const page = await openTour(t, { width: 1440, height: 900 }, 'examples/countdown.rkg');
  await page.locator('[data-source-view="0"]').click();
  assert.equal(await page.locator('#source-code .syntax-keyword').first().innerText(), 'graph');
  assert.equal(await page.locator('#source-pane input, #source-pane form, #source-pane [contenteditable=true]').count(), 0);
  await finish(page);
  const transition = await page.evaluate(async () => {
    const frames = timelineIndex.sessions.find(session => session.session === currentSnapshot.timeline.session).frames;
    for (const frame of frames) {
      const view = await fetch(`/api/snapshot?session=${currentSnapshot.timeline.session}&boundary=${frame.boundary}`).then(response => response.json());
      if (view.current_position.selected_arm && view.loop_runs.some(run => run.active)) return frame.boundary;
    }
  });
  assert.ok(transition);
  await seek(page, transition);
  assert.equal(await page.locator('#source-code [aria-current="step"]').getAttribute('data-line'), String(await page.evaluate(() => currentSnapshot.current_position.first_line)));
  const counter = page.locator('#source-code [data-source-token]').filter({ hasText: /^Loop.measure$/ });
  await counter.focus();
  assert.match(await page.locator('#source-tooltip').innerText(), /Loop counter.*application/s);
  await counter.press('Tab');
  assert.equal(await page.locator('#source-tooltip a').evaluate(link => link === document.activeElement), true);
  await page.keyboard.press('Enter');
  assert.equal(await page.locator('#tab-instances').getAttribute('aria-selected'), 'true');
  await counter.focus();
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('#source-tooltip').isVisible(), false);
  assert.equal(await counter.evaluate(token => token === document.activeElement), true);
  assert.match(await page.locator('.graph-node.active').textContent(), /count/);
  await page.evaluate(() => selectTopologyItem(null));
  await page.locator('.graph-node-name').filter({ hasText: /^count$/ }).click();
  assert.equal(await page.evaluate(() => selectedTopologyItemView(currentSnapshot.topology).path), 'count');
  await page.locator('[data-source-view="0"]').click();
  assert.ok(await page.locator('#source-code .selected-definition').count() > 0);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await seek(page, 0);
  assert.equal(await page.locator('#source-code [aria-current="step"]').count(), 0);
  assert.match(await page.locator('#source-status').innerText(), /before admission/);
});

test('source-first webhook qualification covers timeout, redaction, Retry, Reset and viewport preferences', { timeout: 90000 }, async t => {
  const instructions = await fs.readFile(path.join(root, 'docs/site/examples/04-webhook-to-partner.md'), 'utf8');
  const fixture = instructions.match(/RASTOK_SHOWCASE_PARTNER_TOKEN=([^\s]+)/)[1];
  const source = 'examples/webhook-to-partner.rkg';
  const page = await openTour(t, { width: 1440, height: 760 }, source,
    ['simulate', source, '--ui', '--deployment', 'examples/webhook-to-partner.deployment.json'], {
      RASTOK_SHOWCASE_PARTNER_TOKEN: fixture,
      RASTOK_SHOWCASE_PARTNER_TOKEN_VERSION: 'fixture-v1',
    });
  await page.locator('[data-source-view="0"]').click();
  await page.locator('#scenario-select').selectOption('webhook\u0000timeout_then_success');
  await page.locator('#timeline-position').filter({ hasText: /^LIVE · Session 1 · transition 0$/ }).waitFor();
  const step = async () => {
    const revision = await page.evaluate(() => currentSnapshot.revision);
    await page.locator('#step').click();
    await page.waitForFunction(previous => currentSnapshot.revision > previous && pendingCommandRevision === null, revision);
    assert.ok(!(await page.evaluate(() => JSON.stringify(currentSnapshot))).includes(fixture), 'fixture material never enters the projection');
  };
  const settled = async () => {
    await page.waitForTimeout(400);
    const marker = page.locator('#source-code [aria-current="step"]');
    if (await marker.count()) assert.equal(await marker.evaluate(line => {
      const pane = document.querySelector('#source-code').getBoundingClientRect();
      const rect = line.getBoundingClientRect();
      return rect.top >= pane.top && rect.bottom <= pane.bottom;
    }), true, 'the marker stays inside the source viewport');
    assert.equal(await page.evaluate(() => scrollY), 0, 'source scrolling does not move the page');
  };
  await step();
  await settled();
  await capture(page, 'source-webhook-first-transition-1440-dark');
  for (let count = 0; count < 60; count += 1) {
    if (await page.evaluate(() => currentSnapshot.retry_operations.some(operation => operation.phase === 'AwaitingAdmission'))) break;
    await step();
  }
  assert.equal(await page.evaluate(() => currentSnapshot.retry_operations[0].attempts[0].terminal), 'TimeoutExpired');
  await settled();
  await capture(page, 'source-webhook-after-timeout-1440-dark');
  const partner = page.locator('#source-code [data-source-token]').filter({ hasText: /^partner$/ }).first();
  await partner.hover();
  assert.match(await page.locator('#source-tooltip').innerText(), /port envelope.*\[REDACTED\]/s);
  await capture(page, 'source-webhook-partner-port-redacted');
  const credential = page.locator('#source-code [data-source-token]').filter({ hasText: /^auth_bearer$/ });
  await credential.focus();
  assert.match(await page.locator('#source-tooltip').innerText(), /secret binding.*\[REDACTED\]/s);
  assert.equal(await page.locator('#source-tooltip a').getAttribute('href'), '#panel-instances');
  await capture(page, 'source-webhook-secret-binding-redacted');
  await credential.press('Tab');
  await page.keyboard.press('Enter');
  assert.equal(await page.locator('#tab-instances').getAttribute('aria-selected'), 'true');
  assert.match(await page.locator('#node-info').innerText(), /deliver.partner/);
  const retry = page.locator('#source-code [data-source-token]').filter({ hasText: /^Retry.attempts$/ });
  await retry.focus();
  assert.match(await page.locator('#source-tooltip').innerText(), /Retry counter.*attempt 1\/2/s);
  await capture(page, 'source-webhook-retry-counter');
  await retry.press('Tab');
  assert.equal(await page.locator('#source-tooltip a').getAttribute('href'), '#panel-instances');
  assert.equal(await page.locator('#source-tooltip a').evaluate(link => link === document.activeElement), true);
  await page.keyboard.press('Enter');
  assert.equal(await page.locator('#tab-instances').getAttribute('aria-selected'), 'true');
  await page.evaluate(() => selectTopologyItem(null));
  await page.locator('.graph-node-name').filter({ hasText: /^partner$/ }).click();
  assert.equal(await page.evaluate(() => selectedTopologyItemView(currentSnapshot.topology).path), 'deliver.partner');
  assert.equal(await page.evaluate(() => selectedSourceDefinition.first_line), await page.evaluate(() => selectedTopologyItemView(currentSnapshot.topology).definition.first_line));
  await page.mouse.move(3, 3);
  await capture(page, 'source-webhook-graph-selection-sync');
  await page.locator('[data-source-view="0"]').click();
  await settled();
  assert.ok(await page.locator('#source-code .selected-definition').count());
  for (const width of [1440, 1024]) {
    await page.setViewportSize({ width, height: 760 });
    for (const colorScheme of ['dark', 'light']) {
      await page.emulateMedia({ colorScheme });
      await capture(page, `source-webhook-${width}-${colorScheme}`);
    }
  }
  await page.emulateMedia({ reducedMotion: 'reduce', colorScheme: 'dark' });
  await page.evaluate(() => {
    const pane = document.querySelector('#source-code');
    const scroll = pane.scrollTo.bind(pane);
    pane.scrollTo = options => { globalThis.sourceScrollBehavior = options.behavior; scroll(options); };
  });
  await seek(page, 1);
  assert.equal(await page.evaluate(() => sourceScrollBehavior), 'auto');
  await capture(page, 'source-webhook-1024-reduced-motion');
  await page.locator('#return-live').click();
  await page.locator('[data-command="reset-scenario"]').click();
  await page.locator('#timeline-position').filter({ hasText: /^LIVE · Session 2 · transition 0$/ }).waitFor();
  assert.equal(await page.locator('#source-code [aria-current="step"], #source-code .selected-definition').count(), 0);
  assert.equal(await page.locator('[data-source-view="0"]').getAttribute('aria-selected'), 'true');
  assert.equal(await page.locator('#source-pane input, #source-pane form, #source-pane [contenteditable=true], #source-tooltip input').count(), 0);
  await capture(page, 'source-webhook-reset');
});

test('source-first countdown qualification exposes the changing Loop counter', { timeout: 60000 }, async t => {
  const page = await openTour(t, { width: 1440, height: 760 }, 'examples/countdown.rkg');
  await page.locator('[data-source-view="0"]').click();
  for (let count = 0; count < 40; count += 1) {
    if (await page.evaluate(() => currentSnapshot.loop_runs.some(run => run.active && run.application === '2'))) break;
    const revision = await page.evaluate(() => currentSnapshot.revision);
    await page.locator('#step').click();
    await page.waitForFunction(previous => currentSnapshot.revision > previous && pendingCommandRevision === null, revision);
  }
  const loop = page.locator('#source-code [data-source-token]').filter({ hasText: /^Loop.measure$/ });
  await loop.focus();
  assert.match(await page.locator('#source-tooltip').innerText(), /Loop counter.*application 2/s);
  for (const width of [1440, 1024]) {
    await page.setViewportSize({ width, height: 760 });
    for (const colorScheme of ['dark', 'light']) {
      await page.emulateMedia({ colorScheme });
      await loop.focus();
      await capture(page, `source-countdown-counter-${width}-${colorScheme}`);
    }
  }
  await loop.press('Escape');
  await page.locator('[data-command="reset-scenario"]').click();
  await page.locator('#timeline-position').filter({ hasText: /^LIVE · Session 1 · transition 0$/ }).waitFor();
  assert.equal(await page.locator('#source-code [aria-current="step"]').count(), 0);
  await capture(page, 'source-countdown-reset');
});

test('source, inspector and graph share two persistent accessible splitters', { timeout: 60000 }, async t => {
  const page = await openTour(t, { width: 1440, height: 900 }, 'examples/echo.rkg');
  const geometry = () => page.evaluate(() => {
    const box = selector => {
      const { x, y, width, height } = document.querySelector(selector).getBoundingClientRect();
      return { x, y, width, height };
    };
    return { source: box('#source-section'), details: box('#details-pane'),
      inspector: box('#inspector'), graph: box('#graph-section') };
  });
  const initial = await geometry();
  assert.ok(Math.abs(initial.source.width - initial.details.width) < 2, 'columns start at equal widths');
  assert.ok(Math.abs(initial.inspector.height - initial.graph.height) < 2, 'right panes start at equal heights');
  assert.ok(initial.details.x > initial.source.x && initial.graph.y > initial.inspector.y);
  const status = await page.locator('#command-status').boundingBox();
  const heading = await page.locator('.source-section .graph-heading').boundingBox();
  assert.ok(status.y >= heading.y && status.y + status.height <= heading.y + heading.height, 'status fits in the source header');
  const scenarioStatus = await page.locator('#scenario-status').boundingBox();
  const sourceCode = await page.locator('#source-code').boundingBox();
  assert.ok(scenarioStatus.y >= heading.y + heading.height && scenarioStatus.y + scenarioStatus.height <= sourceCode.y,
    'scenario progress fits between the source header and code');
  assert.equal(await page.locator('[data-inspector-tab]').count(), 6);
  assert.equal(await page.locator('[data-source-view="graph"]').count(), 0, 'graph does not replace the source');
  const boundary = await page.locator('#timeline').inputValue();
  const drag = async (selector, dx, dy) => {
    const rect = await page.locator(selector).boundingBox();
    const x = rect.x + rect.width / 2, y = rect.y + rect.height / 2;
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.mouse.move(x + dx, y + dy, { steps: 4 });
    await page.mouse.up();
  };
  await capture(page, 'split-echo-equal-1440-dark');
  await drag('#workspace-splitter', 80, 0);
  await drag('#details-splitter', 0, 50);
  const dragged = await geometry();
  assert.ok(Math.abs(dragged.source.width - initial.source.width - 80) < 2);
  assert.ok(Math.abs(dragged.inspector.height - initial.inspector.height - 50) < 2,
    JSON.stringify({ initial, dragged }));
  for (const [selector, key] of [['#workspace-splitter', 'ArrowLeft'], ['#details-splitter', 'ArrowUp']]) {
    const splitter = page.locator(selector);
    await splitter.focus();
    const size = Number(await splitter.getAttribute('aria-valuenow'));
    await splitter.press(key);
    assert.equal(Number(await splitter.getAttribute('aria-valuenow')), size - 24);
    await splitter.press('Home');
    assert.equal(Number(await splitter.getAttribute('aria-valuenow')), Number(await splitter.getAttribute('aria-valuemin')));
    await splitter.press('End');
    assert.equal(Number(await splitter.getAttribute('aria-valuenow')), Number(await splitter.getAttribute('aria-valuemax')));
  }
  await drag('#workspace-splitter', -180, 0);
  await drag('#details-splitter', 0, -180);
  const adjusted = await geometry();
  assert.equal(await page.locator('#timeline').inputValue(), boundary, 'layout keys do not navigate or execute');
  await page.reload();
  await page.locator('#step:enabled').waitFor();
  const restored = await geometry();
  assert.ok(Math.abs(restored.source.width - adjusted.source.width) < 2);
  assert.ok(Math.abs(restored.inspector.height - adjusted.inspector.height) < 2);
  await page.setViewportSize({ width: 1024, height: 760 });
  await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'reduce' });
  assert.equal(await page.locator('#source-pane').isVisible(), true);
  assert.equal(await page.locator('#graph').isVisible(), true);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await capture(page, 'split-echo-adjusted-1024-light');
  await page.setViewportSize({ width: 700, height: 760 });
  assert.equal(await page.locator('#workspace-splitter').isVisible(), true);
  await page.waitForFunction(() => document.querySelector('#workspace-splitter').getAttribute('aria-orientation') === 'horizontal');
  assert.equal(await page.locator('#details-splitter').isVisible(), true);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const returned = await geometry();
  assert.ok(Math.abs(returned.source.width - adjusted.source.width) < 2, 'narrow layout does not overwrite the desktop proportions');
  assert.ok(Math.abs(returned.inspector.height - adjusted.inspector.height) < 2);
});

test('debugger function keys use existing controls and preserve native input shortcuts', { timeout: 60000 }, async t => {
  const page = await openTour(t, { width: 1440, height: 900 }, 'examples/echo.rkg');
  const commands = [];
  page.on('request', request => { if (request.url().includes('/api/command/')) commands.push(new URL(request.url()).pathname); });
  await page.locator('#source-code').focus();
  const tokens = page.locator('#source-code [data-source-token]');
  const firstTokenName = await tokens.first().textContent();
  await tokens.first().focus();
  await tokens.nth(1).hover();
  await page.keyboard.press('Tab');
  assert.equal(await page.locator('#source-tooltip a').evaluate(link => link === document.activeElement), true);
  assert.ok((await page.locator('#source-tooltip strong').innerText()).startsWith(`${firstTokenName} · `),
    'Tab inspects the focused identifier even when the pointer hovers another');
  await page.keyboard.press('Shift+Tab');
  assert.equal(await tokens.first().evaluate(token => token === document.activeElement), true);
  await page.keyboard.press('Tab');
  await page.keyboard.press('Tab');
  assert.equal(await tokens.nth(1).evaluate(token => token === document.activeElement), true, 'tooltip links do not trap the Tab order');
  await page.keyboard.press('Escape');
  assert.match(await page.locator('#step').getAttribute('title'), /F10 or n/);
  assert.match(await page.locator('[data-command=resume]').getAttribute('aria-keyshortcuts'), /F8/);
  await page.keyboard.press('F10');
  await page.locator('#timeline-position').filter({ hasText: /^LIVE · Main · transition 1$/ }).waitFor();
  assert.deepEqual(commands, ['/api/command/step']);
  await finish(page);
  await seek(page, 0);
  const before = commands.length;
  await page.locator('#source-code').focus();
  await page.keyboard.press('F10');
  await page.locator('#timeline-position').filter({ hasText: /^HISTORY · Main · transition 1$/ }).waitFor();
  await page.keyboard.press('F8');
  await page.locator('#timeline-position').filter({ hasText: /^REPLAY/ }).waitFor();
  await page.keyboard.press('F8');
  await page.locator('#execution-state').filter({ hasText: /^playback paused$/ }).waitFor();
  assert.equal(commands.length, before, 'function keys navigate retained history without executing commands');
  const boundary = await page.locator('#timeline').inputValue();
  await page.locator('#open-search').click();
  await page.locator('#search').fill('relay');
  await page.keyboard.press('F10');
  await page.keyboard.press('F8');
  assert.equal(await page.locator('#timeline').inputValue(), boundary);
  assert.equal(await page.locator('#search').inputValue(), 'relay');
  const untouched = await page.evaluate(() => {
    const events = [{ key: 'F5' }, { key: 'F11' }, { key: 'F10', shiftKey: true },
      { key: 'F8', ctrlKey: true }, { key: 'F10', metaKey: true }, { key: 'F8', altKey: true },
      { key: 'F10', isComposing: true }, { key: 'F10', repeat: true }, { key: 'F8', repeat: true }];
    return events.every(options => document.querySelector('#source-code').dispatchEvent(
      new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...options })));
  });
  assert.equal(untouched, true, 'browser, context-menu and composition keys are not canceled');
  assert.equal(commands.length, before);
});

test('audit: source focus excludes static identifiers and summaries use readable fields', { timeout: 60000 }, async t => {
  const page = await openTour(t, { width: 1440, height: 760 }, 'examples/countdown.rkg');
  assert.equal(await page.locator('#source-code [data-source-token][aria-label$=": identifier"]').count(), 0);
  assert.ok(await page.locator('#source-code .syntax-comment').count() > 0);
  assert.equal(await page.locator('#source-code .syntax-type').filter({ hasText: /^int$/ }).count() > 0, true);
  for (let count = 0; count < 40; count += 1) {
    if (await page.evaluate(() => currentSnapshot.loop_runs.some(run => run.active && run.application === '2'))) break;
    const revision = await page.evaluate(() => currentSnapshot.revision);
    await page.locator('#step').click();
    await page.waitForFunction(previous => currentSnapshot.revision > previous && pendingCommandRevision === null, revision);
  }
  const counter = page.locator('#source-code [data-source-token]').filter({ hasText: /^Loop.measure$/ });
  assert.equal(await counter.getAttribute('data-value-available'), 'true');
  await counter.focus();
  assert.match(await page.locator('#source-tooltip').innerText(), /application 2.*remaining 3/s);
  assert.doesNotMatch(await page.locator('#source-tooltip').innerText(), /\{"application"/);
  await capture(page, 'audit-loop-summary');
});

test('audit: graph fit shows the complete drawing in the equal split and after resize', { timeout: 60000 }, async t => {
  const page = await openTour(t, { width: 1440, height: 760 });
  const fits = () => page.evaluate(() => {
    const canvas = document.querySelector('.graph-canvas').getBoundingClientRect();
    const graph = document.querySelector('.graph-svg').getBoundingClientRect();
    return graph.left >= canvas.left && graph.right <= canvas.right + 1
      && graph.top >= canvas.top && graph.bottom <= canvas.bottom + 1;
  });
  await page.getByRole('button', { name: 'Fit graph', exact: true }).click();
  assert.equal(await fits(), true, 'Fit shows the complete drawing, including its output ports');
  const top = await page.locator('.workspace').boundingBox();
  await page.keyboard.press('Escape');
  const after = await page.locator('.workspace').boundingBox();
  assert.equal(after.y, top.y, 'dismissing command status does not shift the workspace');
  assert.ok(after.y < 80, 'there is no permanent command-status row');
  await capture(page, 'audit-equal-fit-1440');
  await page.getByRole('button', { name: 'Readable size', exact: true }).click();
  await page.getByRole('button', { name: 'Fit graph', exact: true }).click();
  assert.equal(await fits(), true);
  await page.setViewportSize({ width: 1024, height: 760 });
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  assert.equal(await fits(), true);
  await capture(page, 'audit-equal-fit-1024');
  await page.setViewportSize({ width: 700, height: 760 });
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const graph = await page.locator('#graph-section').boundingBox();
  assert.ok(graph.y < 760 && graph.height > 100, 'the narrow viewport exposes the graph');
  assert.equal(await page.locator('#workspace-splitter').getAttribute('aria-orientation'), 'horizontal');
  await capture(page, 'audit-narrow-three-panes');
});

test('audit: history selection keeps source and node context together', { timeout: 60000 }, async t => {
  const page = await openTour(t);
  await finish(page);
  await page.evaluate(() => selectTopologyItem(currentSnapshot.topology.nodes.find(node => node.path === 'model').id));
  await page.locator('#tab-history').click();
  const record = await page.evaluate(() => currentSnapshot.history.records.find(record => record.owner === 'decide' && record.decision).record_index);
  // Clear the node filter to expose the complete retained journal.
  await page.evaluate(() => { selectedTopologyItem = null; renderRelatedPanels(currentSnapshot); });
  await page.locator(`.timeline-record[data-record="${record}"]`).press('Enter');
  await page.waitForFunction(() => selectedTopologyItemView(currentSnapshot.topology)?.path === 'decide');
  assert.equal(await page.evaluate(() => selectedSourceDefinition?.first_line),
    await page.evaluate(() => selectedTopologyItemView(currentSnapshot.topology).definition.first_line));
  assert.equal(await page.locator('.graph-node.selected').count(), 1);
  await page.locator('#tab-instances').click();
  assert.equal(await page.locator('#node-info pre').count(), 0, 'source text is shown once');
  assert.ok(await page.locator('#node-info .decision-explanation').isVisible());
  await capture(page, 'audit-history-source-selection');
});

test('audit: compact controls keep commands explicit and instance sections useful', { timeout: 60000 }, async t => {
  const page = await openTour(t, { width: 1440, height: 760 }, 'examples/countdown.rkg');
  const commands = [];
  page.on('request', request => { if (request.url().includes('/api/command/')) commands.push(new URL(request.url()).pathname); });
  assert.equal(await page.locator('.transport .playback-toggle:visible').count(), 1);
  assert.equal(await page.locator('.transport [data-command="retry-step"]').count(), 0);
  assert.equal(await page.locator('#fault-panel [data-command="retry-step"]').count(), 1);
  assert.equal(await page.locator('.timeline-session').isVisible(), false);
  await page.locator('#tab-instances').click();
  const emptySections = page.locator('.instance-section[data-count="0"]');
  assert.ok(await emptySections.count());
  assert.equal(await emptySections.evaluateAll(sections => sections.every(section => !section.open)), true);
  await page.locator('#scenario-status').click();
  assert.ok(await page.locator('#source-code .selected-definition').count());
  const scenario = await page.locator('#scenario-select option').nth(1).getAttribute('value');
  await page.locator('#scenario-select').selectOption(scenario);
  await page.locator('#timeline-position').filter({ hasText: /^LIVE · Session 1 · transition 0$/ }).waitFor();
  assert.deepEqual(commands, ['/api/command/select-scenario']);
  assert.equal(await page.locator('#source-code .selected-definition').count(), 0, 'scenario change clears the previous source selection');
  assert.equal(await page.locator('.timeline-session').isVisible(), true);
  await page.locator('#tab-instances').click();
  const populated = page.locator('.instance-section').filter({ has: page.locator('#state') });
  for (let transition = 1; transition <= 8; transition += 1) {
    await page.locator('#step').click();
    await page.waitForFunction(target => Number(currentSnapshot.timeline.boundary) === target && pendingCommandRevision === null, transition);
    if (Number(await populated.getAttribute('data-count')) > 0) break;
  }
  assert.equal(await populated.evaluate(section => section.open), true);
  await populated.locator('summary').first().click();
  const next = Number(await page.locator('#timeline').inputValue()) + 1;
  await page.locator('#step').click();
  await page.waitForFunction(target => Number(currentSnapshot.timeline.boundary) === target && pendingCommandRevision === null, next);
  assert.ok(Number(await populated.getAttribute('data-count')) > 0);
  assert.equal(await populated.evaluate(section => section.open), false, 'manual folding survives a populated refresh');
  await finish(page);
  await seek(page, 0);
  await page.locator('[data-command="resume"]').click();
  assert.equal(await page.locator('.transport .playback-toggle:visible').count(), 1);
  assert.equal(await page.locator('[data-command="pause"]').isVisible(), true);
  await page.locator('[data-command="pause"]').click();
  assert.equal(await page.locator('[data-command="resume"]').isVisible(), true);
  await capture(page, 'audit-compact-controls');
});

test('audit: structured source summaries keep their tree and inspector link keyboard reachable', { timeout: 60000 }, async t => {
  const page = await openTour(t, undefined, 'examples/countdown.rkg');
  // Isolate tooltip navigation with a complete, already filtered read-model summary.
  await page.evaluate(() => {
    const anchor = document.querySelector('#source-code [data-source-token]');
    const token = sourceValues.get(anchor.dataset.sourceToken);
    token.value = { ...token.value, kind: 'DATA path', rendering: '{"receipt":{"public":"accepted"}}', available: true };
    anchor.focus();
  });
  await page.keyboard.press('Tab');
  const branch = page.locator('#source-tooltip summary');
  assert.equal(await branch.evaluate(element => element === document.activeElement), true);
  await page.keyboard.press('Enter');
  assert.equal(await branch.locator('..').evaluate(element => element.open), false);
  await page.keyboard.press('Enter');
  assert.equal(await page.locator('#source-tooltip').getByText('accepted', { exact: true }).isVisible(), true);
  await page.keyboard.press('Tab');
  assert.equal(await page.locator('#source-tooltip a').evaluate(element => element === document.activeElement), true);
  await page.keyboard.press('Shift+Tab');
  assert.equal(await branch.evaluate(element => element === document.activeElement), true);
  await page.keyboard.press('Shift+Tab');
  assert.equal(await page.locator('#source-code [data-source-token]').first().evaluate(element => element === document.activeElement), true);
});
