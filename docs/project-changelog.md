# Changelog

## Unreleased
### Added
- Daemon RPC methods the web client already called (they silently did nothing before). Post-run checks: `check.config`,
  `check.run`, `check.list`, `check.cancel`, a `check` event and a `checks` map in the state. Steps come from
  `.drover/checks.toml` in the repo and run only on `check.run`. Review loop: `review.diff`, `review.list`,
  `review.comment.add|edit|resolve|delete`, `review.hunk.accept|reject`, `review.summary`, `review.send`. Review data is
  kept in `<git-common-dir>/drover/review.json`, so nothing is committed.
### Fixed
- After a computer or daemon restart, the previous panes start again on their own (agents with their resume command,
  shells as new shells in their folder). The native window never offered to resume, so its panes stayed stopped.
  Turn off with `[session] resume_on_start = false` (the web UI then asks, as before).
- Windows: a Claude pane started directly (`cmd /c claude.cmd`, npm install) now records its own conversation
  (`claude --resume <id>`); it fell back to `claude --continue`, so several Claude panes in one folder lost theirs.

## 0.2.20
The web UI and the native window look and work alike. drover-web and drover-desktop are tagged v0.2.20 too.
### Added
- `design/tokens.json` holds the colours of both windows. `design/gen-tokens.mjs` generates each window's theme files
  from it, and the CI of drover-web and drover-desktop fails when they differ. `design/features.json` lists every UI
  feature and where it stands in each window (`node design/check-parity.mjs --strict` prints the gaps).
- Web: tabs in the code editor, git change marks in its gutter, the agent beside the code (Ctrl+J), send the selection
  to an agent (Alt+S), quickfix list from post-run checks, and a review loop in the Changes view.
- Native: the themes come from `tokens.json` (terminal cursor and selection colours too), a ranked command palette with
  hints, a banner when an agent is done, and Ctrl+Click on a file path to open it.
### Fixed
- Ctrl+Click on paths printed by Git Bash (`/c/Users/...`) and on UNC paths (`\\server\share`) opens the file instead
  of saying "Outside this space".

## 0.2.19
drover-web and drover-desktop are tagged v0.2.19 too.
### Changed
- Linux: the native window hides the system title bar and shows minimize / maximize / close after `Changes`, as on
  Windows. The empty part of the tab bar moves the window (double click maximizes) and thin edges resize it.
### Fixed
- Windows: a daemon started in a console (`dr server run`) also saves and stops on Ctrl+Break, closing the console and
  shutdown. Before, only Ctrl+C did.
- Release: the Linux workflow fails early when `pyproject.toml` does not hold the tag's version, as the Windows one
  already did. A wrong tag no longer leaves a half release. Its smoke test also checks `/health` and `dr ls`.

## 0.2.18
drover-web and drover-desktop are tagged v0.2.18 too.
### Fixed
- Windows: a `drover.exe` downloaded in a browser installs on a new PC. Its first run fetched the bundle of release
  v0.1.5, which has none, and stopped with "Drover could not install itself … curl: (22) … 404". It now fetches the
  bundle of its own version. PCs that already had Drover were not affected.
- Windows: the PowerShell install (`irm …/install.ps1 | iex`) installs the release bundle itself, checks its SHA-256,
  and can be run again to update.

## 0.2.17
Windows fixes for the native window, and a one-line install. drover-web and drover-desktop are tagged v0.2.17 too.
### Added
- Windows: install from PowerShell with `irm https://raw.githubusercontent.com/doquangthanh/drover-core/main/scripts/install.ps1 | iex`.
  No admin rights, and no "Windows protected your PC" warning: a file fetched this way is not marked as downloaded
  from the internet. Pin a version with `$env:DROVER_VERSION = "v0.2.17"`. The browser download still works.
### Fixed
- Windows: opening Drover no longer opens an extra console window next to it. `drover-native.exe` was built as a
  console program.
- Windows: the native window starts on a PC without the Microsoft Visual C++ Redistributable. `drover-native.exe` needed
  `VCRUNTIME140.dll` and could not find the copy inside the release folder; it now carries the C runtime itself.
- Website: the Download button points to the newest release that has `drover.exe`. Between tagging a release and
  attaching its bundle, `releases/latest/download/drover.exe` returned 404. The page also shows the VS Code keys the
  app uses instead of the old Ctrl+A prefix.

## 0.2.6
### Fixed
- A tab named after its pane ("shell") follows the pane when it is auto-renamed on starting an agent; a tab renamed
  by hand keeps its name.

