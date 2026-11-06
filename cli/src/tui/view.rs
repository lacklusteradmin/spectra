//! Drawing a frame, kitty's way: the windows edge to edge with one-cell
//! borders and the tab bar along the bottom.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Clear, Paragraph, Widget};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::app::{App, Closing, Mode, Tab};
use super::complete::suggest;
use super::hints::Hint;
use super::keys;
use super::layout::borders;
use super::window::{Menu, PROMPT, Window};
use crate::out;

const MENU_ROWS: usize = 8;
const TAB_TITLE: usize = 24;

fn rgb((r, g, b): (u8, u8, u8)) -> Color {
    Color::Rgb(r, g, b)
}

const SURFACE: Color = Color::Rgb(38, 36, 52);
const ON_SURFACE: Color = Color::Rgb(225, 225, 235);
const ON_ACCENT: Color = Color::Rgb(16, 14, 24);

/// The windows' share of the terminal: all but the tab bar.
pub fn body(size: Size) -> Rect {
    Rect::new(0, 0, size.width, size.height.saturating_sub(1))
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if area.is_empty() {
        return;
    }
    // Here rather than before the frame, so that the windows always fit the
    // buffer they draw to, however the terminal has just been resized.
    app.arrange(body(area.as_size()));
    let bar = Rect::new(0, area.bottom() - 1, area.width, 1);
    let cursor = {
        let buffer = frame.buffer_mut();
        draw_windows(buffer, app);
        draw_borders(buffer, app, area);
        draw_tab_bar(buffer, bar, app);
        draw_mode(buffer, app, area);
        prompt_cursor(app)
    };
    if let Some(cursor) = cursor {
        frame.set_cursor_position(cursor);
    }
}

fn draw_windows(buffer: &mut Buffer, app: &App) {
    let tab = app.tab();
    for (index, window) in tab.windows.iter().enumerate() {
        let Some(rect) = window.rect.filter(|rect| !rect.is_empty()) else {
            continue;
        };
        draw_screen(buffer, window, rect);
        let active = index == tab.active;
        if !window.running() && window.screen().scrollback() == 0 {
            let ghost = active
                .then(|| suggest(&app.commands, window.editor.before_cursor()))
                .filter(|_| window.editor.at_end())
                .and_then(|suggestions| suggestions.ghost());
            draw_prompt(buffer, window, rect, ghost.as_deref());
        }
        draw_scroll_position(buffer, window, rect);
        if active && let Some(menu) = &window.menu {
            draw_menu(buffer, window, rect, menu);
        }
    }
}

fn draw_screen(buffer: &mut Buffer, window: &Window, rect: Rect) {
    let screen = window.screen();
    for row in 0..rect.height {
        for col in 0..rect.width {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            // The wide character to its left already covers this cell.
            if cell.is_wide_continuation() {
                continue;
            }
            let mut style = cell_style(cell);
            if window.selection.is_some_and(|s| s.contains(row, col)) {
                style = style.add_modifier(Modifier::REVERSED);
            }
            let symbol = if cell.has_contents() {
                cell.contents()
            } else {
                " "
            };
            buffer[(rect.x + col, rect.y + row)]
                .set_symbol(symbol)
                .set_style(style);
        }
    }
}

fn cell_style(cell: &vt100::Cell) -> Style {
    let color = |color| match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(index) => Color::Indexed(index),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    };
    let mut modifiers = Modifier::empty();
    modifiers.set(Modifier::BOLD, cell.bold());
    modifiers.set(Modifier::DIM, cell.dim());
    modifiers.set(Modifier::ITALIC, cell.italic());
    modifiers.set(Modifier::UNDERLINED, cell.underline());
    modifiers.set(Modifier::REVERSED, cell.inverse());
    Style::new()
        .fg(color(cell.fgcolor()))
        .bg(color(cell.bgcolor()))
        .add_modifier(modifiers)
}

