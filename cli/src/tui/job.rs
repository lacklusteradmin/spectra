//! A command running in a window: this same binary in a pseudo-terminal, so it
//! is the command a shell would run. It prompts for a password on its own
//! terminal, colours its report and exits with its own code; the interface
//! never sees a secret it did not need to, and holds no wallet state a
//! long-lived process could write back over a newer store.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{Sender, channel};

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

use super::Event;
use super::window::WindowId;

/// Set in every command a window runs: `spectra` alone inside a window is a
/// usage error, not a second interface inside the first.
pub const INSIDE: &str = "SPECTRA_TUI";

/// What every command starts with.
pub struct Launch {
    pub program: PathBuf,
    pub cwd: PathBuf,
    pub data_dir: Option<PathBuf>,
    pub events: Sender<Event>,
}

pub struct Job {
    master: Box<dyn MasterPty + Send>,
    input: Sender<Vec<u8>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

fn pty_size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

impl Job {
    pub fn spawn(
        launch: &Launch,
        window: WindowId,
        arguments: &[String],
        (rows, cols): (u16, u16),
    ) -> Result<Self, String> {
        let describe = |error: Box<dyn std::fmt::Display>| error.to_string();
        let pair = native_pty_system()
            .openpty(pty_size(rows, cols))
            .map_err(|error| describe(Box::new(error)))?;

        let mut command = CommandBuilder::new(&launch.program);
        command.args(arguments);
        // Without this it would start in the home directory, and a relative
        // `--password-file` would mean something else here than in a shell.
        command.cwd(&launch.cwd);
        command.env(INSIDE, "1");
        command.env("TERM", "xterm-256color");
        if let Some(directory) = &launch.data_dir {
            command.env("SPECTRA_DATA_DIR", directory);
        }

        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| describe(Box::new(error)))?;
        // The master reads end-of-file only once the last slave is closed.
        drop(pair.slave);
        let killer = child.clone_killer();

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| describe(Box::new(error)))?;
        let mut writer = pair
            .master
            .take_writer()
            .map_err(|error| describe(Box::new(error)))?;

        let events = launch.events.clone();
        std::thread::spawn(move || {
            let mut buffer = [0; 8192];
            while let Ok(read @ 1..) = reader.read(&mut buffer) {
                let bytes = buffer[..read].to_vec();
                if events.send(Event::Output { window, bytes }).is_err() {
                    return;
                }
            }
            let code = child.wait().ok().map(|status| status.exit_code());
            let _ = events.send(Event::Exit { window, code });
        });

        // On its own thread: a command not reading its input must not be able
        // to stop the whole interface once the terminal's buffer is full.
        let (input, keys) = channel::<Vec<u8>>();
        std::thread::spawn(move || {
            for bytes in keys {
                if writer
                    .write_all(&bytes)
                    .and_then(|()| writer.flush())
                    .is_err()
                {
                    return;
                }
            }
        });

        Ok(Self {
            master: pair.master,
            input,
            killer,
        })
    }

    pub fn send(&self, bytes: Vec<u8>) {
        // Fails only once the command is gone, and then there is no one to tell.
        let _ = self.input.send(bytes);
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        let _ = self.master.resize(pty_size(rows, cols));
    }

    /// End a command whose window is closing. Closing our master would not
    /// hang it up, because the reader thread holds a copy. Only for a command
    /// still running: after its exit this process id may be someone else's.
    pub fn hang_up(&mut self) {
        let _ = self.killer.kill();
    }
}
