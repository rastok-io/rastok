import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Source-level guard for the repository's rustdoc citation convention.
// Only doc-comment destinations are checked, not prose mentions or code links.
export function relativeMarkdownLinks(source) {
  let inBlock = false;
  const docs = source.split("\n").map(line => {
    const trimmed = line.trimStart();
    if (/^\/\/(?:!|\/(?!\/))/.test(trimmed)) return trimmed.slice(3);
    if (/^\/\*(?:!|\*(?!\*))/.test(trimmed)) inBlock = true;
    if (!inBlock) return "";
    const end = trimmed.indexOf("*/");
    if (end !== -1) inBlock = false;
    return (end === -1 ? trimmed : trimmed.slice(0, end))
      .replace(/^\/\*[*!]/, "").replace(/^\s*\* ?/, "");
  }).join("\n");
  const violations = [];
  for (const match of docs.matchAll(/\]\(\s*<?([^\s)>]+)|\]:\s*<?([^\s>]+)|href\s*=\s*["']([^"']+)/g)) {
    const target = match[1] || match[2] || match[3];
    if (/\.md(?:[#?]|$)/i.test(target) && !/^https?:\/\//i.test(target)) {
      const line = docs.slice(0, match.index).split("\n").length;
      violations.push({ line, target });
    }
  }
  return violations;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const paths = execFileSync("git", ["ls-files", "-z", "--", "*.rs"], {
    encoding: "utf8", stdio: ["ignore", "pipe", "pipe"],
  }).split("\0").filter(Boolean);
  let failures = 0;
  for (const path of paths) {
    for (const { line } of relativeMarkdownLinks(readFileSync(path, "utf8"))) {
      console.error(`${path}:${line}: rustdoc Markdown links require an absolute specification URL.`);
      failures += 1;
    }
  }
  console.log(`Rustdoc links: ${paths.length} Rust files checked, ${failures} relative Markdown links.`);
  process.exitCode = failures ? 1 : 0;
}