## 0.2.5
Herdr-inspired agent tracking: per-pane Codex resume, wrapper hint, explainable detection, sidebar summaries,
session backups. drover-web and drover-desktop are tagged v0.2.5 too.
### Added
- `DROVER_AGENT=<agent>` names the agent behind a wrapper (`DROVER_AGENT=claude fence -- claude`); read from the
  process environment (Linux `/proc`, Windows PEB). Panes never inherit it from the daemon.
- `rules.test` explains its answer: `reason` (OSC 7501 report, matched rule, or fallback), `rule_source`, `rule_path`,
  `overrides_bundled`.
- OSC 7501 `summary=<text>` (with or without `state=`): a one-line status shown after the agent's name in the
  sidebar (pane JSON `summary`, max 120 chars). Display only, never changes the state.
- Session safety nets: an unreadable `session.json` (or one from a newer format) is copied to `session-backups/`
  (3 kept) before it is overwritten; saves keep the previous file in `session-snapshots/` at most every 15 minutes
  (48 kept). Recover by copying one over `session.json` while the daemon is stopped.
### Fixed
- Codex panes resume their own conversation (`codex resume <id>`, from `$CODEX_HOME/sessions` by folder) instead of
  `codex resume --last`. Two Codex panes in one folder keep the default, since the files cannot tell them apart.
- Restoring a session no longer resumes one conversation twice: a second pane with the same folder and resume command
  (or one already running) starts fresh.
- drover-web: a tab shows its most urgent pane (Needs you > done > working), not only the focused one.

## 0.2.2
First non-pre-release of the three-repo split: GitHub's "latest" release, so the in-app updater offers it
(it never offers pre-releases such as `-dev`). drover-web and drover-desktop are tagged v0.2.2 too.
### Fixed
- `dr tui`: bright yellow text (pyte `brightbrown`) crashed the pane view with `ColorParseError: 'bright_brown'`; it now maps
  to Rich `bright_yellow`.
- `dr tui`: padding/blank cells had a `None` style, so Textual's monochrome filter crashed rendering (`AttributeError: 'NoneType' object has no attribute 'color'`); they now use an empty `Style()`.
- `dr tui`: clicks never reached the app in a pane, so Claude's "Jump to bottom" (and any clickable TUI) did nothing. When the app turns mouse reporting on (`?1000/1002/1003`, SGR `?1006`), clicks and the wheel are now sent as xterm mouse reports.
- Test `test_dr_run_without_a_template_is_unchanged` expected POSIX quoting on Windows, where `dr run` uses `list2cmdline`.

## 0.2.1-dev
### Changed
- Version bump so Linux and Windows get the same newest build. drover-web and drover-desktop are tagged with the same version;
  `assemble-bundle` joins them. The native window in this release has: whole-interface zoom (Ctrl+Alt+= / - / 0), What's new,
  Feedback, Rules tester, terminal font setting, Settings file editor, Agents Flow sub-agent lanes, terminal mouse reporting,
  and a Files list where a second click closes the file.

### Changed
- The repo is now drover-core (renamed from `drover`; old URLs redirect). The browser UI, the Tauri window and the GPUI window moved to
  drover-web and drover-desktop. The daemon serves the UI from `DROVER_WEB_DIR`; `dr app` opens `drover-native` or `drover` next to `dr`,
  else the browser. `pywebview` and `drover` as a Python command are gone. Releases are joined by `assemble-bundle` from the pieces of the
  three repos (same bundle layout); the updater and `install.sh` read releases of `doquangthanh/drover-core`.
### Added
- Native window: in-app toasts, as in the web UI. A pane you are not looking at going blocked / done / exited, an
  agent trying to answer a prompt (policy), a newer release and a failed request each raise a card bottom right:
  tone colour, agent badge, action button (Answer / Open / Review / Update), slide-in, a countdown bar that pauses
  on hover, at most four, one per pane. Click opens the pane, the Inbox or Settings → Updates.
- Native window: Inbox has a machine filter (all / this computer / each remote) and a side panel with sample toasts,
  which events notify you (`[notify]`), quiet hours and "Send a test notification".
- Native window: Settings → Notifications has controls (Herdr's "Alerts"): per event (needs me / finishes /
  crashes) switch Toast, Desktop and Sound, and pick quiet hours. They edit `[notify]` in settings.toml in place
  (other lines and comments stay).
