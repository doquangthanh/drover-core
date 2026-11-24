# Design: one look for both clients

Drover has two UIs: the **web UI** (`drover-web`, React; it is also what the Tauri window shows) and the **native window**
(`drover-desktop`, GPUI). They cannot share widget code, so they share **data** and are **checked** against it.

| What | Where it lives | How the clients get it |
|------|----------------|------------------------|
| Colours of every theme, terminal colours, sizes (`layout`) | `design/tokens.json` | generated: `node design/gen-tokens.mjs` |
| What each client can do | `design/features.json` | read by `node design/check-parity.mjs` |

## Change a colour or add a theme

1. Edit `design/tokens.json` (a theme needs every key of `ui`, a `term.ansi` list of 16 colours, a `cursor` and a `selection`).
2. `node design/gen-tokens.mjs` writes, in the sibling repos:
   - `drover-web/frontend/src/styles/themes.generated.css` and `src/core/theme-data.generated.ts`
   - `drover-desktop/rust/crates/drover-desktop-native/src/theme_data.rs`
3. Web only: give the theme a code-editor theme in `frontend/src/files/editor-modes.ts` (`THEME_FILE`) and, if Ace has none,
   a palette in the Ace theme generator. `tokens-parity.test.ts` fails until this is done.
4. Commit the three generated files with the change to `tokens.json`.

Never edit a `*.generated.*` / `theme_data.rs` file by hand. `node design/gen-tokens.mjs --check` fails when one is stale
(run it in CI with `--require` so a missing sibling checkout is an error, not a skip).

## Build a new UI feature

The rule: **a feature is not finished until `design/features.json` says where it stands in both clients.**

1. Add a line to `features.json` in the same change. Each client is `done`, `planned` or `n/a`; `n/a` needs a `reason`.
2. Put everything that is not drawing in a place both can use. In order of preference: the daemon (RPC), the
   protocol crate, the design tokens. Only the drawing is written twice.
3. Take sizes and colours from `tokens.json` (`layout` for sizes). Do not copy a number from one client into the other.
4. `node design/check-parity.mjs` must pass. `--strict` prints the gap list: what one client still lacks.
5. In the PR description say which client the change is for and link the line in `features.json` for the other.

## Checks that run

| Check | Command | Fails when |
|-------|---------|------------|
| Generated colours are current | `node design/gen-tokens.mjs --check --require` | a generated file differs from `tokens.json` |
| Web test | `npm test` in `drover-web/frontend` (`tokens-parity.test.ts`) | generated file stale, or a theme has no terminal or editor palette |
| Native test | `cargo test -p drover-desktop-native` (`theme.rs`) | a generated theme cannot be selected, ids repeat |
| Feature table | `node design/check-parity.mjs` | a feature has no status for one client |
