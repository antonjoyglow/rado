// Release helper. Run from the repo root on a release/x.y branch:
//   npm run release -- 0.8.0            bump the version in every manifest and write the 0.8.0 changelog section
//   npm run release -- 0.8.0 --dry-run  print the section, change nothing
//   node scripts/release.mjs notes 0.8.0  print that section's body (release.yml uses it); exit 1 if missing
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const TITLE = /^(feat|fix)(?:\(([^)]+)\))?!?:\s*(.+)$/;

/** "feat(list): header tabs, release 0.7.0 (#8)" -> { type: "feat", text: "list: header tabs (#8)" }; null for other types. */
export function parseTitle(subject) {
  const m = TITLE.exec(subject.trim());
  if (!m) return null;
  const text = m[3].replace(/,\s*release \d+\.\d+\.\d+/i, "").trim();
  return { type: m[1], text: m[2] ? `${m[2]}: ${text}` : text };
}

/** Builds the "## [version] - date" section from commit subjects, newest first. */
export function buildSection(version, date, subjects) {
  const groups = { feat: [], fix: [] };
  for (const s of subjects) {
    const p = parseTitle(s);
    if (p) groups[p.type].push(`- ${p.text}`);
  }
  let out = `## [${version}] - ${date}\n`;
  if (groups.feat.length) out += `\n### Features\n\n${groups.feat.join("\n")}\n`;
  if (groups.fix.length) out += `\n### Fixes\n\n${groups.fix.join("\n")}\n`;
  return out;
}

const heading = (version) => new RegExp(`^## \\[${version.replace(/\./g, "\\.")}\\]`, "m");

/** Inserts the section above the newest one, or replaces that version's section if it is already there. */
export function upsertSection(changelog, version, section) {
  const text = changelog.replace(/\r\n/g, "\n");
  const block = section.trimEnd() + "\n\n";
  const at = text.search(heading(version));
  if (at >= 0) {
    const next = text.slice(at + 1).search(/^## \[/m);
    const end = next >= 0 ? at + 1 + next : text.length;
    return text.slice(0, at) + block + text.slice(end).replace(/^\n+/, "");
  }
  const first = text.search(/^## \[/m);
  const at2 = first >= 0 ? first : text.length;
  return text.slice(0, at2) + block + text.slice(at2);
}

/** The body of a version's section (without its heading), or null when the section is missing or empty. */
export function sectionBody(changelog, version) {
  const text = changelog.replace(/\r\n/g, "\n");
  const at = text.search(heading(version));
  const nl = at < 0 ? -1 : text.indexOf("\n", at);
  if (nl < 0) return null;
  const rest = text.slice(nl + 1);
  const next = rest.search(/^## \[/m);
  const body = (next >= 0 ? rest.slice(0, next) : rest).trim();
  return body || null;
}

/** Sets the rado version in each manifest's text. Throws if a manifest has no version where expected. */
export function bumpVersions(files, version) {
  const swap = (name, text, re, to) => {
    if (!re.test(text)) throw new Error(`No version found in ${name}`);
    return text.replace(re, to);
  };
  const json = (text, edit) => {
    const eol = text.includes("\r\n") ? "\r\n" : "\n";
    const data = JSON.parse(text);
    edit(data);
    return JSON.stringify(data, null, 2).replace(/\n/g, eol) + eol;
  };
  return {
    "package.json": json(files["package.json"], (d) => { d.version = version; }),
    "package-lock.json": json(files["package-lock.json"], (d) => { d.version = version; d.packages[""].version = version; }),
    "src-tauri/tauri.conf.json": swap("tauri.conf.json", files["src-tauri/tauri.conf.json"], /("version":\s*")[^"]+(")/, `$1${version}$2`),
    "src-tauri/Cargo.toml": swap("Cargo.toml", files["src-tauri/Cargo.toml"], /^(version = ")[^"]+(")/m, `$1${version}$2`),
    "src-tauri/Cargo.lock": swap("Cargo.lock", files["src-tauri/Cargo.lock"], /(name = "rado"\r?\nversion = ")[^"]+(")/, `$1${version}$2`),
  };
}

const git = (...args) => execFileSync("git", args, { encoding: "utf8" }).trim();

function main(args) {
  const [first, second] = args;
  if (first === "notes") {
    const body = sectionBody(readFileSync("CHANGELOG.md", "utf8"), second ?? "");
    if (!body) {
      console.error(`CHANGELOG.md has no non-empty "## [${second}]" section.`);
      process.exit(1);
    }
    process.stdout.write(body + "\n");
    return;
  }
  const version = first;
  if (!/^\d+\.\d+\.\d+$/.test(version ?? "")) {
    console.error("Usage: npm run release -- <x.y.z> [--dry-run]   |   node scripts/release.mjs notes <x.y.z>");
    process.exit(1);
  }
  const dryRun = args.includes("--dry-run");
  const since = git("describe", "--tags", "--abbrev=0", "--match", "v*");
  const subjects = git("log", "--no-merges", "--format=%s", `${since}..HEAD`).split("\n").filter(Boolean);
  const d = new Date();
  const date = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  const section = buildSection(version, date, subjects);
  if (!sectionBody(section, version)) console.warn(`Warning: no feat or fix commits since ${since}; the section is empty.`);
  if (dryRun) {
    console.log(`Commits since ${since}: ${subjects.length}\n\n${section}`);
    return;
  }
  const names = ["package.json", "package-lock.json", "src-tauri/tauri.conf.json", "src-tauri/Cargo.toml", "src-tauri/Cargo.lock"];
  const bumped = bumpVersions(Object.fromEntries(names.map((n) => [n, readFileSync(n, "utf8")])), version);
  bumped["CHANGELOG.md"] = upsertSection(readFileSync("CHANGELOG.md", "utf8"), version, section);
  for (const [n, text] of Object.entries(bumped)) writeFileSync(n, text);
  console.log(`Version set to ${version}; CHANGELOG.md has a ${version} section from ${subjects.length} commits since ${since}. Edit it, then commit.`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main(process.argv.slice(2));