- Native window: tab settle (Herdr's `tab.settle`): "Put this tab away" in the command palette hides the tab from the
  tab bar and from next / previous, it keeps running; it comes back when shown, from "N put away", or when an agent in
  it needs you.
- Native window: terminal text zoom, Ctrl+= / Ctrl+- / Ctrl+0 (and palette commands). The window's own chrome keeps its size.
- Native window: pasting an image (a screenshot) into a terminal saves it as a file and pastes its path; "Select all in
  terminal" in the palette selects the screen and scrollback; a welcome card shows before the first space exists.
- Native window: themes. Settings → Appearance picks Dark, Light, Midnight or Auto (follows the desktop); saved as
  `theme = "…"` in settings.toml and applied at once. Every colour in the window and the terminal background / text
  follow the theme (the 16 ANSI colours stay as they are).
### Changed
- Native toasts follow `[notify]`: an event whose channels leave out "toast" raises no card in the window.
- "Test notification" (Settings, Inbox) also shows a sample toast in the window, not only the desktop popup.
- Failed requests sent to the daemon now show a "Something went wrong" toast instead of being dropped.

## 0.1.48
### Added
- Native window: every view is drawn in the window, no browser needed. As in Herdr, Agents, Files and Changes open
  as a panel beside the terminals (tab bar buttons) and follow the focused pane: its agent, its folder, its worktree.
  - Agents: the focused agent (Open / Send / Stop subtree), its tree's Flow, Messages and Policy; the sidebar's
    Agents heading opens every tree with a filter.
  - Changes: the focused pane's worktree, files changed vs a base, coloured diff, Commit / Rebase / Push & open PR;
    a Worktrees tab to pick, create, remove and clean up worktrees.
  - Files (read-only): the focused pane's folder tree with git status, the file with line numbers, its diff vs HEAD,
    Copy file; editing is still in the web UI ("Edit in browser").
  - Inbox: who needs you, quick answers (1 / 2 / 3 / Esc), policy prompts (Allow once / Deny), restart / dismiss.
  - Settings: each config file with its errors, Open in editor, test notification, check and apply updates.
  - Machines: this computer and remote machines, Remove, and the Add a machine form (test, add, install dr).
- Native window: resize with the mouse: the gutter between two panes (double-click evens it out, saved for every
  viewer), the panel's left edge (280-900 px) and the sidebar's right edge.
- Native window: VS Code's keyboard shortcuts, as in the web UI (Ctrl+K Ctrl+S lists them): tabs (Ctrl+N, Ctrl+W,
  Ctrl+Tab, Alt+1-9), panes (Ctrl+\\, Ctrl+K Ctrl+\\, Ctrl+K W, Ctrl+1-9, Ctrl+K Ctrl+arrows, swap, resize, zoom),
  panels and views (Ctrl+Shift+A / E / G / I / M, Ctrl+,), Ctrl+B, F8 / Shift+F8.
