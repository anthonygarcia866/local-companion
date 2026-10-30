#!/usr/bin/env node
// Fails if a tracked Markdown file has a carriage return in it, or a literal
// "\r\n" written into its text. Both came from scripts that mangled escapes
// (2026-10-29: a PROJECT_STATUS entry split over four lines by a raw CR/LF).
// The repo stores Markdown with LF endings, so any CR in the stored file is a
// stray one. Files are read as git stores them (the index), never from the
// working tree, so a Windows checkout's CRLF conversion can't cause a false
// alarm. Write "CRLF" / "LF" in prose instead of the escapes.
//
// Run: node ci/check-md.mjs   (from anywhere inside the repo)

import { execFileSync } from "node:child_process";

const git = (args) => execFileSync("git", args, { maxBuffer: 64 * 1024 * 1024 });
const root = git(["rev-parse", "--show-toplevel"]).toString().trim();
const files = git(["-C", root, "ls-files", "-z", "--", "*.md"]).toString().split("\0").filter(Boolean);
const LITERAL = "\\r\\n"; // the four characters: backslash r backslash n

let bad = 0;
for (const f of files) {
  const text = git(["-C", root, "show", `:${f}`]).toString("utf8");
  text.split("\n").forEach((line, i) => {
    const problems = [];
    if (line.includes("\r")) problems.push("carriage return");
    if (line.includes(LITERAL)) problems.push('literal "\\r\\n"');
    if (problems.length) {
      bad++;
      const at = Math.max(line.indexOf("\r"), line.indexOf(LITERAL));
      const near = line.slice(Math.max(0, at - 40), at + 20).replace(/\r/g, "<CR>");
      console.log(`${f}:${i + 1}: ${problems.join(" + ")}: …${near}…`);
      if (process.env.GITHUB_ACTIONS) console.log(`::error file=${f},line=${i + 1}::${problems.join(" + ")} in a Markdown file`);
    }
  });
}

if (bad) {
  console.log(`\n${bad} line(s) with a stray carriage return or a literal "\\r\\n". Write CRLF / LF in prose.`);
  process.exit(1);
}
console.log(`ok: ${files.length} Markdown files, no stray carriage return or literal "\\r\\n".`);
