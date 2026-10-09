// Renders the app icons Tauri bundles from Glim's brand artwork
// (docs/brand/app-icon.svg), through the Tauri CLI's own `icon` command, and
// keeps only the files tauri.conf.json lists.
//
//   npm run icons

import { execSync } from "node:child_process";
import { copyFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const source = resolve(root, "..", "docs", "brand", "app-icon.svg");
const out = join(root, "src-tauri", "icons");
const KEEP = ["32x32.png", "128x128.png", "128x128@2x.png", "icon.ico", "icon.png"];

const tmp = mkdtempSync(join(tmpdir(), "glim-icons-"));
try {
  execSync(`npx tauri icon "${source}" -o "${tmp}"`, { cwd: root, stdio: "inherit" });
  for (const name of KEEP) copyFileSync(join(tmp, name), join(out, name));
  console.log(`Wrote ${KEEP.join(", ")} to ${out}`);
} finally {
  rmSync(tmp, { recursive: true, force: true });
}