/// How much of a long line the prompt has scrolled past, keeping the cursor in
/// `room` cells.
fn prompt_offset(before_cursor: &str, room: u16) -> usize {
    let mut width = before_cursor.width();
    let mut skipped = 0;
    for c in before_cursor.chars() {
        if width < usize::from(room) {
            break;
        }
        width -= c.width().unwrap_or(0);
        skipped += c.len_utf8();
    }
    skipped
}

fn draw_prompt(buffer: &mut Buffer, window: &Window, rect: Rect, ghost: Option<&str>) {
    let row = window.screen().cursor_position().0.min(rect.height - 1);
    let y = rect.y + row;
    let accent = Style::new().fg(rgb(out::ACCENT));
    let room = rect.width.saturating_sub(PROMPT.width() as u16);
    let x = rect.x + PROMPT.width() as u16;
    buffer.set_stringn(rect.x, y, PROMPT, usize::from(rect.width), accent);

    let text = window.editor.text();
    let skipped = prompt_offset(window.editor.before_cursor(), room);
    let shown = &text[skipped..];
    let (end, _) = buffer.set_stringn(x, y, shown, usize::from(room), Style::new());
    if let Some(ghost) = ghost {
        let left = usize::from(rect.right().saturating_sub(end));
        buffer.set_stringn(end, y, ghost, left, Style::new().fg(rgb(out::HINT)));
    }
}

/// Where the terminal's own cursor belongs, if anywhere.
fn prompt_cursor(app: &App) -> Option<(u16, u16)> {
    if !matches!(app.mode, Mode::Normal | Mode::Leader) {
        return None;
    }
    let window = app.window();
    let rect = window.rect.filter(|rect| !rect.is_empty())?;
    let screen = window.screen();
    if screen.scrollback() != 0 {
        return None;
    }
    let (row, col) = screen.cursor_position();
    if window.running() {
        if screen.hide_cursor() {
            return None;
        }
        return Some((rect.x + col, rect.y + row.min(rect.height - 1)));
    }
    let prompt = PROMPT.width() as u16;
    let room = rect.width.saturating_sub(prompt);
    let before = window.editor.before_cursor();
    let skipped = prompt_offset(before, room);
    let x = rect.x + prompt + before[skipped..].width() as u16;
    Some((x.min(rect.right() - 1), rect.y + row.min(rect.height - 1)))
}

fn draw_scroll_position(buffer: &mut Buffer, window: &Window, rect: Rect) {
    let screen = window.screen();
    if screen.scrollback() == 0 {
        return;
    }
    let label = format!(" {}/{} ", screen.scrollback(), screen.scrollback_len());
    let x = rect
        .right()
        .saturating_sub(label.width() as u16)
        .max(rect.x);
    let style = Style::new().fg(ON_ACCENT).bg(rgb(out::ACCENT));
    buffer.set_stringn(x, rect.y, &label, usize::from(rect.width), style);
}

