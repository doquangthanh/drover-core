#!/usr/bin/env node
// Generates every client's colours from design/tokens.json, so web and native cannot drift apart.
//
//   node design/gen-tokens.mjs                 write the generated files in the sibling repos
//   node design/gen-tokens.mjs --check         write nothing; exit 1 if a generated file is stale (use in CI)
//   --web <frontend dir>  --desktop <crate src dir>   where the sibling repos are (defaults: ../../drover-web/frontend, ...)
//   --require                                  a missing target directory is an error (CI), not a skip
//   --only web|desktop                         handle one client only (each repo's CI checks its own files)
//
// Written to drover-web/frontend:  src/styles/themes.generated.css, src/core/theme-data.generated.ts
// Written to drover-desktop:       <crate>/src/theme_data.rs

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const usage = (msg) => { console.error(`${msg}\nsee the header of design/gen-tokens.mjs for the options`); process.exit(2); };
const opt = (name) => {
  const i = args.indexOf(name);
  if (i < 0) return undefined;
  const v = args[i + 1];
  if (!v || v.startsWith("--")) usage(`${name} needs a value`);
  return v;
};

const check = flag("--check");
const require = flag("--require");
const only = opt("--only");
if (only && only !== "web" && only !== "desktop") usage(`--only must be web or desktop (got "${only}")`);
const webDir = resolve(opt("--web") ?? resolve(here, "../../drover-web/frontend"));
const desktopDir = resolve(opt("--desktop") ?? resolve(here, "../../drover-desktop/rust/crates/drover-desktop-native/src"));

const tokens = JSON.parse(readFileSync(join(here, "tokens.json"), "utf8"));

// ---- validation: a typo here would ship to every client ----
const UI_KEYS = ["canvas", "chrome", "pane", "raised", "card", "fg", "tfg", "muted", "dim", "faint", "accent", "working", "blocked",
  "done", "error", "yellow", "head", "add", "rem", "onAccent", "ink"];
const HEX = /^#[0-9a-f]{6}$/;
// rgba(r,g,b,a) with 0-255 channels and a 0-1 alpha, e.g. rgba(150,178,215,.32)
const RGBA = /^rgba\((\d{1,3}),(\d{1,3}),(\d{1,3}),(0|1|0?\.\d+)\)$/;
const isRgba = (s) => { const m = RGBA.exec(s ?? ""); return !!m && m.slice(1, 4).every((c) => Number(c) <= 255); };
const isColour = (s) => HEX.test(s ?? "") || isRgba(s);
const problems = [];
const ids = new Set();
if (!Array.isArray(tokens.themes) || !tokens.themes.length) usage("tokens.json is invalid: themes must be a non-empty list");
for (const t of tokens.themes) {
  if (!/^[a-z][a-z0-9-]*$/.test(t.id)) problems.push(`theme id "${t.id}" must be lower-case letters, digits and "-"`);
  if (ids.has(t.id)) problems.push(`duplicate theme id "${t.id}"`);
  ids.add(t.id);
  if (t.scheme !== "dark" && t.scheme !== "light") problems.push(`${t.id}: scheme must be "dark" or "light"`);
  for (const k of UI_KEYS) if (!HEX.test(t.ui?.[k] ?? "")) problems.push(`${t.id}: ui.${k} must be a #rrggbb colour`);
  if (!Array.isArray(t.term?.ansi) || t.term.ansi.length !== 16 || t.term.ansi.some((c) => !HEX.test(c))) problems.push(`${t.id}: term.ansi needs 16 #rrggbb colours`);
  if (!t.term?.cursor || !HEX.test(t.term.cursor)) problems.push(`${t.id}: term.cursor must be #rrggbb`);
  if (!isColour(t.term?.selection)) problems.push(`${t.id}: term.selection must be #rrggbb or rgba(r,g,b,a)`);
  if (t.scrim !== undefined && !isColour(t.scrim)) problems.push(`${t.id}: scrim must be #rrggbb or rgba(r,g,b,a)`);
  for (const k of ["deep", "field"]) if (t.mobile?.[k] !== undefined && !HEX.test(t.mobile[k])) problems.push(`${t.id}: mobile.${k} must be #rrggbb`);
  if (t.ui && t.ui.ink !== "#000000" && t.ui.ink !== "#ffffff") problems.push(`${t.id}: ui.ink must be #000000 or #ffffff`);
}
for (const [k, v] of Object.entries(tokens.layout ?? {})) {
  if (!/^[a-z][a-zA-Z0-9]*$/.test(k)) problems.push(`layout.${k}: keys must be camelCase letters and digits`);
  if (typeof v === "number" ? !Number.isFinite(v) || Math.abs(v) >= 1e6 : typeof v !== "string") problems.push(`layout.${k}: must be a string or a number below 1e6`);
}
if (tokens.themes[0]?.id !== "dark") problems.push('the first theme must be "dark" (it is the default)');
if (problems.length) { console.error("tokens.json is invalid:\n  " + problems.join("\n  ")); process.exit(2); }

