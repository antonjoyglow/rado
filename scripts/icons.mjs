// Regenerates src-tauri/icons from the two SVG sources.
//   docs/logo.svg          the line-drawn mark: every icon from 32 px up
//   src-tauri/icons/tray.svg   the filled tile: the tray icon and icon.ico's 16 and 24 px frames,
//                              where the mark's thin lines fall below a pixel
// Run: node scripts/icons.mjs
import { execSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync, copyFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const icons = "src-tauri/icons";
const tauri = (...args) => execSync(`npx tauri icon ${args.map((a) => `"${a}"`).join(" ")}`, { stdio: "inherit" });

tauri("docs/logo.svg", "-o", icons);
for (const dir of ["android", "ios"]) rmSync(join(icons, dir), { recursive: true, force: true });

const tmp = mkdtempSync(join(tmpdir(), "rado-icons-"));
tauri(join(icons, "tray.svg"), "-o", tmp, "-p", "16,24,32");
copyFileSync(join(tmp, "32x32.png"), join(icons, "tray.png"));

// icon.ico is a directory of PNG frames; swap the 16 and 24 px ones for the tile.
const ico = readFileSync(join(icons, "icon.ico"));
const count = ico.readUInt16LE(4);
const frames = [];
for (let i = 0; i < count; i++) {
  const entry = ico.subarray(6 + i * 16, 6 + (i + 1) * 16);
  const size = entry[0] || 256;
  const data = [16, 24].includes(size)
    ? readFileSync(join(tmp, `${size}x${size}.png`))
    : ico.subarray(entry.readUInt32LE(12), entry.readUInt32LE(12) + entry.readUInt32LE(8));
  frames.push({ entry: Buffer.from(entry), data });
}
let offset = 6 + count * 16;
const parts = [ico.subarray(0, 6)];
for (const f of frames) {
  f.entry.writeUInt32LE(f.data.length, 8);
  f.entry.writeUInt32LE(offset, 12);
  offset += f.data.length;
  parts.push(f.entry);
}
writeFileSync(join(icons, "icon.ico"), Buffer.concat([...parts, ...frames.map((f) => f.data)]));
rmSync(tmp, { recursive: true, force: true });
console.log(`icon.ico: ${frames.map((f) => f.entry[0] || 256).join(", ")} px; 16 and 24 from tray.svg`);