- Native window: Ctrl+Shift+P / F1 (all commands), Ctrl+P (jump to a terminal or agent), F2 (rename the focused
  agent) and Ctrl+F (find in the focused terminal's history; Enter: older match). They filter while a Vietnamese
  word is still being composed, and ignore accents ("Viet" finds "Việt").
- `files.list`, `files.read`, `files.status` (and `files.write`) take `pane` instead of `space`: the folder the pane's
  program is in, e.g. an agent's worktree outside the space.
### Fixed
- Native window: Vietnamese looked broken with a terminal font that has no Vietnamese letters (Ubuntu Mono): the
  terminal then uses the bundled JetBrains Mono (OFL), as Herdr does, so every letter of a word comes from one font.
- Native window: the dock shows Drover's icon (the window sets WM_CLASS / app_id `drover`).

## 0.1.47
### Fixed
- Native window: Powerline prompts (agnoster and the like) showed boxes. The separators are drawn as shapes and the
  branch icon shows as ⎇: GPUI on Linux ignores icon-only fonts such as PowerlineSymbols.

## 0.1.46
### Changed
- The GPUI window is the default: `drover` (and `drover app`, the menu entry) starts the daemon, then opens
  `drover-native` from the same folder. The Tauri window is the fallback: `DROVER_WINDOW=webview`, or when
  drover-native is missing or cannot start.
- Native window: Agents, Files, Changes, Inbox, Machines and Settings open inside the window instead of the browser
  (each says what is not drawn yet, with "Open in browser"); Escape or a tab click goes back to the terminals.
### Added
- Native window: a text field that types through the input method (Vietnamese Telex), for the views' filters and forms.
### Fixed
- `drover server run` (autostart, restart after an update) no longer leaves two `drover` processes waiting for the
  daemon: the launcher starts it and exits.
### Added
- The daemon log has one start-up line (context, session, time to ready), ~35 ms on Windows.

## 0.1.45
### Fixed
- Windows, installs on 0.1.43: updating replaced the old runtime folder in place instead of moving to the new app
  folder, so `drover.exe` stayed the old one-file app and the next start could go back to 0.1.43. The daemon now
  finds itself in that folder and finishes the move: copies itself to `%LOCALAPPDATA%\drover\app`, puts the new
  `drover.exe` where the old one was (found from its shims or autostart) and restarts from there, keeping port and
  token. Updating once more (to this version or later) is enough.
- Windows: `drover.exe` no longer opens a console window when it forwards arguments to `dr` (autostart, shims).

## 0.1.44
### Changed
- Windows: the `drover.exe` download is the Rust (Tauri) window, as on Linux. On first run it installs the release
  folder (`%LOCALAPPDATA%\drover\app`: `dr`, `dr-py`, `drover`, `drover-native`; ~10 s) and adds Drover to the
  Start menu; later runs open the window from there. With arguments it is `dr` (old autostart and `dr.cmd` keep
  working). Installs up to 0.1.43 move to it through their normal update; the old one-file app and its runtime
  zip are no longer built.
- Windows: panes follow the folder their program is in, and the terminal font follows Windows Terminal, as on
  Linux (GNOME Terminal).
### Removed
- Windows: the agent guess from the terminal title (a folder named `claude-...` turned a shell pane into a claude
  pane). Agents are seen from their process, as on Linux.

## 0.1.43
Windows and Linux move to one app: same folder, same programs, same update (plan 261119-2253).
### Added
- Windows: an agent typed in a shell pane (`claude`, `codex`, `gemini`, `opencode`, also through `.cmd` shims and
  `node`) shows up as that agent, as on Linux: the daemon looks at the programs the pane's shell started.
- Windows: a claude typed in a shell pane resumes its own conversation after a restart, as on Linux.
- Windows release: `drover-windows-x64.zip` (+ `.sha256`), the same folder as the Linux tarball: `dr` (Rust),
  `dr-py` (Python CLI), `drover` (Rust/Tauri window), `drover-native` (GPUI preview). The one-file `drover.exe`
  download is unchanged in this release.
### Changed
- Linux: `dr` is the Rust CLI, as on Windows: agent commands (`ls`, `read`, `send`, `wait`...) answer in ~16 ms
  instead of ~170 ms; the Python CLI is `dr-py` and runs the other commands. `drover-server` stays as a copy.
- Self-update is one code path: the release folder downloads its new archive, checks the SHA-256 and swaps itself
  (Windows moves the files one by one, since a running program's folder can't be renamed, and puts them back on
  any failure).
### Fixed
- Native (GPUI) window: terminal font on Windows matches the web UI (Consolas instead of a fallback font).

## 0.1.42
### Fixed
- Linux: the release build failed (0.1.41 shipped Windows only); the pane output reader did not compile on unix.

## 0.1.41
### Changed
- Windows: the daemon is the Rust one (inside the runtime `dr.exe`), like on Linux. The Python daemon still runs on
  the first start of a new version (it installs the runtime), if the Rust one fails to start, or with
  `DROVER_ENGINE=python`. Same session, ports and protocol: panes, windows and agents carry on. So after an update the
  Rust daemon takes over from the next start (daemon ~12 MB instead of ~90 MB, starts in ~0.1 s instead of ~1.4 s).
### Fixed
- Rust daemon on Windows: panes printed nothing and never exited (ConPTY waited for a cursor reply; the pane exit
  was only seen at end of output, which ConPTY never sends while the pseudoconsole is open).

## 0.1.40
### Changed
- Windows: `dr run` is answered by the Rust `dr.exe` too: a shell pane starts in ~130 ms instead of ~400-850 ms
  (same command quoting as before; role templates and the interactive attach still use the Python CLI).

## 0.1.39
### Changed
- Windows: `dr` agent commands (`ls`, `read`, `send`, `wait`, `prompt`, `keys`, `wait-output`, `rename`, `kill`,
  `hook claude`) are answered by a small Rust `dr.exe`: ~16 ms a call instead of ~170 ms (herdr: ~25 ms).
  Output and exit codes are unchanged; every other command runs the Python CLI (`dr-py.exe`) as before. Agent
  panes call `dr.exe` directly instead of going through `dr.cmd`.

## 0.1.38
### Added
- Preview of a native desktop window, `drover-native` (Linux bundle, opt-in): a GPUI app drawn straight to the GPU,
  one more client of the daemon. Sidebar with spaces, tab bar, splits, zoom, close with confirm, typing through the
  input method (Vietnamese Telex), resize and scroll. Views it does not have yet (Inbox, Changes, Files, Machines,
  Settings, Agent tree) open in the browser. Select/copy and clickable links are not there yet; the Tauri window
  `drover` stays the default.
- Web UI: `#inbox`, `#changes`, `#files`, `#machines`, `#settings`, `#server`, `#tree` in the URL open that view.
### Changed
- Switching space or tab in the app and web UI is fast again: recently shown tabs stay mounted underneath, so going
  back only shows them instead of re-attaching and redrawing their terminals.
### Fixed
- The last line of a terminal was cut off (padding was counted in the wrong place when fitting rows).
- Ctrl+K Ctrl+arrows only move between panes of the shown tab.

## 0.1.37
### Added
- Helpers report to the agent that started them: when one finishes, needs an answer or exits, drover types a
  `<dr from="%7 review" state="done">…</dr>` note into its parent agent. Never into a shell, a parent on a permission
  prompt, a parent inside `dr wait`, or over a message you are still typing. Setting `orchestration.notes`.
- `dr send parent "text"`: a helper reports back itself; the parent sees it tagged with the helper's name.
- `dr wait --children all|any`: wait until none of your helpers is working, or for the next one with news
  (RPC `pane.wait_children`).
- Role templates: `templates/<name>.md` in the config folder (`command` + instructions), started with
  `dr run reviewer "task"`; `dr templates` lists them.
### Changed
- Helpers stop when the agent that started them exits (`orchestration.kill_children_on_parent_exit` now defaults to
  `true`); `dr run --outlive-parent` keeps one running.
- A pane can no longer be named `parent` (it is a `dr` target now).

## 0.1.36
### Changed
- Keyboard works like VS Code in the app and web UI: the Ctrl+A prefix is gone. New keys include Ctrl+Shift+backtick new
  terminal, Ctrl+N new tab, Ctrl+K Ctrl+\ split down, Ctrl+K W close pane, Ctrl+1-9 focus pane, Alt+1-9 go to tab,
  Ctrl+K Ctrl+arrows focus by direction, Ctrl+K Ctrl+M zoom, F8 / Shift+F8 next agent that needs you / is done,
  Ctrl+K Ctrl+S shortcuts. Send Esc, restart, broadcast and the other agent actions are in the command palette.
  The `prefix` setting is no longer used by the UI; the TUI keeps `tui_prefix` (Ctrl+B).

## 0.1.35
### Fixed
- A pane with long output (or a reloaded window) shows its whole history again, up to the `scrollback` setting, with
  colours, links and the cursor in place. Before, only the last 512 KB of raw output came back, so the history was
  short and its first lines could be garbled.
### Security
- After an in-app update, the daemon token no longer leaks into the environment of panes (agents could read it).

## 0.1.34
### Changed
- Linux: the daemon and the window are now Rust (`drover-server`, Tauri `drover`); `dr` stays Python. The daemon
  idles at ~5–8 MB instead of ~106 MB, the install shrinks from ~630 MB to ~100 MB (no Qt), and only one window
  runs at a time. Same files and settings: an update keeps your session. The window needs WebKitGTK 4.1; without
  it `dr app` opens the browser. `DROVER_ENGINE=python` brings back the Python daemon.
- Chat agents talk to the `claude` CLI directly (no Python SDK in the daemon).
- Remote machines use your system `ssh` (so `~/.ssh/config`, ProxyJump and ssh-agent work as in a terminal).
### Fixed
- Vietnamese (ibus Telex) typing in the window: words no longer repeat; Ctrl+A shortcuts work while it is on.
### Security
- Updates and `install.sh` check the release's SHA-256 before installing.

## 0.1.33
### Fixed
- The app opens again from the menu or desktop icon. Since 0.1.29 the packaged `drover` started the terminal UI
  when run without arguments, which has nowhere to draw when launched from the menu, so no window appeared.
  `drover` with no arguments opens the desktop window again; the installer's menu entry runs `drover app`.
  Already on 0.1.32 and the app will not open? Run `~/.local/share/drover/drover app` once, then update.

## 0.1.32
### Changed
- No limit on how many panes and agents run at once, like Herdr: `max_panes` now defaults to `0` (no limit). Set a
  number under `[orchestration]` in `settings.toml` to cap them again. `max_depth` (agents starting agents, 3 levels)
  stays. Existing `settings.toml` files keep their own `max_panes`; change it to `0` to lift the old default of 12.

## 0.1.31
### Changed
- The terminal UI (`dr`) looks like Herdr's: sidebar entries are two lines (state and name; branch, `↑` commits
  ahead, `±` changed files), long names end in "…", and the open space and focused agent are highlighted (the
  cursor shows only while you move through the sidebar). Tabs are blocks with the open one in the accent colour, a
  clickable `+` and the key hint on the right. A tab's only pane fills it without a frame; split panes get frames.
  State colours match the web UI.