const HEADER = "GENERATED from drover-core/design/tokens.json by design/gen-tokens.mjs. Do not edit: change tokens.json and run the generator.";
const rgbList = (hex) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)).join(" ");
const kebab = (k) => k.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase());
const snake = (k) => k.replace(/[A-Z]/g, (c) => "_" + c.toLowerCase());
const constName = (id) => id.replace(/-/g, "_").toUpperCase();
const rustHex = (hex) => "0x" + hex.slice(1);
// A Rust string literal: JSON escapes like \b, \f or \u0001 are not valid Rust, so escape by hand.
const rustStr = (s) => '"' + String(s).replace(/[\\"]/g, (c) => "\\" + c)
  .replace(/[\u0000-\u001f\u007f]/g, (c) => `\\u{${c.charCodeAt(0).toString(16)}}`) + '"';
// A colour as (0xRRGGBB, alpha) for native, which paints with an rgb value plus an alpha.
const rustRgba = (c) => {
  const m = RGBA.exec(c);
  if (!m) return [rustHex(c), "1.0"];
  const hex = "0x" + m.slice(1, 4).map((n) => Number(n).toString(16).padStart(2, "0")).join("");
  const a = Number(m[4]);
  return [hex, Number.isInteger(a) ? a + ".0" : String(a)];
};
const rustF32 = (v) => (Number.isInteger(v) ? v + ".0" : String(v));

// ---- web: CSS custom properties ----
function css() {
  const out = [`/* ${HEADER} */`];
  // Sizes both clients use: sidebarWidth -> --sidebar-width, in px (strings are skipped: the font is not a size).
  const sizes = Object.entries(tokens.layout ?? {}).filter(([, v]) => typeof v === "number").map(([k, v]) => `  --${kebab(k)}: ${v}px;`);
  if (sizes.length) out.push(`:root {\n${sizes.join("\n")}\n}`);
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
    const [sel, selA] = rustRgba(t.term.selection);
    return `    ThemeDef { id: ${rustStr(t.id)}, label: ${rustStr(t.label)}, note: ${rustStr(t.note)}, light: ${t.scheme === "light"}, palette: &${constName(t.id)},\n` +
      `        cursor: ${rustHex(t.term.cursor)}, selection: ${sel}, selection_alpha: ${selA},\n` +
      `        ansi: [${ansi}] },`;
  }).join("\n");
  const layout = Object.entries(L).map(([k, v]) => typeof v === "number"
    ? `    pub const ${snake(k).toUpperCase()}: f32 = ${rustF32(v)};`
    : `    pub const ${snake(k).toUpperCase()}: &str = ${rustStr(v)};`).join("\n");
  return `// ${HEADER}\n#![allow(dead_code)]\n\nuse crate::theme::Palette;\n\n${pal}\n\n` +
    `pub struct ThemeDef {\n    pub id: &'static str,\n    pub label: &'static str,\n    pub note: &'static str,\n    pub light: bool,\n    pub palette: &'static Palette,\n` +
    `    /// Terminal cursor colour.\n    pub cursor: u32,\n    /// Terminal selection highlight: an rgb colour painted at \`selection_alpha\`.\n    pub selection: u32,\n    pub selection_alpha: f32,\n` +
    `    /// The 16 terminal colours (black..white, then the bright ones).\n    pub ansi: [u32; 16],\n}\n\n` +
    `pub static ALL: [ThemeDef; ${tokens.themes.length}] = [\n${defs}\n];\n\n` +
    `/// Sizes both clients use (the web UI reads the same numbers from tokens.json).\npub mod layout {\n${layout}\n}\n`;
}

// ---- write or check ----
const targets = [
  ["web", webDir, "src/styles/themes.generated.css", css()],
  ["web", webDir, "src/core/theme-data.generated.ts", ts()],
  ["desktop", desktopDir, "theme_data.rs", rust()],
].filter(([client]) => !only || client === only).map(([, ...rest]) => rest);
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
