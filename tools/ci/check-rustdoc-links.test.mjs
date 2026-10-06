import assert from "node:assert/strict";
import test from "node:test";
import { relativeMarkdownLinks } from "./check-rustdoc-links.mjs";

test("inline, reference and HTML relative Markdown links refuse", () => {
  for (const source of [
    "//! See [identity](../spec/identity.md#canonical).",
    "/// [identity]: docs/spec/identity.md",
    '/// <a href="/docs/spec/identity.md">identity</a>',
    "/// [spec](<./types.md>)",
    "/// [spec](\n/// ../types.md#value)",
  ]) assert.equal(relativeMarkdownLinks(source).length, 1, source);
});

test("both block doc forms preserve diagnostic lines", () => {
  assert.deepEqual(relativeMarkdownLinks("fn main() {}\n/**\n * [spec](types.md)\n */"), [
    { line: 3, target: "types.md" },
  ]);
  assert.equal(relativeMarkdownLinks("/*! [spec]: ../types.md */").length, 1);
});

test("absolute reference links and readable short citations pass", () => {
  assert.deepEqual(relativeMarkdownLinks([
    "//! Governing rule: [identity.md §3].",
    "//! [identity.md §3]: https://github.com/rastok-io/rastok/blob/main/docs/spec/identity.md#3",
    "/// See `identity.md` §3 and [`crate::canonical`].",
  ].join("\n")), []);
});

test("ordinary comments, code and non-Markdown destinations are not citations", () => {
  assert.deepEqual(relativeMarkdownLinks([
    '// [file](example.md)',
    'const TEXT: &str = "[file](example.md)";',
    '/** ordinary documentation */',
    'let path = "[file](example.md)";',
    '//// [file](example.md)',
    '/*** [file](example.md) */',
    '/// [type](crate::canonical)',
  ].join("\n")), []);
});
