import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
import test from "node:test";

const script = fileURLToPath(new URL("./check-dco.mjs", import.meta.url));
const signOff = "Signed-off-by: Test Author <author@example.invalid>";

function repository(t) {
  const cwd = mkdtempSync(join(tmpdir(), "rastok-dco-test-"));
  t.after(() => rmSync(cwd, { recursive: true, force: true }));
  const env = {
    PATH: process.env.PATH,
    DEVELOPER_DIR: process.env.DEVELOPER_DIR,
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_CONFIG_GLOBAL: "/dev/null",
    GIT_AUTHOR_NAME: "Test Author",
    GIT_AUTHOR_EMAIL: "author@example.invalid",
    GIT_COMMITTER_NAME: "Test Committer",
    GIT_COMMITTER_EMAIL: "committer@example.invalid",
  };
  const git = (...args) => execFileSync("git", ["-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", ...args], { cwd, env, encoding: "utf8", stdio: ["pipe", "pipe", "pipe"] }).trim();
  git("init", "--initial-branch=main");
  const commit = message => { git("commit", "--allow-empty", "-m", message); return git("rev-parse", "HEAD"); };
  const base = commit("Existing history outside the contribution range");
  const check = (from = base, to = git("rev-parse", "HEAD")) => spawnSync(process.execPath, [script, from, to], { cwd, env, encoding: "utf8" });
  return { git, commit, base, check };
}

test("an author sign-off passes even when the committer differs", t => {
  const repo = repository(t);
  repo.commit(`Implement the change\n\n${signOff}`);
  const result = repo.check();
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /1 commit checked/);
});

test("a missing sign-off fails", t => {
  const repo = repository(t);
  repo.commit("Implement the change");
  assert.equal(repo.check().status, 1);
});

test("another person's sign-off does not certify the author", t => {
  const repo = repository(t);
  repo.commit("Implement the change\n\nSigned-off-by: Test Committer <committer@example.invalid>");
  assert.equal(repo.check().status, 1);
});

test("the real footer may contain multiple credits and sign-offs", t => {
  const repo = repository(t);
  repo.commit(`Implement the change\n\nAssisted-by: Example Test Model 1\nSigned-off-by: Reviewer <reviewer@example.invalid>\n${signOff}`);
  assert.equal(repo.check().status, 0);
});

test("a sign-off quoted in prose is not a footer", t => {
  const repo = repository(t);
  repo.commit(`Discuss sign-offs\n\n${signOff}\n\nThis paragraph deliberately follows the quoted example.`);
  assert.equal(repo.check().status, 1);
});

test("an unsigned earlier commit cannot hide behind a signed tip", t => {
  const repo = repository(t);
  repo.commit("Unsigned change");
  repo.commit(`Signed follow-up\n\n${signOff}`);
  const result = repo.check();
  assert.equal(result.status, 1);
  assert.match(result.stderr, /1 of 2 commits/);
});

test("all merged parents are checked, not only the first-parent line", t => {
  const repo = repository(t);
  repo.git("checkout", "-b", "side");
  repo.commit("Unsigned side change");
  repo.git("checkout", "main");
  repo.commit(`Signed main change\n\n${signOff}`);
  repo.git("merge", "--no-ff", "side", "-m", `Merge side\n\n${signOff}`);
  const result = repo.check();
  assert.equal(result.status, 1);
  assert.match(result.stderr, /1 of 3 commits/);
});

test("a zero base checks the entire newly pushed history", t => {
  const repo = repository(t);
  repo.commit(`Signed tip\n\n${signOff}`);
  const result = repo.check("0".repeat(40));
  assert.equal(result.status, 1);
  assert.match(result.stderr, /1 of 2 commits/);
});

test("a known empty range passes but missing or malformed revisions refuse", t => {
  const repo = repository(t);
  assert.equal(repo.check(repo.base, repo.base).status, 0);
  assert.equal(repo.check("--all").status, 1);
  assert.equal(repo.check("f".repeat(40)).status, 1);
  assert.equal(repo.check(repo.base, "0".repeat(40)).status, 1);
});
