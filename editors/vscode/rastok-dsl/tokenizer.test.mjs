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

test('JSON quoting, raw readers and nested comments return to the host grammar', () => {
  const { tokens } = tokenize(`val first = json! { "text": "} Loop.again // not DSL", "nested": [true, {"n": 2}] }
val scalar = json! "} stop"
val bytes = base64!("text/plain") { YQ== }
val raw = text! { on in { Loop.done } }
/* outer /* nested */ still comment */
model.instance.stop
Loop.again
operator after`);
  assert.ok(tokens.some(t => t.text.includes('Loop.again // not DSL') && t.scopes.includes('string.quoted.double.rkg')));
  assert.ok(tokens.some(t => t.text.includes('on in') && t.scopes.includes('string.unquoted.reader.rkg')));
  assert.ok(tokens.some(t => t.text === 'model.instance.stop' && t.scopes.includes('support.function.instance-command.rkg')));
  assert.ok(tokens.some(t => t.text === 'Loop.again' && t.scopes.includes('support.variable.loop.rkg')));
  assert.ok(tokens.some(t => t.text === 'after' && t.scopes.includes('entity.name.type.rkg')));
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
