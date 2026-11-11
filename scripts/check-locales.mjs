#!/usr/bin/env node
// Validates src/locales/*.json against en.json (the reference) and against the keys used in the code.
//   node scripts/check-locales.mjs
import { readFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const dir = join(root, "src", "locales");
const read = (p) => readFileSync(p, "utf8");
const errors = [];
const warnings = [];
const err = (m) => errors.push(m);

const placeholders = (s) => [...s.matchAll(/\{(\d+)\}/g)].map((m) => m[1]).sort().join(",");

let index;
try { index = JSON.parse(read(join(dir, "index.json"))); }
catch (e) { console.error("index.json: " + e.message); process.exit(1); }
if (!Array.isArray(index) || !index.includes("en")) err('index.json must be an array that contains "en"');

const ref = JSON.parse(read(join(dir, "en.json")));
const refKeys = Object.keys(ref).filter((k) => k !== "_meta");

for (const code of index) {
  const file = join(dir, `${code}.json`);
  if (!existsSync(file)) { err(`${code}: file ${code}.json is listed in index.json but missing`); continue; }
  let data;
  try { data = JSON.parse(read(file)); } catch (e) { err(`${code}: invalid JSON (${e.message})`); continue; }
  if (!data._meta || data._meta.code !== code) err(`${code}: _meta.code must be "${code}"`);
  if (!data._meta || !data._meta.name) err(`${code}: _meta.name is missing`);
  for (const k of refKeys) {
    if (typeof data[k] !== "string") err(`${code}: missing key "${k}"`);
    else if (!data[k].trim()) err(`${code}: empty value for "${k}"`);
    else if (placeholders(data[k]) !== placeholders(ref[k]))
      err(`${code}: placeholders of "${k}" differ from en.json (expected {${placeholders(ref[k])}})`);
  }
  for (const k of Object.keys(data)) if (k !== "_meta" && !(k in ref)) err(`${code}: unknown key "${k}"`);
}

// keys used by the code must exist in en.json
const used = new Set();
const html = read(join(root, "src", "index.html"));
for (const m of html.matchAll(/data-i18n(?:-ph|-title)?="([^"]+)"/g)) used.add(m[1]);
const js = read(join(root, "src", "main.js"));
// every quoted literal that looks like an i18n key (covers ternaries, tables, etc.)
const PREFIXES = "app|folder|mask|opt|maxsize|theme|lang|find|repl|btn|rules|rule|results|status|stat|row|dlg|drop|common|help|about|recipes|update";
for (const m of js.matchAll(new RegExp(`"((?:${PREFIXES})\\.[A-Za-z_.]+)"`, "g"))) {
  if (!m[1].endsWith(".json")) used.add(m[1]); // file names are not keys
}
// numbered help texts (start.1 ... , formats.1 ...) are rendered in a loop
for (const k of Object.keys(ref)) if (/^(start|formats)\.\d+$/.test(k)) used.add(k);
// every recipe in recipes.json needs a title and a description
const recipes = JSON.parse(read(join(root, "src", "recipes.json")));
for (const r of recipes) {
  used.add(`recipe.${r.id}.title`);
  used.add(`recipe.${r.id}.desc`);
  for (const f of ["id", "find", "repl", "sample", "result"]) if (typeof r[f] !== "string") err(`recipes.json: "${r.id}" has no string field "${f}"`);
}
const rs = read(join(root, "src-tauri", "src", "engine.rs"));
for (const m of rs.matchAll(/"((?:skip|err)\.[a-z0-9_]+)/g)) used.add(m[1]);

for (const k of used) if (!(k in ref)) err(`en.json: key "${k}" is used in the code but not defined`);
for (const k of refKeys) if (!used.has(k)) warnings.push(`en.json: key "${k}" is not used anywhere`);

for (const w of warnings) console.warn("warning: " + w);
if (errors.length) {
  for (const e of errors) console.error("error: " + e);
  console.error(`\n${errors.length} problem(s) found`);
  process.exit(1);
}
console.log(`OK: ${index.length} locale(s) [${index.join(", ")}], ${refKeys.length} keys`);