fn draw_menu(buffer: &mut Buffer, window: &Window, rect: Rect, menu: &Menu) {
    let items = &menu.suggestions.items;
    let hint_row = usize::from(menu.suggestions.hint.is_some());
    let room = usize::from(rect.height).saturating_sub(hint_row);
    let shown = items.len().min(MENU_ROWS).min(room);
    let height = (shown + hint_row).min(usize::from(rect.height)) as u16;
    if height == 0 {
        return;
    }
    let prompt_row = window.screen().cursor_position().0.min(rect.height - 1);
    let below = rect.height - prompt_row > height;
    let y = if below {
        rect.y + prompt_row + 1
    } else {
        rect.y + prompt_row.saturating_sub(height)
    };
    let area = Rect::new(rect.x, y, rect.width, height);
    Clear.render(area, buffer);
    buffer.set_style(area, Style::new().fg(ON_SURFACE).bg(SURFACE));

    let mut row = area.y;
    let width = usize::from(area.width);
    if let Some(hint) = &menu.suggestions.hint {
        let style = Style::new().fg(rgb(out::HINT)).bg(SURFACE);
        buffer.set_stringn(area.x + 1, row, hint, width.saturating_sub(2), style);
        row += 1;
    }
    if shown == 0 {
        return;
    }
    let selected = menu.selected.unwrap_or(0);
    let first = selected.saturating_sub(shown - 1);
    let label_width = items
        .iter()
        .map(|item| item.label.width())
        .max()
        .unwrap_or(0);
    for (index, item) in items.iter().enumerate().skip(first).take(shown) {
        let picked = menu.selected == Some(index);
        let (label, help) = if picked {
            let style = Style::new().fg(ON_ACCENT).bg(rgb(out::ACCENT));
            (style.add_modifier(Modifier::BOLD), style)
        } else {
            let label = Style::new().fg(ON_SURFACE).bg(SURFACE);
            (label, Style::new().fg(rgb(out::HINT)).bg(SURFACE))
        };
        let line = Rect::new(area.x, row, area.width, 1);
        buffer.set_style(line, help);
        buffer.set_stringn(area.x + 1, row, &item.label, width.saturating_sub(2), label);
        let help_x = usize::from(area.x) + 3 + label_width;
        if help_x < usize::from(area.right()) {
            let room = usize::from(area.right()) - help_x - 1;
            buffer.set_stringn(help_x as u16, row, &item.help, room, help);
        }
        row += 1;
    }
    if items.len() > shown {
        let place = format!(" {}/{} ", selected + 1, items.len());
        let x = area.right().saturating_sub(place.width() as u16);
        let style = Style::new().fg(rgb(out::HINT)).bg(SURFACE);
        buffer.set_stringn(x, row - 1, &place, width, style);
    }
}

fn draw_borders(buffer: &mut Buffer, app: &App, area: Rect) {
    let tab = app.tab();
    let body = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
    let rects: Vec<Option<Rect>> = tab.windows.iter().map(|window| window.rect).collect();
    for cell in borders(body, &rects, tab.active) {
        let color = if cell.active { out::ACCENT } else { out::HINT };
        buffer[(cell.x, cell.y)]
            .set_symbol(cell.symbol)
            .set_style(Style::new().fg(rgb(color)));
    }
}

/// `text` cut to `width` cells, with `…` for what did not fit.
fn truncated(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let mut kept = String::new();
    let mut used = 1;
    for c in text.chars() {
        used += c.width().unwrap_or(0);
        if used > width {
            break;
        }
        kept.push(c);
    }
    kept.push('…');
    kept
}

fn tab_label(index: usize, tab: &Tab) -> String {
    let marker = if tab.running() > 0 {
        "• "
    } else if tab.activity {
        "* "
    } else {
        ""
    };
    let title = truncated(tab.title(), TAB_TITLE);
    format!(" {} {marker}{title} ", index + 1)
}

fn draw_tab_bar(buffer: &mut Buffer, bar: Rect, app: &mut App) {
    app.tab_bar_row = bar.y;
    if let Mode::Confirm { closing, running } = app.mode {
        let what = match closing {
            Closing::Window => "this window",
            Closing::Tab => "this tab",
            Closing::Everything => "Spectra",
        };
        let commands = if running == 1 {
            "1 command is".to_string()
        } else {
            format!("{running} commands are")
        };
        let text = format!(" {commands} still running. Close {what} and end it? y/N ");
        let style = Style::new()
            .fg(ON_ACCENT)
            .bg(rgb(out::DOWN))
            .add_modifier(Modifier::BOLD);
        buffer.set_style(bar, style);
        buffer.set_stringn(bar.x, bar.y, &text, usize::from(bar.width), style);
        app.tab_spans.clear();
        return;
    }

    let right = status(app);
    let right_width = right.width() as u16;
    let tabs_end = bar.right().saturating_sub(right_width + 1);
    let mut x = bar.x;
    let mut spans = Vec::new();
    for (index, tab) in app.tabs.iter().enumerate() {
        let label = tab_label(index, tab);
        let width = label.width() as u16;
        if x + width > tabs_end {
            break;
        }
        let style = if index == app.active_tab {
            Style::new()
                .fg(ON_ACCENT)
                .bg(rgb(out::ACCENT))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(rgb(out::HINT))
        };
        buffer.set_stringn(x, bar.y, &label, usize::from(width), style);
        spans.push((x..x + width, index));
        x += width + 1;
    }
    app.tab_spans = spans;
    let style = Style::new().fg(rgb(out::HINT));
    let at = bar.right().saturating_sub(right_width);
    buffer.set_stringn(at, bar.y, &right, usize::from(bar.width), style);
}

