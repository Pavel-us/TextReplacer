#!/usr/bin/env node
// Puts the PUBLIC updater key into src-tauri/tauri.conf.json.
//   npx tauri signer generate -w ~/.tauri/textreplacer.key
//   node scripts/set-updater-key.mjs ~/.tauri/textreplacer.key.pub
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const file = process.argv[2];
if (!file) {
  console.error("Usage: node scripts/set-updater-key.mjs <path to the .pub file>");
  process.exit(1);
}
const key = readFileSync(file, "utf8").trim();
if (!key || key.includes("PRIVATE")) {
  console.error("This does not look like a public key file (use the .pub file).");
  process.exit(1);
}
const confPath = join(dirname(fileURLToPath(import.meta.url)), "..", "src-tauri", "tauri.conf.json");
const conf = JSON.parse(readFileSync(confPath, "utf8"));
conf.plugins.updater.pubkey = key;
writeFileSync(confPath, JSON.stringify(conf, null, 2) + "\n");
console.log("OK: public key saved to src-tauri/tauri.conf.json");
