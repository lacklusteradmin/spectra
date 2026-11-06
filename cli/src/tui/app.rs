//! Tabs of windows and what the keyboard and mouse do to them. Everything here
//! is the interface's own view state: wallets, sends and settings stay with
//! the commands the windows run.

use std::ops::Range;
use std::time::{Duration, Instant};

use clap::Command;
use ratatui::crossterm::event::{
    Event as TerminalEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use ratatui::layout::{Position, Rect};

use super::Event;
use super::complete::{self, Item, suggest};
use super::hints::{self, Hint};
use super::job::Launch;
use super::keys::{self, Action};
use super::layout::Layout;
use super::window::{Menu, Selection, Window, WindowId};
use crate::out;

const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const WHEEL_LINES: i32 = 3;

pub struct Tab {
    pub windows: Vec<Window>,
    pub active: usize,
    pub layout: Layout,
    /// A command printed while another tab was showing.
    pub activity: bool,
}

impl Tab {
    fn new(window: Window) -> Self {
        Self {
            windows: vec![window],
            active: 0,
            layout: Layout::Tall,
            activity: false,
        }
    }

    pub fn title(&self) -> &str {
        match self.windows[self.active].title.as_str() {
            "" => "spectra",
            title => title,
        }
    }

    pub fn running(&self) -> usize {
        self.windows
            .iter()
            .filter(|window| window.running())
            .count()
    }
}

/// What closing asks to confirm, with commands still running in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Closing {
    Window,
    Tab,
    Everything,
}

pub enum Mode {
    Normal,
    /// The leader was pressed: the next key is from the table.
    Leader,
    Help,
    Hints(Vec<Hint>),
    Confirm {
        closing: Closing,
        running: usize,
    },
}

pub struct App {
    pub commands: Command,
    launch: Launch,
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
    pub mode: Mode,
    /// A short note in the tab bar, until the next key.
    pub flash: Option<String>,
    pub quit: bool,
    /// Where the last frame put the tab bar and each tab's label, for clicks.
    pub tab_bar_row: u16,
    pub tab_spans: Vec<(Range<u16>, usize)>,
    /// Text to put on the clipboard, written out between frames.
    pub clipboard: Vec<String>,
    /// Never written to disk: this is one session's memory.
    history: Vec<String>,
    next_id: WindowId,
    dragging: bool,
    last_click: Option<(Instant, Position)>,
}

impl App {
    pub fn new(commands: Command, launch: Launch) -> Self {
        Self {
            commands,
            launch,
            tabs: vec![Tab::new(Window::new(0))],
            active_tab: 0,
            mode: Mode::Normal,
            flash: None,
            quit: false,
            tab_bar_row: 0,
            tab_spans: Vec::new(),
            clipboard: Vec::new(),
            history: Vec::new(),
            next_id: 1,
            dragging: false,
            last_click: None,
        }
    }

