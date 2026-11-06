//! kitty's key table. kitty's bindings all hang off one modifier, Ctrl+Shift
//! by default; a terminal without its keyboard protocol reports Ctrl+Shift+T
//! as plain Ctrl+T, and most terminals keep those chords for themselves. So
//! the same table is also reachable after a leader key every terminal
//! delivers: `Ctrl+B`, then the key.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    NewWindow,
    NewTab,
    CloseWindow,
    CloseTab,
    Quit,
    NextWindow,
    PreviousWindow,
    NextTab,
    PreviousTab,
    GotoTab(usize),
    NextLayout,
    MoveWindowForward,
    MoveWindowBackward,
    MoveWindowToTop,
    MoveTabForward,
    MoveTabBackward,
    ScrollLineUp,
    ScrollLineDown,
    ScrollPageUp,
    ScrollPageDown,
    ScrollTop,
    ScrollBottom,
    Hints,
    Clear,
    Help,
}

/// What the help overlay lists: the key after the leader, and what it does.
pub const TABLE: &[(&str, &str)] = &[
    ("Enter", "new window"),
    ("t", "new tab"),
    ("w", "close window"),
    ("q", "close tab"),
    ("Q", "quit"),
    ("] [", "next, previous window"),
    ("→ ←", "next, previous tab"),
    ("1…9", "go to tab"),
    ("l", "next layout"),
    ("f b `", "move window forward, back, to top"),
    (". ,", "move tab forward, back"),
    ("↑ ↓", "scroll a line"),
    ("PgUp PgDn", "scroll a page"),
    ("Home End", "scroll to top, bottom"),
    ("e", "copy an address or hash by label"),
    ("Delete", "clear the window"),
    ("Ctrl+B", "send Ctrl+B itself"),
    ("?", "this help"),
];

pub fn is_leader(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('b') && key.modifiers == KeyModifiers::CONTROL
}

/// The action of a Ctrl+Shift chord, on a terminal that reports one.
pub fn chord(key: KeyEvent) -> Option<Action> {
    if !key
        .modifiers
        .contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT)
    {
        return None;
    }
    let code = match key.code {
        KeyCode::Char(c) => KeyCode::Char(unshifted(c)),
        code => code,
    };
    bound(code)
}

/// The key under a shifted symbol on a US layout, since a terminal may report
/// either. A Q under the chord is `q`: only the leader can tell them apart.
fn unshifted(c: char) -> char {
    match c {
        '}' => ']',
        '{' => '[',
        '<' => ',',
        '>' => '.',
        '~' => '`',
        '!' => '1',
        '@' => '2',
        '#' => '3',
        '$' => '4',
        '%' => '5',
        '^' => '6',
        '&' => '7',
        '*' => '8',
        '(' => '9',
        c => c.to_ascii_lowercase(),
    }
}

/// What the key after the leader does.
pub fn bound(code: KeyCode) -> Option<Action> {
    Some(match code {
        KeyCode::Enter => Action::NewWindow,
        KeyCode::Char('t') => Action::NewTab,
        KeyCode::Char('w') => Action::CloseWindow,
        KeyCode::Char('q') => Action::CloseTab,
        KeyCode::Char('Q') => Action::Quit,
        KeyCode::Char(']') => Action::NextWindow,
        KeyCode::Char('[') => Action::PreviousWindow,
        KeyCode::Right => Action::NextTab,
        KeyCode::Left => Action::PreviousTab,
        KeyCode::Char(digit @ '1'..='9') => Action::GotoTab(digit as usize - '1' as usize),
        KeyCode::Char('l') => Action::NextLayout,
        KeyCode::Char('f') => Action::MoveWindowForward,
        KeyCode::Char('b') => Action::MoveWindowBackward,
        KeyCode::Char('`') => Action::MoveWindowToTop,
        KeyCode::Char('.') => Action::MoveTabForward,
        KeyCode::Char(',') => Action::MoveTabBackward,
        KeyCode::Up => Action::ScrollLineUp,
        KeyCode::Down => Action::ScrollLineDown,
        KeyCode::PageUp => Action::ScrollPageUp,
        KeyCode::PageDown => Action::ScrollPageDown,
        KeyCode::Home => Action::ScrollTop,
        KeyCode::End => Action::ScrollBottom,
        KeyCode::Char('e') => Action::Hints,
        KeyCode::Delete => Action::Clear,
        KeyCode::Char('?') | KeyCode::F(1) => Action::Help,
        _ => return None,
    })
}

