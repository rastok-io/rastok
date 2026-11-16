import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const script = join(root, "tools/package-editor.mjs");
const extension = join(root, "editors/vscode/rastok-dsl");

test("the VSIX contains only the data extension and its license, without overwriting", () => {
  const output = mkdtempSync(join(tmpdir(), "rastok-editor-package-"));
  try {
    const result = spawnSync(process.execPath, [script, output], { encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
    const manifest = JSON.parse(readFileSync(join(extension, "package.json"), "utf8"));
    const name = `rastok-dsl-${manifest.version}.vsix`;
    assert.deepEqual(readdirSync(output), [name]);
    const archive = join(output, name);
    const entries = execFileSync("unzip", ["-Z1", archive], { encoding: "utf8" }).trim().split("\n").sort();
    assert.deepEqual(entries, [
      "[Content_Types].xml", "extension.vsixmanifest", "extension/LICENSE",
      "extension/README.md", "extension/language-configuration.json", "extension/package.json",
      "extension/syntaxes/rkg.tmLanguage.json",
    ].sort());
    for (const file of ["README.md", "language-configuration.json", "package.json", "syntaxes/rkg.tmLanguage.json"]) {
      assert.deepEqual(execFileSync("unzip", ["-p", archive, `extension/${file}`]), readFileSync(join(extension, file)));
    }
    assert.deepEqual(execFileSync("unzip", ["-p", archive, "extension/LICENSE"]), readFileSync(join(root, "LICENSE")));
    const xml = execFileSync("unzip", ["-p", archive, "extension.vsixmanifest"], { encoding: "utf8" });
    assert.ok(xml.includes(`Id="${manifest.name}" Version="${manifest.version}" Publisher="${manifest.publisher}"`));
    assert.ok(xml.includes('Type="Microsoft.VisualStudio.Code.Manifest" Path="extension/package.json"'));
    assert.ok(xml.includes(`Id="Microsoft.VisualStudio.Code.Engine" Value="${manifest.engines.vscode}"`));
    const original = readFileSync(archive);
    const repeated = spawnSync(process.execPath, [script, output], { encoding: "utf8" });
    assert.notEqual(repeated.status, 0);
    assert.match(repeated.stderr, /already exists/);
    assert.deepEqual(readFileSync(archive), original);
  } finally {
    rmSync(output, { recursive: true });
  }
});