    /// The first frame's content: where to start, and the wallets, which is
    /// local and quick and says what there is to work with.
    pub fn open(&mut self) {
        self.window_mut().note(
            out::HINT,
            "Spectra. Tab lists commands, Ctrl+B then ? lists keys.",
        );
        self.window_mut().echo("wallet list");
        self.run_line("wallet list");
    }

    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active_tab]
    }

    fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active_tab]
    }

    pub fn window(&self) -> &Window {
        let tab = self.tab();
        &tab.windows[tab.active]
    }

    fn window_mut(&mut self) -> &mut Window {
        let tab = self.tab_mut();
        let active = tab.active;
        &mut tab.windows[active]
    }

    /// Place the active tab's windows in `body`, resizing what changed.
    pub fn arrange(&mut self, body: Rect) {
        let active_tab = self.active_tab;
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            if index != active_tab {
                tab.windows.iter_mut().for_each(|window| window.rect = None);
                continue;
            }
            let rects = tab.layout.arrange(body, tab.windows.len(), tab.active);
            for (window, rect) in tab.windows.iter_mut().zip(rects) {
                window.rect = rect;
                if let Some(rect) = rect {
                    window.fit(rect.height, rect.width);
                }
            }
        }
    }

    pub fn handle(&mut self, event: Event) {
        match event {
            Event::Terminal(TerminalEvent::Key(key)) => self.key(key),
            Event::Terminal(TerminalEvent::Mouse(mouse)) => self.mouse(mouse),
            Event::Terminal(TerminalEvent::Paste(text)) => self.paste(&text),
            // A resize shows at the next frame, which every event brings.
            Event::Terminal(_) => {}
            Event::Output { window, bytes } => {
                let active_tab = self.active_tab;
                for (index, tab) in self.tabs.iter_mut().enumerate() {
                    if let Some(found) = tab.windows.iter_mut().find(|w| w.id == window) {
                        found.feed(&bytes);
                        tab.activity |= index != active_tab;
                    }
                }
            }
            Event::Exit { window, code } => {
                let found = self.tabs.iter_mut().flat_map(|tab| &mut tab.windows);
                if let Some(found) = found.into_iter().find(|w| w.id == window) {
                    found.finish(code);
                }
            }
        }
    }

    // ─── Keyboard ───────────────────────────────────────────────────────────

    fn key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        self.flash = None;
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => {}
            Mode::Help => return,
            Mode::Leader => return self.after_leader(key),
            Mode::Hints(hints) => return self.pick_hint(key, hints),
            Mode::Confirm { closing, .. } => {
                if matches!(key.code, KeyCode::Char('y' | 'Y')) {
                    self.close(closing);
                }
                return;
            }
        }
        if keys::is_leader(key) {
            self.mode = Mode::Leader;
        } else if let Some(action) = keys::chord(key) {
            self.perform(action);
        } else if key.code == KeyCode::F(1) {
            self.perform(Action::Help);
        } else if key.code == KeyCode::PageUp {
            self.perform(Action::ScrollPageUp);
        } else if key.code == KeyCode::PageDown {
            self.perform(Action::ScrollPageDown);
        } else if self.window().running() {
            let window = self.window_mut();
            window.scroll_to(0);
            window.send(keys::encode(key));
        } else {
            self.prompt_key(key);
        }
    }

    fn after_leader(&mut self, key: KeyEvent) {
        if keys::is_leader(key) {
            // Pressed twice: the key itself, which the leader otherwise hides.
            if self.window().running() {
                self.window_mut().send(keys::encode(key));
            } else {
                self.window_mut().editor.left();
            }
        } else if let Some(action) = keys::bound(key.code) {
            self.perform(action);
        }
    }

    fn prompt_key(&mut self, key: KeyEvent) {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let word = control || alt;
        let window = self.window_mut();
        window.selection = None;
        window.scroll_to(0);
        let menu_selected = window.menu.as_ref().is_some_and(|m| m.selected.is_some());
        match key.code {
            KeyCode::Enter if menu_selected => return self.accept_selected(),
            KeyCode::Enter => return self.submit(),
            KeyCode::Tab => return self.tab_key(),
            KeyCode::BackTab | KeyCode::Up if menu_selected => return self.move_menu(-1),
            KeyCode::Down if menu_selected => return self.move_menu(1),
            KeyCode::Esc => {
                window.menu = None;
                return;
            }
            KeyCode::Up => return self.recall(false),
            KeyCode::Down => return self.recall(true),
            KeyCode::Left if word => window.editor.word_left(),
            KeyCode::Left => window.editor.left(),
            KeyCode::Right if word => window.editor.word_right(),
            KeyCode::Right if window.editor.at_end() => return self.accept_ghost(),
            KeyCode::Right => window.editor.right(),
            KeyCode::Home => window.editor.home(),
            KeyCode::End => window.editor.end(),
            KeyCode::Backspace if word => window.editor.delete_word(),
            KeyCode::Backspace => window.editor.backspace(),
            KeyCode::Delete => window.editor.delete(),
            KeyCode::Char(c) if control => match c {
                'a' => window.editor.home(),
                'e' => window.editor.end(),
                'f' => window.editor.right(),
                'u' => window.editor.delete_to_start(),
                'k' => window.editor.delete_to_end(),
                'w' => window.editor.delete_word(),
                'c' => {
                    window.editor.take();
                    window.menu = None;
                }
                'l' => window.clear(),
                'p' => return self.recall(false),
                'n' => return self.recall(true),
                'd' if window.editor.text().is_empty() => return self.close(Closing::Window),
                'd' => window.editor.delete(),
                _ => {}
            },
            KeyCode::Char(c) if !alt => window.editor.insert(c.encode_utf8(&mut [0; 4])),
            _ => {}
        }
        self.refresh_menu();
    }

    fn recall(&mut self, forward: bool) {
        let tab = &mut self.tabs[self.active_tab];
        let editor = &mut tab.windows[tab.active].editor;
        if forward {
            editor.recall_next(&self.history);
        } else {
            editor.recall_previous(&self.history);
        }
    }

    /// An open menu follows what is typed; it closes when nothing is left.
    fn refresh_menu(&mut self) {
        if self.window().menu.is_none() {
            return;
        }
        let suggestions = suggest(&self.commands, self.window().editor.before_cursor());
        let window = self.window_mut();
        window.menu = (!suggestions.items.is_empty()).then_some(Menu {
            suggestions,
            selected: None,
        });
    }

    /// Tab completes the only reading, opens a menu of several, and walks an
    /// open one.
    fn tab_key(&mut self) {
        if self
            .window()
            .menu
            .as_ref()
            .is_some_and(|m| !m.suggestions.items.is_empty())
        {
            return self.move_menu(1);
        }
        let suggestions = suggest(&self.commands, self.window().editor.before_cursor());
        if let [only] = suggestions.items.as_slice() {
            let (start, only) = (suggestions.start, only.clone());
            self.accept(start, &only);
            return;
        }
        let selected = (!suggestions.items.is_empty()).then_some(0);
        self.window_mut().menu = Some(Menu {
            suggestions,
            selected,
        });
    }

    fn move_menu(&mut self, step: isize) {
        let Some(menu) = self.window_mut().menu.as_mut() else {
            return;
        };
        let count = menu.suggestions.items.len() as isize;
        if count == 0 {
            return;
        }
        let at = menu.selected.map_or(-step.min(0) - 1, |at| at as isize);
        menu.selected = Some((at + step).rem_euclid(count) as usize);
    }

    fn accept_selected(&mut self) {
        let Some(menu) = self.window_mut().menu.take() else {
            return;
        };
        if let Some(item) = menu.selected.and_then(|at| menu.suggestions.items.get(at)) {
            self.accept(menu.suggestions.start, item);
        }
    }

    fn accept(&mut self, start: usize, item: &Item) {
        let window = self.window_mut();
        window
            .editor
            .replace_to_cursor(start, &format!("{} ", item.text));
        window.menu = None;
    }

    /// Right at the end of the line takes the completion being shown, when it
    /// is one: a placeholder like `<WALLET>` is for reading.
    fn accept_ghost(&mut self) {
        let suggestions = suggest(&self.commands, self.window().editor.before_cursor());
        if suggestions.typed > 0
            && let Some(item) = suggestions.items.first()
        {
            let (start, item) = (suggestions.start, item.clone());
            self.accept(start, &item);
        }
    }

    fn submit(&mut self) {
        let window = self.window_mut();
        window.menu = None;
        let line = window.editor.take();
        window.echo(&line);
        let line = line.trim().to_string();
        match line.as_str() {
            "" => {}
            "exit" => self.close(Closing::Window),
            "clear" => self.window_mut().clear(),
            _ => {
                if self.history.last() != Some(&line) {
                    self.history.push(line.clone());
                }
                self.run_line(&line);
            }
        }
    }

    pub fn run_line(&mut self, line: &str) {
        let started = complete::arguments(line).and_then(|arguments| {
            let tab = &mut self.tabs[self.active_tab];
            tab.windows[tab.active].start(&self.launch, &arguments)
        });
        if let Err(error) = started {
            self.window_mut().note(out::DOWN, &format!("✗ {error}"));
        }
    }

    fn paste(&mut self, text: &str) {
        let window = self.window_mut();
        if window.running() {
            window.send(text.replace('\n', "\r").into_bytes());
        } else {
            // A pasted newline never runs anything: what would run stays in
            // front of the person who pasted it, until they press Enter.
            let line: Vec<&str> = text.lines().map(str::trim).collect();
            window.editor.insert(&line.join(" "));
        }
    }

    // ─── Actions ────────────────────────────────────────────────────────────

    pub fn perform(&mut self, action: Action) {
        match action {
            Action::NewWindow => {
                let window = self.new_window();
                let tab = self.tab_mut();
                tab.windows.push(window);
                tab.active = tab.windows.len() - 1;
            }
            Action::NewTab => {
                let window = self.new_window();
                self.tabs.push(Tab::new(window));
                self.show_tab(self.tabs.len() - 1);
            }
            Action::CloseWindow => self.ask_close(Closing::Window),
            Action::CloseTab => self.ask_close(Closing::Tab),
            Action::Quit => self.ask_close(Closing::Everything),
            Action::NextWindow => self.step_window(1),
            Action::PreviousWindow => self.step_window(-1),
            Action::NextTab => self.step_tab(1),
            Action::PreviousTab => self.step_tab(-1),
            Action::GotoTab(index) => {
                if index < self.tabs.len() {
                    self.show_tab(index);
                }
            }
            Action::NextLayout => {
                let tab = self.tab_mut();
                tab.layout = tab.layout.next();
            }
            Action::MoveWindowForward => self.move_window(1),
            Action::MoveWindowBackward => self.move_window(-1),
            Action::MoveWindowToTop => {
                let tab = self.tab_mut();
                let window = tab.windows.remove(tab.active);
                tab.windows.insert(0, window);
                tab.active = 0;
            }
            Action::MoveTabForward => self.move_tab(1),
            Action::MoveTabBackward => self.move_tab(-1),
            Action::ScrollLineUp => self.window_mut().scroll_by(1),
            Action::ScrollLineDown => self.window_mut().scroll_by(-1),
            Action::ScrollPageUp => self.scroll_page(1),
            Action::ScrollPageDown => self.scroll_page(-1),
            Action::ScrollTop => self.window_mut().scroll_to(usize::MAX),
            Action::ScrollBottom => self.window_mut().scroll_to(0),
            Action::Hints => {
                let hints = hints::scan(self.window().screen());
                if hints.is_empty() {
                    self.flash = Some("nothing to copy on screen".to_string());
                } else {
                    self.mode = Mode::Hints(hints);
                }
            }
            Action::Clear => self.window_mut().clear(),
            Action::Help => self.mode = Mode::Help,
        }
    }

    fn new_window(&mut self) -> Window {
        let window = Window::new(self.next_id);
        self.next_id += 1;
        window
    }

    fn show_tab(&mut self, index: usize) {
        self.active_tab = index;
        self.tab_mut().activity = false;
    }

    fn step_window(&mut self, step: isize) {
        let tab = self.tab_mut();
        tab.active = stepped(tab.active, step, tab.windows.len());
    }

    fn step_tab(&mut self, step: isize) {
        self.show_tab(stepped(self.active_tab, step, self.tabs.len()));
    }

    fn move_window(&mut self, step: isize) {
        let tab = self.tab_mut();
        let target = stepped(tab.active, step, tab.windows.len());
        tab.windows.swap(tab.active, target);
        tab.active = target;
    }

    fn move_tab(&mut self, step: isize) {
        let target = stepped(self.active_tab, step, self.tabs.len());
        self.tabs.swap(self.active_tab, target);
        self.active_tab = target;
    }

    fn scroll_page(&mut self, direction: i32) {
        let page = self.window().rect.map_or(1, |rect| i32::from(rect.height));
        self.window_mut().scroll_by(direction * (page - 1).max(1));
    }

    /// Closing ends what runs inside. A command mid-broadcast is not something
    /// to end by a stray key, so that takes a yes.
    fn ask_close(&mut self, closing: Closing) {
        let running = match closing {
            Closing::Window => usize::from(self.window().running()),
            Closing::Tab => self.tab().running(),
            Closing::Everything => self.tabs.iter().map(Tab::running).sum(),
        };
        if running == 0 {
            self.close(closing);
        } else {
            self.mode = Mode::Confirm { closing, running };
        }
    }

    fn close(&mut self, closing: Closing) {
        match closing {
            Closing::Everything => self.quit = true,
            Closing::Tab => self.close_tab(),
            Closing::Window => {
                let tab = self.tab_mut();
                if tab.windows.len() == 1 {
                    return self.close_tab();
                }
                tab.windows.remove(tab.active);
                tab.active = tab.active.min(tab.windows.len() - 1);
            }
        }
    }

    fn close_tab(&mut self) {
        if self.tabs.len() == 1 {
            self.quit = true;
            return;
        }
        self.tabs.remove(self.active_tab);
        self.show_tab(self.active_tab.min(self.tabs.len() - 1));
    }

    fn pick_hint(&mut self, key: KeyEvent, hints: Vec<Hint>) {
        let KeyCode::Char(label) = key.code else {
            return;
        };
        if let Some(hint) = hints.into_iter().find(|hint| hint.label == label) {
            self.copy(hint.text);
        }
    }

    fn copy(&mut self, text: String) {
        self.flash = Some(format!("copied {}", out::short_hash(&text)));
        self.clipboard.push(text);
    }

    // ─── Mouse ──────────────────────────────────────────────────────────────

    fn window_at(&self, at: Position) -> Option<usize> {
        let windows = &self.tab().windows;
        windows
            .iter()
            .position(|window| window.rect.is_some_and(|rect| rect.contains(at)))
    }

    fn mouse(&mut self, mouse: MouseEvent) {
        // Only the keyboard answers an open question: a click moving the focus
        // would change which window a pending close is about.
        if !matches!(self.mode, Mode::Normal) {
            return;
        }
        let at = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => self.press(at),
            MouseEventKind::Drag(MouseButton::Left) if self.dragging => self.drag(at),
            MouseEventKind::Up(MouseButton::Left) if self.dragging => {
                self.dragging = false;
                let window = self.window_mut();
                let moved = window.selection.is_some_and(|s| s.anchor != s.head);
                if !moved {
                    window.selection = None;
                } else if let Some(text) = window.selected_text() {
                    self.copy(text);
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let lines = match mouse.kind {
                    MouseEventKind::ScrollUp => WHEEL_LINES,
                    _ => -WHEEL_LINES,
                };
                if let Some(index) = self.window_at(at) {
                    self.tab_mut().windows[index].scroll_by(lines);
                }
            }
            _ => {}
        }
    }

    fn press(&mut self, at: Position) {
        self.flash = None;
        let clicked_tab = (at.y == self.tab_bar_row)
            .then(|| self.tab_spans.iter().find(|(span, _)| span.contains(&at.x)))
            .flatten();
        if let Some(&(_, tab)) = clicked_tab {
            return self.show_tab(tab);
        }
        let Some(index) = self.window_at(at) else {
            return;
        };
        self.tab_mut().active = index;
        let window = self.window_mut();
        let Some(rect) = window.rect else {
            return;
        };
        let cell = (at.y - rect.y, at.x - rect.x);
        let double = self
            .last_click
            .is_some_and(|(when, place)| place == at && when.elapsed() < DOUBLE_CLICK);
        self.last_click = Some((Instant::now(), at));
        if double {
            self.select_word(cell);
        } else {
            self.window_mut().selection = Some(Selection {
                anchor: cell,
                head: cell,
            });
            self.dragging = true;
        }
    }

    fn drag(&mut self, at: Position) {
        let window = self.window_mut();
        let (Some(rect), Some(selection)) = (window.rect, window.selection.as_mut()) else {
            return;
        };
        selection.head = (
            at.y.clamp(rect.top(), rect.bottom().saturating_sub(1)) - rect.y,
            at.x.clamp(rect.left(), rect.right().saturating_sub(1)) - rect.x,
        );
    }

    fn select_word(&mut self, (row, col): (u16, u16)) {
        let window = self.window_mut();
        let Some(word) = hints::word_at(window.screen(), row, col) else {
            return;
        };
        if let (Some(first), Some(last)) = (word.cells.first(), word.cells.last()) {
            window.selection = Some(Selection {
                anchor: *first,
                head: *last,
            });
        }
        self.copy(word.text);
    }
}

