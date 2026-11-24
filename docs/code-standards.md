# Code standards

General: YAGNI/KISS/DRY, kebab-case file names, files under ~200 lines, real code (no mocks), conventional commits.

## Python (`src/drover`)
- aiohttp daemon; RPC methods register in `rpc/`; every data channel needs the daemon token.
- Tests: `.venv/Scripts/python.exe -m pytest -q`. `tests/daemon_harness.py` boots the real app.

## Rust (`rust/`)
- Cargo workspace: `drover-core` (panes, detection), `drover-daemon` (server, RPC in `methods/`, chat, remote),
  `drover-cli` (bin `dr` = release `drover-server`), `drover-proto` (shared client types), `drover-tui`. Edition 2024.
- The wire protocol is the contract: keep `/ws`, `/rpc`, state files byte-compatible with the Python daemon; the
  Python suite runs against the binary with `DROVER_TEST_DAEMON_BIN`.
- Prefer system tools over heavy crates where the OS has them (`ssh`, `curl`, `notify-send`, `tar`).
- Every cargo run goes through `scripts/cargo-capped.sh` (memory cap); one heavy build at a time.

## Frontend and windows
Live in drover-web (React UI, Tauri window) and drover-desktop (GPUI window); each has its own README and CI. This repo keeps
the protocol they depend on: change `/ws`, `/rpc` or `drover-proto` only with a compatible tag bump in both.

## Windows and Linux: one app
- One release folder on every OS: `dr` (Rust CLI + daemon), `dr-py` (Python CLI), `drover` (Tauri window, drover-web),
  `drover-native` (GPUI, drover-desktop). core builds `dr` + `dr-py` (`release-linux.yml`, `release-windows.yml`);
  `assemble-bundle.yml` adds the windows and the UI.
- A feature is written once. OS code only as a small shim below one shared function (`running_agent`,
  `process_cwd`, `self_update_apply::swap`, PTY/ConPTY): never a feature switched off with `cfg!(windows)` or
  `sys.platform`. Allowed differences: OS APIs and config values (paths, `.exe`, PATH separator, fonts).
- `ci.yml` builds the whole Rust workspace and runs its tests on Linux and Windows for every push: keep it green.

## One look for both windows
- Colours and sizes of the web UI and the native window come from `design/tokens.json` (generated into drover-web and
  drover-desktop; never edit a generated file). Which UI features each window has is `design/features.json`: a feature
  is not finished until it states where it stands in both. How, step by step: `design/README.md`.