/// A key as the bytes a terminal sends a running command for it.
pub fn encode(key: KeyEvent) -> Vec<u8> {
    let sequence = |text: &str| text.as_bytes().to_vec();
    let mut bytes = match key.code {
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => control(c),
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => sequence("\x1b[Z"),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => sequence("\x1b[A"),
        KeyCode::Down => sequence("\x1b[B"),
        KeyCode::Right => sequence("\x1b[C"),
        KeyCode::Left => sequence("\x1b[D"),
        KeyCode::Home => sequence("\x1b[H"),
        KeyCode::End => sequence("\x1b[F"),
        KeyCode::Delete => sequence("\x1b[3~"),
        _ => Vec::new(),
    };
    if key.modifiers.contains(KeyModifiers::ALT) && !bytes.is_empty() {
        bytes.insert(0, 0x1b);
    }
    bytes
}

/// Ctrl with a key: the control code at that key's place in ASCII.
fn control(c: char) -> Vec<u8> {
    match c.to_ascii_uppercase() {
        upper @ '@'..='_' => vec![upper as u8 & 0x1f],
        ' ' => vec![0],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    const CTRL_SHIFT: KeyModifiers = KeyModifiers::CONTROL.union(KeyModifiers::SHIFT);

    #[test]
    fn a_chord_needs_both_modifiers() {
        let t = KeyCode::Char('t');
        assert_eq!(chord(press(t, CTRL_SHIFT)), Some(Action::NewTab));
        assert_eq!(chord(press(t, KeyModifiers::CONTROL)), None);
        assert_eq!(chord(press(t, KeyModifiers::NONE)), None);
    }

    #[test]
    fn a_chord_reads_the_same_whichever_shifted_form_arrives() {
        for c in ['T', 't'] {
            let key = press(KeyCode::Char(c), CTRL_SHIFT);
            assert_eq!(chord(key), Some(Action::NewTab));
        }
        for c in [']', '}'] {
            let key = press(KeyCode::Char(c), CTRL_SHIFT);
            assert_eq!(chord(key), Some(Action::NextWindow));
        }
        let two = press(KeyCode::Char('@'), CTRL_SHIFT);
        assert_eq!(chord(two), Some(Action::GotoTab(1)));
    }

    #[test]
    fn only_the_leader_can_quit() {
        assert_eq!(bound(KeyCode::Char('Q')), Some(Action::Quit));
        let chorded = press(KeyCode::Char('Q'), CTRL_SHIFT);
        assert_eq!(chord(chorded), Some(Action::CloseTab));
    }

    #[test]
    fn the_leader_is_control_b_alone() {
        let b = KeyCode::Char('b');
        assert!(is_leader(press(b, KeyModifiers::CONTROL)));
        assert!(!is_leader(press(b, CTRL_SHIFT)));
        assert!(!is_leader(press(b, KeyModifiers::NONE)));
    }

    #[test]
    fn a_running_command_gets_what_a_terminal_would_send() {
        let bytes = |code, modifiers| encode(press(code, modifiers));
        let none = KeyModifiers::NONE;
        assert_eq!(bytes(KeyCode::Char('é'), none), "é".as_bytes());
        assert_eq!(bytes(KeyCode::Enter, none), b"\r");
        assert_eq!(bytes(KeyCode::Backspace, none), [0x7f]);
        assert_eq!(bytes(KeyCode::Up, none), b"\x1b[A");
        assert_eq!(bytes(KeyCode::Char('c'), KeyModifiers::CONTROL), [0x03]);
        assert_eq!(bytes(KeyCode::Char('d'), KeyModifiers::CONTROL), [0x04]);
        assert_eq!(bytes(KeyCode::Char('x'), KeyModifiers::ALT), b"\x1bx");
        assert!(bytes(KeyCode::F(5), none).is_empty());
    }
}
