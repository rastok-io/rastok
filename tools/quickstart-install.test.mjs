import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const root = fileURLToPath(new URL("../", import.meta.url));
const quickstart = readFileSync(join(root, "docs/site/getting-started.md"), "utf8")
  .split("<!-- quickstart:start -->")[1].split("<!-- quickstart:end -->")[0];
const commands = [...quickstart.matchAll(/```sh\n([^]*?)\n```/g)]
  .map(match => match[1]).filter(command => command.includes("curl -fLO"));
assert.equal(commands.length, 2);

for (const [index, command] of commands.entries()) {
  test(`installation ${index + 1} verifies its archive and leaves the caller's directory untouched`, () => {
    const fixture = mkdtempSync(join(tmpdir(), "rastok-install-proof-"));
    try {
      for (const dir of ["archive", "tools", "working", "temporary"]) mkdirSync(join(fixture, dir));
      const archive = command.match(/rastok-(\d+\.\d+\.\d+-alpha)-[^ /]+\.tar\.gz/);
      assert.ok(archive);
      const version = `rastok ${archive[1]}`;
      writeFileSync(join(fixture, "archive/rastok"), `#!/bin/sh\nprintf '%s\\n' '${version}'\n`, { mode: 0o755 });
      execFileSync("tar", ["-czf", archive[0], "rastok"], { cwd: join(fixture, "archive") });
      const bytes = readFileSync(join(fixture, "archive", archive[0]));
      const checksum = createHash("sha256").update(bytes).digest("hex");
      const checksumPath = join(fixture, "archive", `${archive[0]}.sha256`);
      writeFileSync(checksumPath, `${checksum}  ${archive[0]}\n`);
      // Substitute only downloading; checksum, extraction and installation use the real tools.
      writeFileSync(join(fixture, "tools/curl"), `#!/bin/sh
test "$#" = 2 && test "$1" = -fLO || exit 2
case "$2" in https://github.com/rastok-io/rastok/releases/download/*) ;; *) exit 2 ;; esac
cp "$RASTOK_INSTALL_FIXTURES/\${2##*/}" .
`, { mode: 0o755 });
      writeFileSync(join(fixture, "working/keep.txt"), "user content\n");
      const installed = join(fixture, "installation");
      // Do not change HOME: redirect the command's explicit installation paths into the fixture.
      const local = command.replaceAll("$HOME", installed);
      const options = {
        cwd: join(fixture, "working"), encoding: "utf8",
        env: { ...process.env, PATH: `${join(fixture, "tools")}:${process.env.PATH}`, TMPDIR: join(fixture, "temporary"), RASTOK_INSTALL_FIXTURES: join(fixture, "archive") },
      };
      const result = spawnSync("sh", ["-c", local], options);
      assert.equal(result.status, 0, result.stderr);
      assert.ok(result.stdout.includes(`${archive[0]}: OK`), result.stdout);
      assert.ok(result.stdout.endsWith(`${version}\n`), result.stdout);
      assert.deepEqual(readdirSync(join(fixture, "working")), ["keep.txt"]);
      assert.equal(readFileSync(join(fixture, "working/keep.txt"), "utf8"), "user content\n");
      const binary = readFileSync(join(installed, ".local/bin/rastok"));
      writeFileSync(checksumPath, `${"0".repeat(64)}  ${archive[0]}\n`);
      const refused = spawnSync("sh", ["-c", local], options);
      assert.notEqual(refused.status, 0);
      assert.ok(!refused.stdout.endsWith(`${version}\n`));
      assert.deepEqual(readFileSync(join(installed, ".local/bin/rastok")), binary);
      assert.deepEqual(readdirSync(join(fixture, "working")), ["keep.txt"]);
    } finally {
      rmSync(fixture, { recursive: true });
    }
  });
}
