#!/usr/bin/env node
// Generates every client's colours from design/tokens.json, so web and native cannot drift apart.
//
//   node design/gen-tokens.mjs                 write the generated files in the sibling repos
//   node design/gen-tokens.mjs --check         write nothing; exit 1 if a generated file is stale (use in CI)
//   --web <frontend dir>  --desktop <crate src dir>   where the sibling repos are (defaults: ../../drover-web/frontend, ...)
//   --require                                  a missing target directory is an error (CI), not a skip
//
// Written to drover-web/frontend:  src/styles/themes.generated.css, src/core/theme-data.generated.ts
// Written to drover-desktop:       <crate>/src/theme_data.rs

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const opt = (name, fallback) => { const i = args.indexOf(name); return i >= 0 ? resolve(args[i + 1]) : fallback; };

const check = flag("--check");
const require = flag("--require");
const webDir = opt("--web", resolve(here, "../../drover-web/frontend"));
const desktopDir = opt("--desktop", resolve(here, "../../drover-desktop/rust/crates/drover-desktop-native/src"));

const tokens = JSON.parse(readFileSync(join(here, "tokens.json"), "utf8"));

// ---- validation: a typo here would ship to every client ----
const UI_KEYS = ["canvas", "chrome", "pane", "raised", "card", "fg", "tfg", "muted", "dim", "faint", "accent", "working", "blocked",
  "done", "error", "yellow", "head", "add", "rem", "onAccent", "ink"];
const HEX = /^#[0-9a-f]{6}$/;
const problems = [];
const ids = new Set();
for (const t of tokens.themes) {
  if (!/^[a-z][a-z0-9-]*$/.test(t.id)) problems.push(`theme id "${t.id}" must be lower-case letters, digits and "-"`);
  if (ids.has(t.id)) problems.push(`duplicate theme id "${t.id}"`);
  ids.add(t.id);
  if (t.scheme !== "dark" && t.scheme !== "light") problems.push(`${t.id}: scheme must be "dark" or "light"`);
  for (const k of UI_KEYS) if (!HEX.test(t.ui?.[k] ?? "")) problems.push(`${t.id}: ui.${k} must be a #rrggbb colour`);
  if (!Array.isArray(t.term?.ansi) || t.term.ansi.length !== 16 || t.term.ansi.some((c) => !HEX.test(c))) problems.push(`${t.id}: term.ansi needs 16 #rrggbb colours`);
  if (!t.term?.cursor || !HEX.test(t.term.cursor)) problems.push(`${t.id}: term.cursor must be #rrggbb`);
  if (!t.term?.selection) problems.push(`${t.id}: term.selection is missing`);
}
if (tokens.themes[0]?.id !== "dark") problems.push('the first theme must be "dark" (it is the default)');
if (problems.length) { console.error("tokens.json is invalid:\n  " + problems.join("\n  ")); process.exit(2); }

const HEADER = "GENERATED from drover-core/design/tokens.json by design/gen-tokens.mjs. Do not edit: change tokens.json and run the generator.";
const rgbList = (hex) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)).join(" ");
const kebab = (k) => k.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase());
const snake = (k) => k.replace(/[A-Z]/g, (c) => "_" + c.toLowerCase());
const constName = (id) => id.replace(/-/g, "_").toUpperCase();
const rustHex = (hex) => "0x" + hex.slice(1);

// ---- web: CSS custom properties ----
function css() {
  const out = [`/* ${HEADER} */`];
  for (const t of tokens.themes) {
    const sel = t.id === "dark" ? ':root, :root[data-theme="dark"]' : `:root[data-theme="${t.id}"]`;
    const lines = [];
    for (const k of UI_KEYS) {
      if (k === "ink") lines.push(`  --ink: ${rgbList(t.ui.ink)};`);
      else lines.push(`  --${kebab(k)}: ${t.ui[k]};`);
    }
    for (const k of ["accent", "working", "blocked", "done", "error"]) lines.push(`  --${k}-rgb: ${rgbList(t.ui[k])};`);
    // Phone layout backgrounds (mobile.css): the default theme names its own; the others follow the scheme.
    lines.push(`  --m-deep: ${t.mobile?.deep ?? (t.scheme === "light" ? "var(--canvas)" : "var(--chrome)")};`);
    lines.push(`  --m-field: ${t.mobile?.field ?? "var(--raised)"};`);
    lines.push(`  color-scheme: ${t.scheme};`);
    out.push(`${sel} {\n${lines.join("\n")}\n}`);
    if (t.scrim) out.push(`:root[data-theme="${t.id}"] .scrim { background: ${t.scrim}; }`);
  }
  return out.join("\n") + "\n";
}

