//! A kitty window: its own screen and scrollback, a prompt, and at most one
//! running command.

use std::num::NonZeroU16;

use ratatui::layout::Rect;

use super::complete::Suggestions;
use super::editor::Editor;
use super::job::{Job, Launch};
use crate::out;

pub type WindowId = u64;

pub const PROMPT: &str = "› ";

const SCROLLBACK: usize = 10_000;

/// Enough for the scrollback, and small enough to replay on every resize.
const TRANSCRIPT: usize = 4 << 20;

/// An SGR foreground from the CLI's own palette.
fn tinted((r, g, b): (u8, u8, u8), text: &str) -> String {
    format!("\x1b[38;2;{r};{g};{b}m{text}\x1b[0m")
}

fn parser(rows: u16, cols: u16) -> vt100::Parser {
    let size = |cells| NonZeroU16::new(cells).unwrap_or(NonZeroU16::MIN);
    vt100::Parser::new(size(rows), size(cols), SCROLLBACK)
}

/// Text being selected with the mouse, in visible (row, column) cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: (u16, u16),
    pub head: (u16, u16),
}

impl Selection {
    fn ordered(self) -> ((u16, u16), (u16, u16)) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    pub fn contains(self, row: u16, col: u16) -> bool {
        let (start, end) = self.ordered();
        (row, col) >= start && (row, col) <= end
    }
}

/// The completion menu, once Tab has opened it.
#[derive(Debug, Default)]
pub struct Menu {
    pub suggestions: Suggestions,
    pub selected: Option<usize>,
}

pub struct Window {
    pub id: WindowId,
    /// The command last run here; the tab bar shows the active window's.
    pub title: String,
    /// Where the layout put it; `None` when not on screen.
    pub rect: Option<Rect>,
    pub editor: Editor,
    pub menu: Option<Menu>,
    pub selection: Option<Selection>,
    /// Everything written to the screen, replayed on a resize so that text
    /// reflows to the new width instead of being cut off by it.
    transcript: Vec<u8>,
    parser: vt100::Parser,
    size: (u16, u16),
    job: Option<Job>,
}