### Fixed
- Linux desktop window: terminal text no longer looks letter-spaced. In Qt WebEngine xterm's WebGL renderer drew a
  small fallback font whatever the terminal font; the window now uses the DOM renderer (browsers keep WebGL).
- After a restart each Claude pane resumes its own conversation (`claude --resume <id>`, read from Claude Code's
  session file) instead of the folder's newest one (`--continue`), so two Claude panes in one repo no longer come
  back as the same conversation. Linux/macOS; elsewhere `--continue` as before.

## 0.1.30
### Added
- Terminal font like a terminal app: by default Drover's terminals use your system terminal's font (on Linux the
  GNOME Terminal profile font, else the desktop monospace font, e.g. Ubuntu Mono 13 pt = 17 px) instead of the
  browser's monospace. Pick another font and size in Settings > Appearance > Terminal font (installed monospace fonts
  are listed; kept per device). New RPC `fonts.list`.

## 0.1.29
### Changed
- The terminal UI (`dr` / `dr tui`) works like Herdr: prefix **Ctrl+B** (new setting `tui_prefix`; the web UI keeps
  `prefix`, Ctrl+A) and Herdr's keys: `c` new tab, `n`/`p` next/previous tab, `1-9` tabs of the current space,
  `T`/`X` rename/close tab, `N`/`W`/`D` new/rename/close space, `v`/`-` split, `h j k l` move, `H J K L` swap,
  `g` goto picker, `w`/`b` sidebar, `[` scroll/copy mode, `q` detach. Drover extras: `a` next agent that needs you,
  `r` restore.
