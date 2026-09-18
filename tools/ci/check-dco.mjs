import { execFileSync } from "node:child_process";

// Check real contribution commits, not GitHub's synthetic PR merge commit.
// No shell evaluates commit messages, author identities or revision arguments.
try {
  const [base, head, ...extra] = process.argv.slice(2);
  const objectId = /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/;
  if (extra.length || !objectId.test(base || "") || !objectId.test(head || "") || /^0+$/.test(head)) {
    throw new Error("usage: node tools/ci/check-dco.mjs <base-sha-or-zero> <head-sha>");
  }
  const git = (...args) => execFileSync("git", args, { encoding: "utf8", stdio: ["pipe", "pipe", "pipe"] });
  git("rev-parse", "--verify", `${head}^{commit}`);
  if (!/^0+$/.test(base)) git("rev-parse", "--verify", `${base}^{commit}`);
  const range = /^0+$/.test(base) ? head : `${base}..${head}`;
  const commits = git("rev-list", "--reverse", range, "--").trim().split("\n").filter(Boolean);
  let failures = 0;
  for (const commit of commits) {
    const [name, email, message] = git("show", "-s", "--format=%an%x00%ae%x00%B", commit).split("\0");
    const trailers = execFileSync("git", ["interpret-trailers", "--parse"], {
      input: message, encoding: "utf8", stdio: ["pipe", "pipe", "pipe"],
    });
    const expected = `${name} <${email}>`;
    const signed = trailers.split("\n").some(line => {
      const match = /^Signed-off-by:\s*(.+)$/i.exec(line);
      return match?.[1].trim() === expected;
    });
    if (!signed) {
      // Keep untrusted message text out of workflow-command output.
      console.error(`${commit.slice(0, 12)}: missing author-matching Signed-off-by trailer; use git commit -s.`);
      failures += 1;
    }
  }
  if (failures) {
    console.error(`DCO failed: ${failures} of ${commits.length} commits lack the author's sign-off.`);
    process.exitCode = 1;
  } else {
    console.log(`DCO passed: ${commits.length} ${commits.length === 1 ? "commit" : "commits"} checked.`);
  }
} catch (error) {
  console.error(`DCO check refused: ${error.message}`);
  process.exitCode = 1;
}