fn stepped(at: usize, step: isize, count: usize) -> usize {
    (at as isize + step).rem_euclid(count.max(1) as isize) as usize
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    use clap::CommandFactory;

    use super::*;

    const BODY: Rect = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 23,
    };

    fn app() -> App {
        let mut commands = crate::Cli::command();
        commands.build();
        let launch = Launch {
            program: "/nonexistent/spectra".into(),
            cwd: "/".into(),
            data_dir: None,
            events: channel().0,
        };
        let mut app = App::new(commands, launch);
        app.arrange(BODY);
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        app.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn typed(app: &mut App, text: &str) {
        text.chars().for_each(|c| press(app, KeyCode::Char(c)));
    }

    fn leader(app: &mut App, code: KeyCode) {
        app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
        press(app, code);
    }

    fn line(app: &App) -> &str {
        app.window().editor.text()
    }

    fn screen_text(app: &App) -> String {
        app.window().screen().contents()
    }

    #[test]
    fn the_leader_reaches_the_table_and_a_second_press_cancels_nothing_else() {
        let mut app = app();
        leader(&mut app, KeyCode::Char('t'));
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active_tab, 1);
        leader(&mut app, KeyCode::Enter);
        assert_eq!(app.tab().windows.len(), 2);
        assert_eq!(app.tab().active, 1);
        leader(&mut app, KeyCode::Char('1'));
        assert_eq!(app.active_tab, 0);
        leader(&mut app, KeyCode::Char('9'));
        assert_eq!(app.active_tab, 0, "no ninth tab to go to");
    }

    #[test]
    fn an_unbound_key_after_the_leader_is_dropped_not_typed() {
        let mut app = app();
        leader(&mut app, KeyCode::Char('z'));
        assert_eq!(line(&app), "");
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn windows_and_tabs_step_around_both_ends() {
        let mut app = app();
        leader(&mut app, KeyCode::Enter);
        leader(&mut app, KeyCode::Char(']'));
        assert_eq!(app.tab().active, 0);
        leader(&mut app, KeyCode::Char('['));
        assert_eq!(app.tab().active, 1);
        leader(&mut app, KeyCode::Char('t'));
        leader(&mut app, KeyCode::Right);
        assert_eq!(app.active_tab, 0);
        leader(&mut app, KeyCode::Left);
        assert_eq!(app.active_tab, 1);
    }

    #[test]
    fn moving_a_window_takes_it_along() {
        let mut app = app();
        leader(&mut app, KeyCode::Enter);
        leader(&mut app, KeyCode::Enter);
        let moved = app.window().id;
        leader(&mut app, KeyCode::Char('b'));
        assert_eq!(app.tab().active, 1);
        assert_eq!(app.window().id, moved);
        leader(&mut app, KeyCode::Char('`'));
        assert_eq!(app.tab().active, 0);
        assert_eq!(app.window().id, moved);
    }

    #[test]
    fn closing_the_last_window_of_the_last_tab_quits() {
        let mut app = app();
        leader(&mut app, KeyCode::Enter);
        leader(&mut app, KeyCode::Char('w'));
        assert_eq!(app.tab().windows.len(), 1);
        assert!(!app.quit);
        leader(&mut app, KeyCode::Char('t'));
        leader(&mut app, KeyCode::Char('q'));
        assert_eq!(app.tabs.len(), 1);
        assert!(!app.quit);
        leader(&mut app, KeyCode::Char('w'));
        assert!(app.quit);
    }

    #[test]
    fn output_for_a_closed_window_is_ignored() {
        let mut app = app();
        app.handle(Event::Output {
            window: 99,
            bytes: b"late".to_vec(),
        });
        app.handle(Event::Exit {
            window: 99,
            code: Some(0),
        });
        assert!(!screen_text(&app).contains("late"));
    }

    #[test]
    fn output_in_a_hidden_tab_is_marked_as_activity_until_shown() {
        let mut app = app();
        leader(&mut app, KeyCode::Char('t'));
        let background = app.tabs[0].windows[0].id;
        app.handle(Event::Output {
            window: background,
            bytes: b"news\r\n".to_vec(),
        });
        assert!(app.tabs[0].activity);
        leader(&mut app, KeyCode::Char('1'));
        assert!(!app.tabs[0].activity);
        assert!(screen_text(&app).contains("news"));
    }

    #[test]
    fn a_command_that_cannot_start_says_why_in_its_window() {
        let mut app = app();
        typed(&mut app, "wallet list");
        press(&mut app, KeyCode::Enter);
        assert_eq!(line(&app), "");
        let screen = screen_text(&app);
        assert!(screen.contains("› wallet list"), "{screen}");
        assert!(screen.contains('✗'), "{screen}");
        assert!(!app.window().running());
    }

    #[test]
    fn an_open_quote_is_reported_and_starts_nothing() {
        let mut app = app();
        typed(&mut app, "wallet show \"main");
        press(&mut app, KeyCode::Enter);
        assert!(screen_text(&app).contains("a quote is not closed"));
    }

    #[test]
    fn builtins_act_on_the_window() {
        let mut app = app();
        typed(&mut app, "hello there");
        app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(line(&app), "");
        app.window_mut().feed(b"old output\r\n");
        typed(&mut app, "clear");
        press(&mut app, KeyCode::Enter);
        assert_eq!(screen_text(&app).trim(), "");
        typed(&mut app, "exit");
        press(&mut app, KeyCode::Enter);
        assert!(app.quit);
    }

    #[test]
    fn up_and_down_walk_the_lines_run_in_any_window() {
        let mut app = app();
        typed(&mut app, "chains");
        press(&mut app, KeyCode::Enter);
        leader(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Up);
        assert_eq!(line(&app), "chains");
        press(&mut app, KeyCode::Down);
        assert_eq!(line(&app), "");
    }

    #[test]
    fn tab_completes_the_only_reading() {
        let mut app = app();
        typed(&mut app, "stak");
        press(&mut app, KeyCode::Tab);
        assert_eq!(line(&app), "staking ");
        assert!(app.window().menu.is_none());
    }

    #[test]
    fn tab_opens_a_menu_that_enter_accepts_without_running() {
        let mut app = app();
        press(&mut app, KeyCode::Tab);
        let first = {
            let menu = app.window().menu.as_ref().unwrap();
            assert_eq!(menu.selected, Some(0));
            menu.suggestions.items[1].text.clone()
        };
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Enter);
        assert_eq!(line(&app), format!("{first} "));
        assert!(screen_text(&app).trim().is_empty(), "nothing ran");
    }

    #[test]
    fn a_menu_wraps_both_ways_and_escape_closes_it() {
        let mut app = app();
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Up);
        let menu = app.window().menu.as_ref().unwrap();
        assert_eq!(menu.selected, Some(menu.suggestions.items.len() - 1));
        press(&mut app, KeyCode::Down);
        assert_eq!(app.window().menu.as_ref().unwrap().selected, Some(0));
        press(&mut app, KeyCode::Esc);
        assert!(app.window().menu.is_none());
    }

    #[test]
    fn typing_narrows_an_open_menu() {
        let mut app = app();
        press(&mut app, KeyCode::Tab);
        typed(&mut app, "st");
        let menu = app.window().menu.as_ref().unwrap();
        assert_eq!(menu.suggestions.items.len(), 1);
        assert_eq!(menu.suggestions.items[0].text, "staking");
        typed(&mut app, "zz");
        assert!(app.window().menu.is_none());
    }

    #[test]
    fn right_takes_a_completion_but_not_a_placeholder() {
        let mut app = app();
        typed(&mut app, "dona");
        press(&mut app, KeyCode::Right);
        assert_eq!(line(&app), "donations ");
        app.window_mut().editor.take();
        typed(&mut app, "wallet show ");
        press(&mut app, KeyCode::Right);
        assert_eq!(line(&app), "wallet show ");
    }

    #[test]
    fn a_pasted_newline_does_not_run_anything() {
        let mut app = app();
        app.paste("wallet\n list\n");
        assert_eq!(line(&app), "wallet list");
        assert!(screen_text(&app).trim().is_empty());
    }

    #[test]
    fn hints_copy_by_label_and_any_other_key_cancels() {
        let mut app = app();
        let address = "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359";
        app.window_mut().feed(format!("{address}\r\n").as_bytes());
        leader(&mut app, KeyCode::Char('e'));
        assert!(matches!(&app.mode, Mode::Hints(hints) if hints.len() == 1));
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        assert!(app.clipboard.is_empty());
        leader(&mut app, KeyCode::Char('e'));
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.clipboard, [address]);
        assert!(app.flash.as_ref().unwrap().starts_with("copied"));
    }

    #[test]
    fn hints_with_nothing_to_copy_say_so() {
        let mut app = app();
        leader(&mut app, KeyCode::Char('e'));
        assert!(matches!(app.mode, Mode::Normal));
        assert!(app.flash.is_some());
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn a_drag_copies_what_it_covers_and_a_plain_click_copies_nothing() {
        let mut app = app();
        app.window_mut().feed(b"hello world\r\n");
        let left = MouseButton::Left;
        app.mouse(mouse(MouseEventKind::Down(left), 0, 0));
        app.mouse(mouse(MouseEventKind::Up(left), 0, 0));
        assert!(app.clipboard.is_empty());
        assert!(app.window().selection.is_none());
        app.mouse(mouse(MouseEventKind::Down(left), 6, 0));
        app.mouse(mouse(MouseEventKind::Drag(left), 10, 0));
        app.mouse(mouse(MouseEventKind::Up(left), 10, 0));
        assert_eq!(app.clipboard, ["world"]);
    }

    #[test]
    fn a_click_cannot_change_which_window_a_pending_close_is_about() {
        let mut app = app();
        leader(&mut app, KeyCode::Enter);
        app.arrange(BODY);
        app.mode = Mode::Confirm {
            closing: Closing::Window,
            running: 1,
        };
        app.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, 2));
        assert_eq!(app.tab().active, 1);
    }

    #[test]
    fn a_double_click_copies_the_word() {
        let mut app = app();
        app.window_mut().feed(b"to abc123 now\r\n");
        let down = MouseEventKind::Down(MouseButton::Left);
        app.mouse(mouse(down, 4, 0));
        app.mouse(mouse(MouseEventKind::Up(MouseButton::Left), 4, 0));
        app.mouse(mouse(down, 4, 0));
        assert_eq!(app.clipboard, ["abc123"]);
    }

    #[test]
    fn a_click_focuses_the_window_under_it() {
        let mut app = app();
        leader(&mut app, KeyCode::Enter);
        app.arrange(BODY);
        assert_eq!(app.tab().active, 1);
        let down = MouseEventKind::Down(MouseButton::Left);
        app.mouse(mouse(down, 2, 2));
        assert_eq!(app.tab().active, 0);
    }

    #[test]
    fn the_wheel_scrolls_the_window_under_it_and_typing_returns_to_the_bottom() {
        let mut app = app();
        for line in 0..40 {
            app.window_mut().feed(format!("line {line}\r\n").as_bytes());
        }
        app.mouse(mouse(MouseEventKind::ScrollUp, 2, 2));
        assert_eq!(app.window().screen().scrollback(), WHEEL_LINES as usize);
        typed(&mut app, "x");
        assert_eq!(app.window().screen().scrollback(), 0);
    }
}