- Sidebar shows spaces on top and agents below (plain shells are hidden); a tab bar above the panes.
- With no space yet, the TUI opens one in the current folder.
### Added
- Pane scrollback in the TUI: mouse wheel or copy mode (`y` copies the visible lines).
### Fixed
- A tab created with `tab.create` now reaches clients under its real name instead of "tab".

## 0.1.28
### Changed
- Agents are detected like Herdr: from the program that holds a shell pane's terminal (foreground process group),
  not from the terminal title. Typing `claude`, `codex`, `gemini` or `opencode` turns the pane into that agent;
  quitting back to the prompt turns it into a shell again (and hides it from Agents). Windows keeps the title check.
- "+" in the tab bar, `Ctrl+N` and `Ctrl+A c` open a new terminal instead of the agent picker.
- A new space starts in the folder the focused pane is in now (home when unknown).
- A space is named after the folder its first pane is in and follows `cd`, unless you renamed it.

## 0.1.27
### Changed
- New space works like herdr: no folder or agent picker. "+" / "New space" opens a fresh space with a shell in the
  home folder. Run `claude`, `codex`, `gemini` or `opencode` in it and the pane shows up under Agents; plain shells
  stay out of the Agents list.
- `space.create`: `cwd` is optional (defaults to home); `new: true` always makes a fresh space, even for a folder
  that already has one (named `folder-2`, `folder-3`, ...).

## 0.1.26
### Fixed
- Release builds now bundle Textual (`uv sync --extra tui` in CI), so plain `dr` from `install.sh` opens the terminal
  UI instead of the desktop window.

## 0.1.25
### Changed
- Plain `dr` now opens the terminal UI when Textual is installed (`pip install 'drover[tui]'`). `dr app` still opens
  the desktop window, and `dr` falls back to it when Textual is missing.

## 0.1.24
### Added
- `dr tui`: terminal UI in your own terminal (Textual). Sidebar with spaces, tabs and agent states, split panes with
  live terminals, tmux-style prefix keys (`Ctrl+A`, same as the web UI), a restore dialog for panes from the previous
  session and a layout that adapts to narrow terminals. Needs the optional extra: `pip install 'drover[tui]'`.
  The daemon and web UI are unchanged; `dr` without arguments still opens the desktop window.

## 0.1.23
### Added
- In-app update shows a full-screen "Updating Drover..." cover until the server is back and the page reloads.
### Fixed
- After an in-app update the new daemon keeps the old port and token, and the page reloads once the connection
  returns. Before, a browser tab stayed on the dead address with every agent showing "exit" and the
  "Pick up where you left off?" dialog never appeared.
- An agent started by typing `claude` (or another agent) inside a shell pane now remembers its resume command, so
  "Resume" after a restart continues the conversation (`claude --continue`) instead of opening a fresh one.

## 0.1.22
### Changed
- Terminal and mono text in the web UI use the system default monospace font instead of the bundled JetBrains Mono.