impl Window {
    pub fn new(id: WindowId) -> Self {
        let size = (24, 80);
        Self {
            id,
            title: String::new(),
            rect: None,
            editor: Editor::default(),
            menu: None,
            selection: None,
            transcript: Vec::new(),
            parser: parser(size.0, size.1),
            size,
            job: None,
        }
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    pub fn running(&self) -> bool {
        self.job.is_some()
    }

    pub fn fit(&mut self, rows: u16, cols: u16) {
        if self.size == (rows, cols) {
            return;
        }
        self.size = (rows, cols);
        let scrolled = self.screen().scrollback();
        self.parser = parser(rows, cols);
        self.parser.process(&self.transcript);
        self.parser.screen_mut().set_scrollback(scrolled);
        self.selection = None;
        if let Some(job) = &self.job {
            job.resize(rows, cols);
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        self.transcript.extend_from_slice(bytes);
        if self.transcript.len() > TRANSCRIPT {
            let cut = self.transcript.len() - TRANSCRIPT / 2;
            let line = self.transcript[cut..].iter().position(|b| *b == b'\n');
            self.transcript
                .drain(..line.map_or(cut, |line| cut + line + 1));
        }
    }

    /// A line as the prompt showed it, kept in the scrollback above its output.
    pub fn echo(&mut self, line: &str) {
        let echoed = format!("{}{line}\r\n", tinted(out::ACCENT, PROMPT));
        self.feed(echoed.as_bytes());
    }

    /// A line of the interface's own, not a command's.
    pub fn note(&mut self, color: (u8, u8, u8), text: &str) {
        let noted = format!("{}\r\n", tinted(color, text));
        self.feed(noted.as_bytes());
    }

    pub fn start(&mut self, launch: &Launch, arguments: &[String]) -> Result<(), String> {
        self.job = Some(Job::spawn(launch, self.id, arguments, self.size)?);
        self.title = arguments.join(" ");
        Ok(())
    }

    /// The command exited: the prompt returns on a line of its own, under the
    /// exit code when that says anything. Exit codes are part of the CLI's
    /// interface, so a window shows them rather than swallowing them.
    pub fn finish(&mut self, code: Option<u32>) {
        self.job = None;
        self.feed(b"\x1b[0m");
        if self.screen().cursor_position().1 != 0 {
            self.feed(b"\r\n");
        }
        match code {
            Some(0) => {}
            Some(code) => self.note(out::HINT, &format!("exit {code}")),
            None => self.note(out::HINT, "exit unknown"),
        }
    }

    pub fn send(&mut self, bytes: Vec<u8>) {
        if let Some(job) = &self.job {
            job.send(bytes);
        }
    }

    pub fn clear(&mut self) {
        self.transcript.clear();
        self.parser = parser(self.size.0, self.size.1);
        self.selection = None;
    }

    pub fn scroll_by(&mut self, lines: i32) {
        let target = self.screen().scrollback() as i64 + i64::from(lines);
        self.scroll_to(usize::try_from(target).unwrap_or(0));
    }

    /// `set_scrollback` caps it at what the scrollback holds.
    pub fn scroll_to(&mut self, lines: usize) {
        self.parser.screen_mut().set_scrollback(lines);
        self.selection = None;
    }

    /// The selected text, as it would paste: wrapped rows rejoined.
    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection?.ordered();
        let text = self
            .screen()
            .contents_between(start.0, start.1, end.0, end.1 + 1);
        Some(text).filter(|text| !text.trim().is_empty())
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        if let Some(job) = &mut self.job {
            job.hang_up();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows_of(window: &Window) -> Vec<String> {
        let (_, cols) = window.screen().size();
        window.screen().rows(0, cols.get()).collect()
    }

    #[test]
    fn narrowing_a_window_wraps_text_instead_of_cutting_it() {
        let mut window = Window::new(1);
        window.feed(b"0123456789abcdefghij\r\n");
        window.fit(24, 10);
        let rows = rows_of(&window);
        assert_eq!(rows[0], "0123456789");
        assert_eq!(rows[1], "abcdefghij");
        window.fit(24, 80);
        assert_eq!(rows_of(&window)[0], "0123456789abcdefghij");
    }

    #[test]
    fn the_transcript_keeps_whole_lines_when_it_outgrows_its_limit() {
        let mut window = Window::new(1);
        let line = [b'x'; 1023];
        for _ in 0..(TRANSCRIPT / 1024 + 8) {
            window.feed(&line);
            window.feed(b"\n");
        }
        assert!(window.transcript.len() <= TRANSCRIPT);
        assert_eq!(window.transcript.first(), Some(&b'x'));
        assert_eq!(window.transcript.last(), Some(&b'\n'));
    }

    #[test]
    fn scrolling_past_either_end_stops_there() {
        let mut window = Window::new(1);
        window.fit(4, 20);
        for line in 0..20 {
            window.feed(format!("line {line}\r\n").as_bytes());
        }
        window.scroll_by(-3);
        assert_eq!(window.screen().scrollback(), 0);
        window.scroll_by(1000);
        assert_eq!(window.screen().scrollback(), 17);
        assert_eq!(rows_of(&window)[0], "line 0");
        window.scroll_to(0);
        assert_eq!(rows_of(&window)[2], "line 19");
    }

    #[test]
    fn a_selection_reads_the_same_whichever_way_it_was_dragged() {
        let mut window = Window::new(1);
        window.feed(b"first line\r\nsecond line\r\n");
        for (anchor, head) in [((0, 6), (1, 5)), ((1, 5), (0, 6))] {
            window.selection = Some(Selection { anchor, head });
            assert_eq!(window.selected_text().unwrap(), "line\nsecond");
        }
    }

    #[test]
    fn selecting_only_blank_cells_copies_nothing() {
        let mut window = Window::new(1);
        window.selection = Some(Selection {
            anchor: (3, 0),
            head: (3, 5),
        });
        assert_eq!(window.selected_text(), None);
    }

    #[test]
    fn clearing_forgets_the_scrollback_too() {
        let mut window = Window::new(1);
        window.feed(b"a seed phrase, say\r\n");
        window.clear();
        window.fit(10, 40);
        assert!(rows_of(&window).iter().all(String::is_empty));
    }
}
