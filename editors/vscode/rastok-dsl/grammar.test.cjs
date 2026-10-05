"use strict";
// Lexical smoke tests only, not a substitute for VS Code's TextMate tokenizer.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const grammar = require("./syntaxes/rkg.tmLanguage.json");

test("the editor bundle registers .rkg against the shared grammar", () => {
  const manifest = require("./package.json");
  const language = manifest.contributes.languages.find(language => language.id === "rkg");
  const registration = manifest.contributes.grammars.find(grammar => grammar.language === language.id);
  assert.deepEqual(language.extensions, [".rkg"]);
  assert.ok(language.aliases.includes("Rastok"));
  assert.equal(registration.scopeName, grammar.scopeName);
  assert.deepEqual(require(registration.path), grammar);
  assert.equal(require(`./${language.configuration}`).comments.lineComment, "//");
});

test("every TextMate regular expression compiles in the shared regex subset", () => {
  function visit(value) {
    if (value && typeof value === "object") {
      for (const [key, child] of Object.entries(value)) {
        if (["match", "begin", "end"].includes(key)) new RegExp(child);
        else visit(child);
      }
    }
  }
  visit(grammar);
});

test("manager commands, annotations and spatial arrows have distinct scopes", () => {
  const matches = (word, scope) => grammar.patterns.some(rule => rule.match && rule.name === scope && new RegExp(rule.match).test(word));
  for (const word of ["Loop.next", "Loop.again", "Loop.done", "Loop.expired", "Loop.state"]) assert.ok(matches(word, "support.variable.loop.rkg"));
  for (const word of ["model.instance.stop", "conversation.model.instance.stop"]) assert.ok(matches(word, "support.function.instance-command.rkg"));
  for (const word of ["@personal", "@secret", "@sensitive", "@measure"]) assert.ok(matches(word, "storage.modifier.annotation.rkg"));
  for (const arrow of ["->", "=>", "<->"]) assert.ok(matches(arrow, "keyword.operator.rkg"));
  assert.ok(matches("has(data.usage)", "support.function.rkg"));
  assert.equal(matches("when", "keyword.control.rkg"), false, "removed wiring guard is not an executable keyword");
  assert.match("operator decide", new RegExp(grammar.repository.declarations.match));
  assert.equal(new RegExp(grammar.repository.declarations.match).exec("connector model")[2], "model");
});

test("v19 annotations, Retry and idempotence reflect the executable language", () => {
  const matches = (word, scope) => grammar.patterns.some(rule => rule.match && rule.name === scope && new RegExp(rule.match).test(word));
  for (const word of ["Retry.attempts", "Retry.delay", "Retry.on"]) assert.ok(matches(word, "support.variable.retry.rkg"), word);
  assert.ok(matches("Retry", "entity.other.inherited-class.rkg"));
  assert.ok(matches("@note", "storage.modifier.annotation.rkg"));
  assert.ok(matches("@struct", "storage.type.rkg"));
  assert.ok(matches("idempotent", "variable.language.rkg"));
  assert.ok(matches("json_profile", "variable.language.rkg"));
  assert.equal(matches("effect", "variable.language.rkg"), false);
  assert.equal(matches("Retry.again", "support.variable.retry.rkg"), false);
  assert.equal(matches("@struct", "storage.modifier.annotation.rkg"), false);
});

test("the source type vocabulary, lifecycle and unsigned literals are covered", () => {
  const matches = (word, scope) => grammar.patterns.some(rule => rule.match && rule.name === scope && new RegExp(rule.match).test(word));
  for (const word of ["long", "double", "number", "date", "time", "uuid", "timestamp_ntz", "table"]) {
    assert.ok(matches(word, "storage.type.rkg"), word);
  }
  for (const word of ["list", "tuple"]) assert.equal(matches(word, "storage.type.rkg"), false, `${word} is not a built-in source type`);
  assert.ok(matches("start", "keyword.control.rkg"));
  assert.ok(matches("now()", "support.function.rkg"));
  assert.ok(matches("42u", "constant.numeric.rkg"));
});

test("comments, strings and readers precede keyword matching", () => {
  assert.deepEqual(grammar.patterns.slice(0, 3).map(rule => rule.include), ["#comments", "#strings", "#readers"]);
  assert.match("// Loop.again is prose", new RegExp(grammar.repository.comments.patterns[0].begin));
  assert.match('base64! { YQ== }', new RegExp(grammar.repository['raw-reader'].begin));
  assert.match('base64!("text/plain") { YQ== }', new RegExp(grammar.repository['raw-reader'].begin));
  assert.match('\\"', new RegExp(grammar.repository.strings.patterns[0].match));
});