## 0.1.21
### Fixed
- Desktop app (Windows): clicking a link in a terminal pane (plain URLs and Claude Code's OSC 8 links) showed
  "Get an app to open this 'about' link" instead of opening the browser. Links now open in the default browser;
  only http(s) links are opened.

## 0.1.20
### Changed
- Windows: `dr` calls from agent panes and Claude hooks start about 2x faster (measured `dr ls`: ~390 ms -> ~190 ms).
  Each release now also ships `drover-cli-windows-x64.zip`, an unpacked `dr.exe`. The daemon downloads it once per
  version in the background into `<state dir>/runtime/<version>/`, and the `dr` shim uses it as soon as it is
  there. Offline, or if anything fails, `dr` keeps using drover.exe. `drover.exe` is still the only download.
- Packaged builds leave out setuptools / pkg_resources: PyInstaller's setuptools start-up hook imported it (~0.5 s
  in source Python) on every start, while nothing in drover uses it.
### Fixed
- The `dr.cmd` / `hd.cmd` shims were written with doubled carriage returns (`\r\r\n`) and rewritten on every
  pane start; now byte-exact and written only when they change.
- In-app update: a download cut off mid-way (`IncompleteRead`) now fails cleanly instead of with an unexpected error.

## 0.1.19
### Added
- Flow board shows Claude Code sub-agents (Task/Agent tool: planner, tester, ...) as their own lanes under the
  `claude` pane that runs them, with "delegates" / "returned" arrows (with the sub-agent's last message) and a live
  bar. No extra panes, no extra tokens: drover starts `claude` with `--settings <state dir>/claude-subagent-hooks.json`
  (SubagentStart / SubagentStop hooks running the silent `dr hook claude`); `~/.claude/settings.json` is untouched.
  Turn it off with `claude_subagent_hooks = false` in settings.toml. Panes started before the update get it on restart.
### Fixed
- Agent tree Flow board: an agent that exits (a one-shot `dr run claude -p ...`, a finished command) now gets an
  "exited" event with its exit code to its parent (or you), and its activity bar ends there; before, nothing was logged.
- `dr wait <helper> --state done` hung until its timeout when the helper finished while its pane was focused (a
  focused pane goes working → idle, never "done"); that transition now ends the wait, and the Flow board logs it as
  "finished".
- Agent tree events keep at most 500 characters of text, so a long prompt no longer stays in memory in full and goes
  out in every `messages.list` reply.

## 0.1.18
### Changed
- Agent tree Flow board shows every agent, not only helpers started with `dr run`: agents you start get a
  "you started" event, and a "needs you" / "finished" event to you when they block or finish.
### Fixed
- Vietnamese (and other non-ASCII) prompts piped to `dr prompt <agent> -` on Windows no longer arrive as mojibake
  in the agent and the agent tree: the CLI now reads stdin as UTF-8, not cp1252.
- Chat panel on Windows no longer opens a console window for the Agent SDK's `claude.exe` child.

## 0.1.17
### Fixed
- Windows in-app update no longer fails with "an older drover.exe.old is still in use" when a window or daemon from
  an earlier update is still running: the running exe moves to drover.exe.old2, .old3... instead, and every leftover
  .old copy is removed on the next start once nothing runs it.

## 0.1.16
### Changed
- Agent tree redesigned. The tree draws real connector lines, shows each agent's title, a state badge and a name
  filter. The selected agent gets a header with its tree's counts (helpers, working, needs you, done), a progress
  bar and an "Answer" banner for every helper waiting for you.
- New Flow tab (the default): one swimlane per agent of the selected tree, an arrow per `dr run` / `dr send` /
  `dr wait` / done / needs-you / denied event from `messages.list`, and a bar showing when each helper was active.
  Selecting a helper dims what it was not part of. Messages and Policy stay as tabs.

## 0.1.15
### Changed
- Sidebar is back to two halves, Spaces (grouped by machine) and Agents (grouped by space); the single workspace
  tree of 0.1.14 is gone, along with its collapse state, the machine picker on "+", the top-bar workspace title and
  the new-workspace toast. "Workspace" is "Space" again in the UI.
- Chats are listed inside the Agents section, under the space with the same folder (others last), instead of a
  separate "Chats" group.

## 0.1.14
### Changed
- Sidebar is one tree of workspaces: each workspace lists its agents under it and collapses (chevron or ←/→,
  remembered per device), with a dot that still shows when a collapsed agent needs you. The separate Agents list is gone.
- Remote workspaces sit in the same list with a machine chip and its link state; "+" asks which machine when one is
  connected.
- A workspace with a single tab shows its name and folder in the top bar instead of a lone tab.
- A toast names any workspace that appears without being opened from the UI (for example an agent started from the
  CLI in a new folder), with a Rename action.
- "Space" is now "Workspace" everywhere in the UI.

## 0.1.13
### Changed
- In-app toasts redesigned: tone-coloured accent bar and badge (needs you, done, error, update, stopped), an action
  button ("Answer", "Open", "Update"), a countdown bar that pauses on hover, longer lifetime for "needs you", at most
  four on screen. The Inbox side panel sample uses the same card.
- Desktop notifications now say what happened and what the agent was doing (agent name plus the terminal title), use a
  critical urgency for "needs you", and group per pane.

## 0.1.12
### Fixed
- "Check for updates" failed once GitHub's API rate limit (60 calls/hour per IP) was used up; it now falls back to
  the tag that github.com/<repo>/releases/latest redirects to.

## 0.1.11
### Added
- In-app update on Windows: Settings → Updates downloads the release `drover.exe`, checks it is a real program,
  renames the running file to `drover.exe.old`, puts the new one in its place and restarts the server; the `.old`
  copy is removed on the next start.
- The UI checks for a new release 15 s after start and every 12 h, and shows a toast (once per version) that opens
  Settings → Updates.

## 0.1.10
### Added
- `dr prompt NAME "text" [--wait]` (`pane.prompt`): submits a prompt (bracketed paste when the app enabled it), refuses
  an agent on a permission prompt without typing (exit 6, `agent_blocked`), reports `agent_prompt_stalled` (exit 7) when
  the screen shows no work within 5 s, otherwise waits for the turn to settle.
- `dr keys NAME esc up enter ctrl+c shift+tab ...` (`pane.keys`): named keys, under the same permission-prompt policy as
  `dr send`.
- `dr wait-output NAME PATTERN [--regex] [--lines N]` (`pane.wait_output`): wait for text in any pane's recent output.
- `dr read --lines N` on a full-screen agent (alternate screen + SGR mouse, e.g. OpenCode) scrolls the agent's own
  viewport with wheel events, joins the pages once (fixed header/footer kept once) and scrolls back to the bottom.
  Only for idle/done agents.
- `dr rename` (`pane.rename` now returns `{pane, name}` and rejects duplicate names or names with `/` or `:`).
- Layout from scripts, all printing JSON with the ids they create: `dr space new|ls`, `dr tab new|ls|rename|close`
  (`tab.create`), `dr pane split|move|swap|zoom|unzoom|share|focus` (`pane.split`, `pane.move`, `pane.swap`,
  `pane.zoom`, `pane.share`).
- `dr completion bash|zsh|fish|powershell`, generated from the argparse tree; agent names come from `dr ls --names`,
  which never starts the daemon.

### Fixed
- A pane could stay `working` forever after `dr send`: when a quick command went quiet before the 3 s post-input hold
  ended, the monitor stopped re-evaluating it, so `dr wait` timed out. The monitor now keeps checking until the hold
  has debounced.

### Changed
- Renamed herd to Drover everywhere: Python package `drover`, window/app `drover` (`drover.exe` on Windows), CLI `dr`
  (`hd` stays as an alias), env vars `DROVER_*`, pane env `DR_PANE_ID`, worktree branches `dr/<task>`, release assets
  `drover.exe` / `drover-linux-<arch>.tar.gz`, GitHub repo `doquangthanh/drover`. Old `herd` config/state folders and
  the sign-in autostart entry move over automatically on first start; the Linux installer removes an old herd install.
- Web UI rewritten as Vite + React + TypeScript + Zustand (`frontend/`). Same layout, CSS and shortcuts; the sidebar,
  top bar and status bar no longer rebuild on every state event, Ace and the xterm WebGL addon load lazily.
- `/` now serves the built UI from `src/drover/web_dist/` (also reachable at `/ui/`); hashed assets are cached as immutable,
  `index.html` stays `no-store`. Missing build answers `503 UI not built`.
- Building from source needs Node 20.19+ or 22+: `npm --prefix frontend ci && npm --prefix frontend run build`.
- `packaging/drover.spec` bundles `web_dist/` and fails fast when it is missing; the wheel includes it via hatch `artifacts`.

### Removed
- Legacy vanilla-JS UI (`src/drover/web/js`, `index.html`) and vendored xterm files (xterm now comes from npm `@xterm/*`).
- CSS and fonts moved to `frontend/src/styles/` (content unchanged).

### Fixed
- Layout tree type in the frontend matches the daemon (`split`, not `dir`).
