#!/usr/bin/env node
// Checks design/features.json: every feature states where it stands in BOTH clients.
//   node design/check-parity.mjs            fails (exit 1) on a missing or invalid status, or an n/a without a reason
//   node design/check-parity.mjs --strict   also fails while one client has a feature the other only plans (the gap list)
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const strict = process.argv.includes("--strict");
const { features } = JSON.parse(readFileSync(join(here, "features.json"), "utf8"));

const STATUS = new Set(["done", "planned", "n/a"]);
const errors = [];
const ids = new Set();
for (const f of features) {
  if (ids.has(f.id)) errors.push(`${f.id}: duplicate id`);
  ids.add(f.id);
  for (const client of ["web", "native"]) {
    if (!STATUS.has(f[client])) errors.push(`${f.id}: "${client}" must be done, planned or n/a (got ${JSON.stringify(f[client])})`);
  }
  if ((f.web === "n/a" || f.native === "n/a") && !f.reason) errors.push(`${f.id}: n/a needs a "reason"`);
  if (f.web === "n/a" && f.native === "n/a") errors.push(`${f.id}: n/a in both clients is not a feature`);
}

const gaps = features.filter((f) => (f.web === "done") !== (f.native === "done") && f.web !== "n/a" && f.native !== "n/a");
const pad = (s, n) => String(s).padEnd(n);
console.log(`${pad("feature", 30)}${pad("web", 9)}${pad("native", 9)}`);
for (const f of features) console.log(`${pad(f.id, 30)}${pad(f.web, 9)}${pad(f.native, 9)}${f.reason ? "  (" + f.reason + ")" : ""}`);
console.log(`\n${features.length} features, ${gaps.length} with a gap between the clients`);

if (errors.length) { console.error("\nfeatures.json problems:\n  " + errors.join("\n  ")); process.exit(1); }
if (strict && gaps.length) { console.error("\n--strict: " + gaps.map((g) => g.id).join(", ")); process.exit(1); }
