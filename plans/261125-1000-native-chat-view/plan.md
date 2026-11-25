# Native Chat view (drover-desktop, GPUI)

Goal: the web UI's Chat view (talk to Claude, transcript, tool cards, permission cards, composer, model and mode) in
the native window. Tracked in `design/features.json` as `chat-view`.
Work repo: `drover-desktop` (branch `feat/native-chat-view`). The daemon already serves chats; nothing changes in core.

## Protocol (read from code, verified)
- RPC: `chat.create{cwd,name?,model?,permission_mode?}`, `chat.list`, `chat.history{chat}`, `chat.send{chat,text?,images?}`,
  `chat.interrupt`, `chat.answer{chat,request,allow,always?}`, `chat.set_mode`, `chat.set_model`, `chat.options{chat}`
  (commands + models), `chat.files{chat,query?,limit?}`, `chat.rename`, `chat.remove`.
- Events: `chats` (whole list), `chat` = `{chat, op:"item", item}` or `{chat, op:"delta", id, text}`; `state`/`resync`
  carry `chats` too. The native client hands every event to `drain` as `Incoming::Event`, so no drover-proto change.
- Items: user, text, thinking, tool, permission, result, notice (shapes in drover-web `chat-types.ts`).

## Phases
| # | Phase | Status |
|---|-------|--------|
| 1 | Data layer: types, upsert/delta/merge, tool summary, edit diff (pure + tests); events in `drain`; `chat.history` | done |
| 2 | Chat view: chat list, transcript (user, text, thinking, tool, permission, result, notice), composer, Stop, new chat | done |
| 3 | Composer extras: model picker, permission mode, `/` commands and `@` files from `chat.options` / `chat.files` | done (menus, suggestions with keys, pictures) |
| 4 | Parity table, docs, final checks | done, except a manual run in the app |

## Decisions
- Entry: a new `View::Chat` (Ctrl+Shift+C... only if free; else palette "Chats" and "New chat"), left list, right transcript.
- Markdown: block level only at first (fenced code in a monospace box, headings, bullets, paragraphs); inline bold and
  code later if cheap. Images in the composer: not in this plan (marked in features.json note).
- Files under 200 lines: `chat/` module folder, one file per concern.

## Checks (low RAM: one heavy command at a time, capped scope)
- `cargo check -p drover-desktop-native` after each phase; one test file per run (`cargo test ... chat -- --test-threads=1`).
- Not verified without running the app against a daemon with a real `claude` CLI: report as "not run" until done.

## Risks
- GPUI text with mixed styles (inline code) is fiddly: defer.
- Long transcripts: render only the tail first (cap items), scroll via `overflow_y_scroll`.
