// Optional real TextMate/Oniguruma qualification. Reuse the site's installed
// highlighter through explicit paths; the compiler gains no Node dependency.
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { test } from 'node:test';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

for (const name of ['RASTOK_TEXTMATE', 'RASTOK_ONIGURUMA']) {
  assert.ok(process.env[name], `${name} must name an installed module; this qualification does not silently skip`);
}
const { Registry, INITIAL } = await import(pathToFileURL(resolve(process.env.RASTOK_TEXTMATE)));
const { createOnigurumaEngine } = await import(pathToFileURL(resolve(process.env.RASTOK_ONIGURUMA, 'index.mjs')));
const { default: wasm } = await import(pathToFileURL(resolve(process.env.RASTOK_ONIGURUMA, 'wasm-inlined.mjs')));
const source = JSON.parse(await readFile(new URL('./syntaxes/rkg.tmLanguage.json', import.meta.url)));
const engine = await createOnigurumaEngine(wasm);
const registry = new Registry({ onigLib: { createOnigScanner: engine.createScanner, createOnigString: engine.createString }, loadGrammar: name => name === source.scopeName ? source : null });
const grammar = registry.loadGrammar(source.scopeName);
assert.ok(grammar);

function tokenize(text) {
  let stack = INITIAL;
  const tokens = [];
  for (const line of text.split('\n')) {
    const result = grammar.tokenizeLine(line, stack);
    stack = result.ruleStack;
    for (const token of result.tokens) tokens.push({ text: line.slice(token.startIndex, token.endIndex), scopes: token.scopes });
  }
  return { tokens, stack };
}

test('JSON quoting, raw readers and block comments return to the host grammar', () => {
  const { tokens } = tokenize(`val first = json! { "text": "} Loop.again // not DSL", "nested": [true, {"n": 2}] }
val scalar = json! "} stop"
val bytes = base64!("text/plain") { YQ== }
val raw = text! { on in { Loop.done } }
/* comment */
model.instance.stop
Loop.again
operator after`);
  assert.ok(tokens.some(t => t.text.includes('Loop.again // not DSL') && t.scopes.includes('string.quoted.double.rkg')));
  assert.ok(tokens.some(t => t.text.includes('on in') && t.scopes.includes('string.unquoted.reader.rkg')));
  assert.ok(tokens.some(t => t.text === 'model.instance.stop' && t.scopes.includes('support.function.instance-command.rkg')));
  assert.ok(tokens.some(t => t.text === 'Loop.again' && t.scopes.includes('support.variable.loop.rkg')));
  assert.ok(tokens.some(t => t.text === 'after' && t.scopes.includes('entity.name.type.rkg')));
});

test('block comments end at the first closer, even after a nested opener or //', () => {
  for (const comment of ['/* a // not a line comment */', '/* a /* not a nested comment */']) {
    const { tokens } = tokenize(`${comment} operator after\noperator next`);
    for (const name of ['after', 'next']) {
      assert.ok(tokens.some(t => t.text === name && t.scopes.includes('entity.name.type.rkg')), `${comment}: ${name} is outside the comment`);
    }
  }
});

test('expression single quotes protect DSL-looking content and ordinary strings stop at EOL', () => {
  const { tokens } = tokenize("val text = 'Loop.again // not a comment'\noperator after");
  assert.ok(tokens.some(t => t.text.includes('Loop.again // not a comment') && t.scopes.includes('string.quoted.single.rkg')));
  assert.ok(tokens.some(t => t.text === 'after' && t.scopes.includes('entity.name.type.rkg')));
  for (const quote of ['"', "'"]) {
    for (const suffix of ['', '\\']) {
      const { tokens: recovered } = tokenize(`val broken = ${quote}unterminated${suffix}\noperator recovered`);
      assert.ok(recovered.some(t => t.text === 'recovered' && t.scopes.includes('entity.name.type.rkg')), `${quote}${suffix}: the next line is not a string`);
    }
  }
});

test('source types, constructor calls, logical time and uint literals have the right scopes', () => {
  for (const type of ['long', 'double', 'number', 'date', 'time', 'uuid', 'timestamp_ntz', 'table', 'duration', 'decimal']) {
    const parameters = type === 'table' ? '<struct { n: int }>' : type === 'decimal' ? '(18,6)' : '';
    const { tokens } = tokenize(`input request: ${type}${parameters}`);
    assert.ok(tokens.some(t => t.text === type && t.scopes.includes('storage.type.rkg')), `${type}: missing type scope`);
  }
  for (const [name, args] of [['now', ''], ['duration', '"1s"'], ['timestamp', '"2026-09-29T00:00:00Z"'], ['media_type', '"text/plain"'], ['decimal', '"1.25", 18, 2']]) {
    const { tokens } = tokenize(`val result = ${name}(${args})`);
    assert.ok(tokens.some(t => t.text === name && t.scopes.includes('support.function.rkg')), `${name}: a constructor call is not a type`);
  }
  const { tokens } = tokenize('on start { out = 42u }');
  assert.ok(tokens.some(t => t.text === 'start' && t.scopes.includes('keyword.control.rkg')));
  assert.ok(tokens.some(t => t.text === '42u' && t.scopes.includes('constant.numeric.rkg')));
});

test('every shipped example finishes outside a reader, string or block comment', async () => {
  const examples = new URL('../../../examples/', import.meta.url);
  const names = (await readdir(examples)).filter(name => name.endsWith('.rkg')).sort();
  assert.ok(names.includes('agent-gateway.rkg'));
  for (const name of names) {
    const text = await readFile(new URL(name, examples), 'utf8');
    const { tokens } = tokenize(`${text}\noperator tokenizer_sentinel`);
    assert.ok(tokens.some(t => t.text === 'tokenizer_sentinel' && t.scopes.includes('entity.name.type.rkg')), `${name}: lexical state leaked beyond the source`);
  }
});

test('flat annotation declarations and Retry use distinct executable scopes', () => {
  const { tokens } = tokenize(`type note = @struct { text: text }
type Reading = struct { quantity: decimal(18,6) @measure(id = amount, unit = "USD") @note(text = "Reported") }
connector request : HttpClient + Retry {
  idempotent = true
  json_profile = projected
  Retry.on = [ServerError]
  Retry.attempts = 2
  Retry.delay = { kind = fixed ; after = duration("1s") }
}`);
  for (const [text, scope] of [
    ['@struct', 'storage.type.rkg'],
    ['@measure', 'storage.modifier.annotation.rkg'],
    ['@note', 'storage.modifier.annotation.rkg'],
    ['Retry', 'entity.other.inherited-class.rkg'],
    ['Retry.on', 'support.variable.retry.rkg'],
    ['Retry.attempts', 'support.variable.retry.rkg'],
    ['Retry.delay', 'support.variable.retry.rkg'],
    ['idempotent', 'variable.language.rkg'],
    ['json_profile', 'variable.language.rkg'],
  ]) assert.ok(tokens.some(t => t.text === text && t.scopes.includes(scope)), `${text}: missing ${scope}`);
});