// ---- web: the theme list, xterm palettes and the Settings preview cards ----
function ts() {
  const q = JSON.stringify;
  const defs = tokens.themes.map((t) => `  { id: ${q(t.id)}, label: ${q(t.label)}, note: ${q(t.note)}, scheme: ${q(t.scheme)} },`).join("\n");
  const names = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];
  const term = tokens.themes.map((t) => {
    const a = t.term.ansi;
    const colors = names.map((n, i) => `${n}: ${q(a[i])}`).join(", ");
    const bright = names.map((n, i) => `bright${n[0].toUpperCase()}${n.slice(1)}: ${q(a[i + 8])}`).join(", ");
    return `  ${q(t.id)}: {\n    background: ${q(t.ui.pane)}, foreground: ${q(t.ui.tfg)}, cursor: ${q(t.term.cursor)}, cursorAccent: ${q(t.ui.pane)},\n` +
      `    selectionBackground: ${q(t.term.selection)},\n    ${colors},\n    ${bright},\n  },`;
  }).join("\n");
  const preview = tokens.themes.map((t) => {
    const alpha = t.ui.ink === "#000000" ? "rgba(0,0,0,.1)" : "rgba(255,255,255,.1)";
    return `  ${q(t.id)}: { cv: ${q(t.ui.canvas)}, ch: ${q(t.ui.chrome)}, cp: ${q(t.ui.pane)}, fg: ${q(t.ui.fg)}, mu: ${q(t.ui.muted)}, ac: ${q(t.ui.accent)}, bd: ${q(alpha)}, ok: ${q(t.ui.add)} },`;
  }).join("\n");
  return `// ${HEADER}\nimport type { ITheme } from "@xterm/xterm";\n\n` +
    `export interface ThemeDef { id: string; label: string; note: string; scheme: "dark" | "light" }\n` +
    `export const THEME_DEFS: ThemeDef[] = [\n${defs}\n];\n\n` +
    `export const TERM_THEMES: Record<string, ITheme> = {\n${term}\n};\n\n` +
    `export interface ThemePreview { cv: string; ch: string; cp: string; fg: string; mu: string; ac: string; bd: string; ok: string }\n` +
    `export const THEME_PREVIEW: Record<string, ThemePreview> = {\n${preview}\n};\n`;
}

// ---- native: Rust palettes ----
function rust() {
  const L = tokens.layout;
  const pal = tokens.themes.map((t) => {
    const f = UI_KEYS.filter((k) => k !== "ink").map((k) => `${snake(k)}: ${rustHex(t.ui[k])}`);
    return `pub const ${constName(t.id)}: Palette = Palette {\n    ${f.join(", ")},\n    ink: ${rustHex(t.ui.ink)},\n};`;
  }).join("\n");
  const defs = tokens.themes.map((t) => {
    const ansi = t.term.ansi.map(rustHex).join(", ");
    return `    ThemeDef { id: ${JSON.stringify(t.id)}, label: ${JSON.stringify(t.label)}, note: ${JSON.stringify(t.note)}, light: ${t.scheme === "light"}, palette: &${constName(t.id)},\n` +
      `        ansi: [${ansi}] },`;
  }).join("\n");
  const layout = Object.entries(L).filter(([, v]) => typeof v === "number").map(([k, v]) => `    pub const ${snake(k).toUpperCase()}: f32 = ${Number.isInteger(v) ? v + ".0" : v};`).join("\n");
  return `// ${HEADER}\n#![allow(dead_code)]\n\nuse crate::theme::Palette;\n\n${pal}\n\n` +
    `pub struct ThemeDef {\n    pub id: &'static str,\n    pub label: &'static str,\n    pub note: &'static str,\n    pub light: bool,\n    pub palette: &'static Palette,\n    /// The 16 terminal colours (black..white, then the bright ones).\n    pub ansi: [u32; 16],\n}\n\n` +
    `pub static ALL: [ThemeDef; ${tokens.themes.length}] = [\n${defs}\n];\n\n` +
    `/// Sizes both clients use (the web UI reads the same numbers from tokens.json).\npub mod layout {\n${layout}\n}\n`;
}

// ---- write or check ----
const targets = [
  [webDir, "src/styles/themes.generated.css", css()],
  [webDir, "src/core/theme-data.generated.ts", ts()],
  [desktopDir, "theme_data.rs", rust()],
];
const norm = (s) => s.replace(/\r\n/g, "\n");
let stale = 0, skipped = 0;
for (const [dir, rel, text] of targets) {
  if (!existsSync(dir)) {
    if (require) { console.error(`missing directory: ${dir}`); stale++; } else { console.log(`skip ${rel}: ${dir} not found`); skipped++; }
    continue;
  }
  const file = join(dir, rel);
  const current = existsSync(file) ? norm(readFileSync(file, "utf8")) : null;
  if (current === text) { console.log(`ok    ${rel}`); continue; }
  if (check) { console.error(`STALE ${rel}: run \`node design/gen-tokens.mjs\` in drover-core and commit the result`); stale++; continue; }
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, text);
  console.log(`wrote ${rel}`);
}
process.exit(stale ? 1 : 0);
