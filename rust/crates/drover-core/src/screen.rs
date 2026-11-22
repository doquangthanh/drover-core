//! A pane's emulated screen: visible text, scrollback, title, terminal modes.
//! Replaces `pane_screen_text.py` (pyte) and `terminal_mode_tracker.py` (alacritty tracks the modes itself).

use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;

/// Collects what the emulator reports back: the window title, and text a program copied with OSC 52 (Claude Code
/// copies its own mouse selection that way; alacritty's default accepts copy only, never clipboard reads). Terminal
/// query replies (`PtyWrite`) are dropped, like the Python daemon did: an attached xterm.js answers them.
#[derive(Clone, Default)]
pub struct ScreenEvents {
    title: Arc<Mutex<String>>,
    copied: Arc<Mutex<Option<String>>>,
}

impl EventListener for ScreenEvents {
    fn send_event(&self, event: Event) {
        match event {
            Event::Title(t) => *self.title.lock().unwrap() = t,
            Event::ResetTitle => self.title.lock().unwrap().clear(),
            Event::ClipboardStore(_, text) => *self.copied.lock().unwrap() = Some(text),
            _ => {}
        }
    }
}

struct Size(usize, usize);

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.1
    }
    fn screen_lines(&self) -> usize {
        self.1
    }
    fn columns(&self) -> usize {
        self.0
    }
}

pub struct PaneScreen {
    term: Term<ScreenEvents>,
    parser: Processor,
    events: ScreenEvents, // shares the title with the listener inside `term`
}

impl PaneScreen {
    pub fn new(cols: usize, rows: usize, history: usize) -> Self {
        let config = Config { scrolling_history: history, ..Config::default() };
        let events = ScreenEvents::default();
        Self { term: Term::new(config, &Size(cols, rows), events.clone()), parser: Processor::new(), events }
    }

    /// Apply output. True when visible text changed (mode switches and terminal queries do not count).
    pub fn feed(&mut self, data: &[u8]) -> bool {
        let before = self.visible_hash();
        self.parser.advance(&mut self.term, data);
        self.visible_hash() != before
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.term.resize(Size(cols, rows));
    }

    pub fn term(&self) -> &Term<ScreenEvents> {
        &self.term
    }

    /// For viewers: scroll the display through history, read modes.
    pub fn term_mut(&mut self) -> &mut Term<ScreenEvents> {
        &mut self.term
    }

    pub fn title(&self) -> String {
        self.events.title.lock().unwrap().clone()
    }

    /// The last text a program copied (OSC 52) since the previous call; a viewer puts it on its clipboard.
    pub fn take_copied(&self) -> Option<String> {
        self.events.copied.lock().unwrap().take()
    }

    fn row_text(&self, y: i32) -> String {
        let grid = self.term.grid();
        let row = &grid[Line(y)];
        let mut s = String::with_capacity(grid.columns());
        for x in 0..grid.columns() {
            let cell = &row[Column(x)];
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            s.push(cell.c);
            if let Some(extra) = cell.zerowidth() {
                s.extend(extra);
            }
        }
        s.trim_end().to_string()
    }

    fn visible_hash(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for y in 0..self.term.grid().screen_lines() as i32 {
            self.row_text(y).hash(&mut h);
        }
        h.finish()
    }

    pub fn visible_lines(&self) -> Vec<String> {
        (0..self.term.grid().screen_lines() as i32).map(|y| self.row_text(y)).collect()
    }

    /// Scrollback, oldest first (empty while a full-screen app is on the alternate screen).
    pub fn history_lines(&self) -> Vec<String> {
        let size = self.term.grid().history_size() as i32;
        (1..=size).rev().map(|n| self.row_text(-n)).collect()
    }

    pub fn all_lines(&self) -> Vec<String> {
        let mut lines = self.history_lines();
        lines.extend(self.visible_lines());
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines
    }

    pub fn read_text(&self, last: Option<usize>) -> String {
        let lines = self.all_lines();
        let start = match last {
            Some(n) if n > 0 => lines.len().saturating_sub(n),
            _ => 0,
        };
        lines[start..].join("\n")
    }

    /// Case-insensitive search; (1-based line number, trimmed line up to 200 chars).
    pub fn search(&self, needle: &str, limit: usize) -> Vec<(usize, String)> {
        let needle = needle.to_lowercase();
        self.all_lines()
            .into_iter()
            .enumerate()
            .filter(|(_, line)| line.to_lowercase().contains(&needle))
            .take(limit)
            .map(|(i, line)| (i + 1, line.trim().chars().take(200).collect()))
            .collect()
    }

    pub fn modes(&self) -> Modes {
        let m = self.term.mode();
        Modes {
            alt_screen: m.contains(TermMode::ALT_SCREEN),
            mouse: m.intersects(TermMode::MOUSE_MODE),
            mouse_sgr: m.contains(TermMode::SGR_MOUSE),
            bracketed_paste: m.contains(TermMode::BRACKETED_PASTE),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modes {
    pub alt_screen: bool,
    pub mouse: bool,
    pub mouse_sgr: bool,
    pub bracketed_paste: bool,
}

impl Modes {
    /// A full-screen app that scrolls its own history on mouse-wheel input (SGR encoded).
    pub fn scrollable_transcript(&self) -> bool {
        self.alt_screen && self.mouse && self.mouse_sgr
    }
}

#[cfg(test)]
#[path = "screen_tests.rs"]
mod tests;