fn status(app: &App) -> String {
    if let Some(flash) = &app.flash {
        return format!("{flash} ");
    }
    match app.mode {
        Mode::Leader => return "^B … ".to_string(),
        Mode::Hints(_) => return "press a label to copy, Esc to cancel ".to_string(),
        _ => {}
    }
    let tab = app.tab();
    format!(
        "{} {}/{} · Tab commands · ^B ? keys ",
        tab.layout.name(),
        tab.active + 1,
        tab.windows.len()
    )
}

fn draw_mode(buffer: &mut Buffer, app: &App, area: Rect) {
    match &app.mode {
        Mode::Help => draw_help(buffer, area),
        Mode::Hints(hints) => {
            if let Some(rect) = app.window().rect {
                draw_hints(buffer, rect, hints);
            }
        }
        _ => {}
    }
}

fn draw_hints(buffer: &mut Buffer, rect: Rect, hints: &[Hint]) {
    let matched = Style::new()
        .fg(rgb(out::UP))
        .add_modifier(Modifier::UNDERLINED);
    let label = Style::new()
        .fg(ON_ACCENT)
        .bg(rgb(out::UP))
        .add_modifier(Modifier::BOLD);
    for hint in hints {
        for &(row, col) in &hint.cells {
            // A resize since the labels were found can leave a cell off screen.
            if let Some(cell) = buffer.cell_mut((rect.x + col, rect.y + row)) {
                cell.set_style(matched);
            }
        }
        let first = hint.cells.first();
        if let Some(cell) =
            first.and_then(|(row, col)| buffer.cell_mut((rect.x + col, rect.y + row)))
        {
            cell.set_char(hint.label).set_style(label);
        }
    }
}

