// Regenerates src-tauri/icons from docs/logo.svg, dropping the mobile icons and the PNG tauri.conf.json doesn't list.
// Run from the repo root: node scripts/icons.mjs
import { execSync } from "node:child_process";
import { rmSync } from "node:fs";

execSync("npx tauri icon docs/logo.svg -o src-tauri/icons", { stdio: "inherit" });
for (const extra of ["android", "ios", "64x64.png"]) rmSync(`src-tauri/icons/${extra}`, { recursive: true, force: true });
