//! `spectra` with no command: kitty's tabs, windows and layouts around the
//! CLI's own commands.
//!
//! Each window has a prompt and runs what is typed there as a real `spectra`
//! process in a pseudo-terminal (see [`job`]), so everything a command does at
//! a terminal it does here unchanged, and nothing about a command is written
//! twice. What can be typed comes from the command definitions ([`complete`]).
//! Scripts keep calling the subcommands directly.

mod app;
mod complete;
mod editor;
mod hints;
mod job;
mod keys;
mod layout;
mod view;
mod window;

use std::io::{IsTerminal, Write, stdin, stdout};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use base64::Engine as _;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event as TerminalEvent, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};

use crate::error::{CliError, CliResult};
use app::App;
use job::Launch;
use window::WindowId;

/// What the interface reacts to, all on one channel so that one thread owns
/// every window.
pub enum Event {
    Terminal(TerminalEvent),
    Output { window: WindowId, bytes: Vec<u8> },
    Exit { window: WindowId, code: Option<u32> },
}

/// `spectra` alone opens the interface only where someone can use it: at a
/// terminal, outside one of its own windows, and not asked for JSON.
pub fn available(json: bool) -> bool {
    !json
        && stdin().is_terminal()
        && stdout().is_terminal()
        && std::env::var_os(job::INSIDE).is_none()
}

pub fn run(data_dir: Option<PathBuf>, mut commands: clap::Command) -> CliResult<()> {
    let failure = |error: std::io::Error| CliError::failure(format!("terminal: {error}"));
    let program = std::env::current_exe().map_err(failure)?;
    let cwd = std::env::current_dir().map_err(failure)?;
    // Propagates the global flags, so each command offers them.
    commands.build();

    let (events, incoming) = channel();
    let launch = Launch {
        program,
        cwd,
        data_dir,
        events: events.clone(),
    };
    let screen = Screen::enter().map_err(failure)?;
    forward_terminal_events(events);
    let result = run_app(App::new(commands, launch), incoming).map_err(failure);
    drop(screen);
    result
}

fn forward_terminal_events(events: Sender<Event>) {
    std::thread::spawn(move || {
        while let Ok(event) = event::read() {
            if events.send(Event::Terminal(event)).is_err() {
                return;
            }
        }
    });
}

fn run_app(mut app: App, incoming: Receiver<Event>) -> std::io::Result<()> {
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    // So the first command starts in a terminal of the window's real size.
    app.arrange(view::body(terminal.size()?));
    app.open();
    loop {
        terminal.draw(|frame| view::draw(frame, &mut app))?;
        for text in app.clipboard.drain(..) {
            copy_to_clipboard(&text)?;
        }
        let Ok(event) = incoming.recv() else {
            return Ok(());
        };
        app.handle(event);
        // Whatever else has arrived joins this frame, so a command printing
        // quickly costs one redraw rather than one per chunk.
        while let Ok(event) = incoming.try_recv() {
            app.handle(event);
        }
        if app.quit {
            return Ok(());
        }
    }
}

/// OSC 52, which also reaches the local clipboard from over SSH; and `pbcopy`
/// on a Mac, whose Terminal.app does not take OSC 52.
fn copy_to_clipboard(text: &str) -> std::io::Result<()> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    let mut out = stdout();
    write!(out, "\x1b]52;c;{encoded}\x07")?;
    out.flush()?;
    #[cfg(target_os = "macos")]
    {
        use std::process::{Command, Stdio};
        if let Ok(mut pbcopy) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() {
            if let Some(mut input) = pbcopy.stdin.take() {
                input.write_all(text.as_bytes())?;
            }
            pbcopy.wait()?;
        }
    }
    Ok(())
}

/// The terminal as the interface needs it, given back however the interface
/// ends. A panic restores it first, so its message lands on a usable screen.
struct Screen {
    enhanced_keys: bool,
}

impl Screen {
    fn enter() -> std::io::Result<Self> {
        enable_raw_mode()?;
        // From here a failure drops this, which undoes what got done.
        let mut screen = Self {
            enhanced_keys: false,
        };
        let mut out = stdout();
        execute!(
            out,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste
        )?;
        // Without it Ctrl+Shift+T and Ctrl+T are one key, and only the leader
        // can reach the table.
        if supports_keyboard_enhancement().unwrap_or(false) {
            execute!(
                out,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )?;
            screen.enhanced_keys = true;
        }
        let enhanced_keys = screen.enhanced_keys;
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore(enhanced_keys);
            previous(info);
        }));
        Ok(screen)
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        restore(self.enhanced_keys);
    }
}

/// Every step is tried: one failing must not leave the rest undone.
fn restore(enhanced_keys: bool) {
    let mut out = stdout();
    if enhanced_keys {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(
        out,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    );
    let _ = disable_raw_mode();
}