fn draw_help(buffer: &mut Buffer, area: Rect) {
    let keys_width = keys::TABLE
        .iter()
        .map(|(key, _)| key.width())
        .max()
        .unwrap_or(0);
    let mut lines: Vec<String> = keys::TABLE
        .iter()
        .map(|(key, action)| {
            let pad = " ".repeat(keys_width - key.width());
            format!(" {key}{pad}   {action}")
        })
        .collect();
    lines.push(String::new());
    lines.push(" Tab completes commands, flags and values".to_string());
    lines.push(" Drag or double-click to copy; Shift-drag for the terminal's own".to_string());
    lines.push(" Ctrl+C ends a running command; exit closes a window".to_string());

    let width = lines.iter().map(|line| line.width()).max().unwrap_or(0) as u16 + 3;
    let height = lines.len() as u16 + 2;
    let place = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width.min(area.width),
        height.min(area.height),
    );
    Clear.render(place, buffer);
    let block = Block::bordered()
        .title(" After Ctrl+B, or with Ctrl+Shift ")
        .border_style(Style::new().fg(rgb(out::ACCENT)));
    Paragraph::new(lines.join("\n"))
        .block(block)
        .render(place, buffer);
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    use clap::CommandFactory;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{Event as TerminalEvent, KeyCode, KeyEvent, KeyModifiers};

    use super::super::Event;
    use super::super::job::Launch;
    use super::*;

    const WIDTH: u16 = 60;
    const HEIGHT: u16 = 12;

    fn app() -> App {
        let mut commands = crate::Cli::command();
        commands.build();
        let launch = Launch {
            program: "/nonexistent/spectra".into(),
            cwd: "/".into(),
            data_dir: None,
            events: channel().0,
        };
        App::new(commands, launch)
    }

    fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
        let event = TerminalEvent::Key(KeyEvent::new(code, modifiers));
        app.handle(Event::Terminal(event));
    }

    fn press(app: &mut App, code: KeyCode) {
        key(app, code, KeyModifiers::NONE);
    }

    fn typed(app: &mut App, text: &str) {
        text.chars().for_each(|c| press(app, KeyCode::Char(c)));
    }

    fn leader(app: &mut App, code: KeyCode) {
        key(app, KeyCode::Char('b'), KeyModifiers::CONTROL);
        press(app, code);
    }

    /// A frame as text, one string per row, with where the cursor ended.
    fn frame(app: &mut App) -> (Vec<String>, Option<(u16, u16)>) {
        let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let rows = (0..HEIGHT)
            .map(|y| {
                (0..WIDTH)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect();
        (rows, prompt_cursor(app))
    }

    #[test]
    fn a_new_window_is_a_prompt_over_a_tab_bar() {
        let (rows, cursor) = frame(&mut app());
        assert!(rows[0].starts_with("› "));
        assert!(rows[HEIGHT as usize - 1].starts_with(" 1 spectra "));
        assert!(rows[HEIGHT as usize - 1].contains("tall 1/1"));
        assert_eq!(cursor, Some((2, 0)));
    }

    #[test]
    fn typed_text_moves_the_cursor_and_shows_the_rest_of_the_word() {
        let mut app = app();
        typed(&mut app, "stak");
        let (rows, cursor) = frame(&mut app);
        assert!(rows[0].starts_with("› staking"));
        assert_eq!(cursor, Some((6, 0)));
    }

    #[test]
    fn a_long_line_scrolls_to_keep_the_cursor_in_the_window() {
        let mut app = app();
        typed(&mut app, &"a".repeat(100));
        let (_, cursor) = frame(&mut app);
        let (x, _) = cursor.unwrap();
        assert!(x < WIDTH);
    }

    #[test]
    fn a_wide_line_keeps_its_cursor_in_step_with_the_text() {
        let mut app = app();
        typed(&mut app, "钱包");
        let (rows, cursor) = frame(&mut app);
        assert!(rows[0].contains('钱'));
        assert_eq!(cursor, Some((6, 0)));
    }

    #[test]
    fn two_windows_share_a_border_and_tall_is_the_default() {
        let mut app = app();
        leader(&mut app, KeyCode::Enter);
        let (rows, cursor) = frame(&mut app);
        assert_eq!(rows[0].chars().nth(30), Some('│'));
        assert!(rows[0].starts_with("› "));
        assert_eq!(rows[0].chars().nth(31), Some('›'));
        assert_eq!(cursor, Some((33, 0)), "in the new, active window");
        assert!(rows[HEIGHT as usize - 1].contains("tall 2/2"));
    }

    #[test]
    fn the_stack_layout_shows_only_the_active_window() {
        let mut app = app();
        leader(&mut app, KeyCode::Enter);
        for _ in 0..5 {
            leader(&mut app, KeyCode::Char('l'));
        }
        let (rows, _) = frame(&mut app);
        assert!(!rows[0].contains('│'));
        assert!(rows[HEIGHT as usize - 1].contains("stack 2/2"));
    }

    #[test]
    fn output_keeps_its_colour_and_the_prompt_follows_it() {
        let mut app = app();
        frame(&mut app);
        app.tabs[0].windows[0].feed(b"\x1b[38;2;1;2;3mhello\x1b[0m\r\n");
        let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].symbol(), "h");
        assert_eq!(buffer[(0, 0)].fg, Color::Rgb(1, 2, 3));
        assert_eq!(buffer[(0, 1)].symbol(), "›");
    }

    #[test]
    fn the_menu_lists_each_command_with_its_help() {
        let mut app = app();
        press(&mut app, KeyCode::Tab);
        let (rows, _) = frame(&mut app);
        assert!(rows[1].contains("tor"), "{rows:?}");
        assert!(
            rows[1].contains("Start the configured transport"),
            "{rows:?}"
        );
    }

    #[test]
    fn the_menu_opens_above_a_prompt_at_the_bottom() {
        let mut app = app();
        frame(&mut app);
        let lines = "line\r\n".repeat(usize::from(HEIGHT));
        app.tabs[0].windows[0].feed(lines.as_bytes());
        press(&mut app, KeyCode::Tab);
        let (rows, _) = frame(&mut app);
        let prompt = HEIGHT as usize - 2;
        assert!(rows[prompt].starts_with("› "));
        assert!(rows[prompt - 1].contains(" "), "{rows:?}");
        assert!(rows[prompt - MENU_ROWS].starts_with(" tor"), "{rows:?}");
    }

    #[test]
    fn a_scrolled_window_hides_its_prompt_and_says_where_it_is() {
        let mut app = app();
        frame(&mut app);
        let lines: String = (0..40).map(|line| format!("line {line}\r\n")).collect();
        app.tabs[0].windows[0].feed(lines.as_bytes());
        press(&mut app, KeyCode::PageUp);
        let (rows, cursor) = frame(&mut app);
        assert!(rows[0].contains(" 10/30 "), "{rows:?}");
        assert!(!rows.iter().any(|row| row.starts_with("› ")));
        assert_eq!(cursor, None);
    }

    #[test]
    fn the_tab_bar_marks_the_active_tab_and_records_where_each_label_is() {
        let mut app = app();
        leader(&mut app, KeyCode::Char('t'));
        let (rows, _) = frame(&mut app);
        let bar = &rows[HEIGHT as usize - 1];
        assert!(bar.starts_with(" 1 spectra   2 spectra "), "{bar}");
        assert_eq!(app.tab_spans[0].0, 0..11);
        assert_eq!(app.tab_spans[1].0, 12..23);
        let click = ratatui::crossterm::event::MouseEvent {
            kind: ratatui::crossterm::event::MouseEventKind::Down(
                ratatui::crossterm::event::MouseButton::Left,
            ),
            column: 3,
            row: HEIGHT - 1,
            modifiers: KeyModifiers::NONE,
        };
        app.handle(Event::Terminal(TerminalEvent::Mouse(click)));
        assert_eq!(app.active_tab, 0);
    }

    #[test]
    fn a_tab_title_is_cut_rather_than_crowding_the_bar() {
        assert_eq!(truncated("short", 24), "short");
        assert_eq!(truncated("abcdefgh", 5), "abcd…");
        assert_eq!(truncated("钱包钱包", 5), "钱包…");
    }

    #[test]
    fn help_lists_the_whole_table() {
        let mut app = app();
        leader(&mut app, KeyCode::Char('?'));
        let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let text: String = (0..30)
            .flat_map(|y| (0..80).map(move |x| (x, y)))
            .map(|at| terminal.backend().buffer()[at].symbol().to_string())
            .collect();
        for (_, action) in keys::TABLE {
            assert!(text.contains(action), "{action}");
        }
        press(&mut app, KeyCode::Char('x'));
        assert!(matches!(app.mode, Mode::Normal), "any key closes it");
    }

    #[test]
    fn a_long_menu_says_how_far_along_it_is() {
        let mut app = app();
        press(&mut app, KeyCode::Tab);
        let (rows, _) = frame(&mut app);
        let total = app.window().menu.as_ref().unwrap().suggestions.items.len();
        assert!(
            rows[MENU_ROWS].contains(&format!(" 1/{total} ")),
            "{rows:?}"
        );
    }

    #[test]
    fn labels_found_before_a_resize_do_not_draw_off_screen() {
        let mut app = app();
        frame(&mut app);
        let address = "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359";
        let lines = format!("\r\n\r\n\r\n\r\n\r\n\r\n\r\n\r\n{address}\r\n");
        app.tabs[0].windows[0].feed(lines.as_bytes());
        leader(&mut app, KeyCode::Char('e'));
        assert!(matches!(app.mode, Mode::Hints(_)));
        let mut terminal = Terminal::new(TestBackend::new(30, 3)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    }

    #[test]
    fn a_terminal_too_small_for_anything_still_draws() {
        let mut app = app();
        leader(&mut app, KeyCode::Enter);
        leader(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Tab);
        for (width, height) in [(1, 1), (2, 2), (5, 1), (1, 5), (3, 3), (40, 4), (40, 9)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        }
    }
}
