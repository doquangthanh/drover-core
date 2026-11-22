//! Ported from `tests/test_pane_screen_text.py` and the mode part of `test_terminal_keys_modes_and_page_merge.py`.

use super::*;

#[test]
fn read_text_returns_visible_output() {
    let mut s = PaneScreen::new(40, 5, 100);
    s.feed(b"first line\r\nsecond line\r\n");
    let text = s.read_text(None);
    assert!(text.contains("first line") && text.contains("second line"));
}

#[test]
fn scrolled_off_lines_stay_in_scrollback() {
    let mut s = PaneScreen::new(40, 3, 100);
    for i in 0..10 {
        s.feed(format!("row {i}\r\n").as_bytes());
    }
    let text = s.read_text(None);
    assert!(text.contains("row 0") && text.contains("row 9"), "{text}");
}

#[test]
fn read_last_n_lines_returns_tail() {
    let mut s = PaneScreen::new(40, 3, 100);
    for i in 0..6 {
        s.feed(format!("row {i}\r\n").as_bytes());
    }
    let tail = s.read_text(Some(2));
    assert_eq!(tail.lines().last(), Some("row 5"));
    assert_eq!(tail.lines().count(), 2);
}

#[test]
fn search_is_case_insensitive_and_numbers_lines() {
    let mut s = PaneScreen::new(40, 5, 100);
    s.feed(b"alpha\r\nERROR: boom\r\nomega\r\n");
    assert_eq!(s.search("error", 20), vec![(2, "ERROR: boom".to_string())]);
}

#[test]
fn garbage_escape_sequences_do_not_raise() {
    let mut s = PaneScreen::new(40, 5, 100);
    s.feed(b"\x1b[99999;;;Zxyz\x1b]0;title\x07ok");
    assert!(s.read_text(None).contains("ok"));
    assert_eq!(s.title(), "title");
}

#[test]
fn resize_keeps_existing_text() {
    let mut s = PaneScreen::new(40, 5, 100);
    s.feed(b"hi there\r\n");
    s.resize(60, 10);
    assert!(s.read_text(None).contains("hi there"));
}

#[test]
fn feed_reports_visible_change_only() {
    let mut s = PaneScreen::new(40, 5, 100);
    assert!(s.feed(b"hello"));
    assert!(!s.feed(b"\x1b[?2004h"), "a mode switch is not a visible change");
}

#[test]
fn modes_follow_set_and_reset_sequences() {
    let mut s = PaneScreen::new(40, 5, 100);
    s.feed(b"plain text, no escapes");
    assert!(!s.modes().scrollable_transcript());
    s.feed(b"\x1b[?1049h\x1b[?1002;1006h\x1b[?2004h");
    let m = s.modes();
    assert!(m.alt_screen && m.mouse && m.mouse_sgr && m.bracketed_paste);
    assert!(m.scrollable_transcript());
    s.feed(b"\x1b[?1006l");
    assert!(!s.modes().scrollable_transcript());
    s.feed(b"\x1b[?1049l\x1b[?2004l");
    assert!(!s.modes().alt_screen && !s.modes().bracketed_paste);
}

#[test]
fn utf8_wide_chars_read_back_cleanly() {
    let mut s = PaneScreen::new(40, 5, 100);
    s.feed("tiếng Việt 日本語 🎉".as_bytes());
    assert_eq!(s.visible_lines()[0], "tiếng Việt 日本語 🎉");
}

#[test]
fn osc52_copy_is_kept_once_for_a_viewer() {
    let mut s = PaneScreen::new(20, 4, 100);
    // "héllo" base64-encoded, as Claude Code sends its mouse selection; BEL or ST both end it.
    s.feed(b"\x1b]52;c;aMOpbGxv\x07");
    assert_eq!(s.take_copied().as_deref(), Some("héllo"));
    assert_eq!(s.take_copied(), None);
    // A clipboard read request (`?`) is never answered or stored.
    s.feed(b"\x1b]52;c;?\x1b\\");
    assert_eq!(s.take_copied(), None);
}
