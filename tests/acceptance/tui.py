"""`spectra` alone at a terminal: the windowed interface, driven as a person
drives it, through a pseudo-terminal, reading what it draws.

The interface's own rules are tested with `cargo test -p spectra_cli`. This
covers what only the built binary shows: that it refuses to open anywhere but a
terminal, that a window runs the real command with its colour, its exit code
and its hidden password prompt, and what lands in the store afterwards.
"""
import base64
import codecs
import os
import pty
import re
import select
import signal
import struct
import subprocess
import time
import unicodedata

import fcntl
import termios

from harness import BINARY, LEGAL, Spectra, scratch

# The driver sets these for the suites that sign; here a command must ask.
os.environ.pop('SPECTRA_PASSWORD', None)
os.environ.pop('SPECTRA_SEED', None)

ROWS, COLS = 30, 100
LEADER = '\x02'
ESCAPE_SEQUENCE = re.compile(r'\x1b(?:\[([?<>=]?)([0-9;:]*)([ -/]*[@-~])|\]([^\x07\x1b]*)(?:\x07|\x1b\\)|[ -/]*[0-~])')
INCOMPLETE = re.compile(r'\x1b(?:\[[?<>=]?[0-9;:]*[ -/]*|\][^\x07\x1b]*\x1b?|[ -/]*)?$')


class Terminal:
    """Enough of a terminal for what the interface writes: cursor moves, text
    and erasing. It keeps the private modes and what reaches the clipboard, and
    answers the capability query, so the interface need not wait out its
    timeout: by default as a terminal without kitty's keyboard protocol."""

    def __init__(self, kitty_keys=False):
        self.kitty_keys = kitty_keys
        self.keyboard_pushes = 0
        self.modes = set()
        self.copied = []
        self.row = self.col = 0
        self.grid = [[' '] * COLS for _ in range(ROWS)]
        self.pending = ''
        self.decoder = codecs.getincrementaldecoder('utf-8')()

    def feed(self, data):
        """Apply `data`; what it asks to be answered."""
        text = self.pending + self.decoder.decode(data)
        self.pending = ''
        tail = INCOMPLETE.search(text)
        if tail:
            self.pending = text[tail.start():]
            text = text[:tail.start()]
        answers = b''
        at = 0
        for match in ESCAPE_SEQUENCE.finditer(text):
            self.write(text[at:match.start()])
            at = match.end()
            answers += self.sequence(*match.groups())
        self.write(text[at:])
        return answers

    def write(self, text):
        for c in text:
            if c == '\r':
                self.col = 0
            elif c == '\n':
                self.row = min(self.row + 1, ROWS - 1)
            elif c >= ' ' and self.col < COLS:
                self.grid[self.row][self.col] = c
                self.col += 2 if unicodedata.east_asian_width(c) in 'WF' else 1

    def sequence(self, prefix, parameters, final, osc):
        if osc is not None:
            if osc.startswith('52;c;'):
                self.copied.append(base64.b64decode(osc[5:]).decode())
            return b''
        if final is None:
            return b''
        numbers = [int(part or 0) for part in (parameters or '').split(';') if part.isdigit() or not part]
        first = numbers[0] if numbers else 0
        if prefix == '?' and final == 'c':
            return b''
        if prefix == '?' and final == 'u':
            return b'\x1b[?0u' if self.kitty_keys else b''
        if prefix == '>' and final == 'u':
            self.keyboard_pushes += 1
        if prefix == '<' and final == 'u':
            self.keyboard_pushes -= 1
        if prefix == '' and final == 'c':
            return b'\x1b[?62c'
        if prefix == '?' and final in 'hl':
            for number in numbers:
                (self.modes.add if final == 'h' else self.modes.discard)(number)
        elif final == 'H':
            self.row = max(first, 1) - 1
            self.col = max(numbers[1] if len(numbers) > 1 else 1, 1) - 1
        elif final == 'J':
            self.grid = [[' '] * COLS for _ in range(ROWS)]
        elif final == 'K':
            self.grid[self.row][self.col:] = [' '] * (COLS - self.col)
        return b''

    def text(self):
        return '\n'.join(''.join(row).rstrip() for row in self.grid)

    def bar(self):
        return ''.join(self.grid[-1]).rstrip()


class Interface:
    """The binary at a terminal of its own, until the block ends."""

    def __init__(self, spectra, *args, kitty_keys=False):
        self.terminal = Terminal(kitty_keys)
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            environment = {**spectra.env, 'TERM': 'xterm-256color', 'SPECTRA_DATA_DIR': str(spectra.directory)}
            os.execve(str(BINARY), [str(BINARY), *args], environment)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack('HHHH', ROWS, COLS, 0, 0))
        self.status = None

    def __enter__(self):
        return self

    def __exit__(self, *_):
        if self.status is None:
            os.kill(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
        os.close(self.fd)

    def pump(self, seconds):
        """Read what the interface writes for up to `seconds`."""
        readable, _, _ = select.select([self.fd], [], [], seconds)
        if not readable:
            return False
        try:
            data = os.read(self.fd, 65536)
        except OSError:
            return False
        if not data:
            return False
        answer = self.terminal.feed(data)
        if answer:
            os.write(self.fd, answer)
        return True

    def see(self, wanted, timeout=30):
        """Wait until `wanted` is on screen; the screen."""
        deadline = time.monotonic() + timeout
        while wanted not in self.terminal.text():
            assert time.monotonic() < deadline, f'never saw {wanted!r} in:\n{self.terminal.text()}'
            self.pump(0.05)
        return self.terminal.text()

    def gone(self, unwanted, timeout=30):
        deadline = time.monotonic() + timeout
        while unwanted in self.terminal.text():
            assert time.monotonic() < deadline, f'still see {unwanted!r} in:\n{self.terminal.text()}'
            self.pump(0.05)

    def settle(self):
        """Read until the interface has stopped drawing."""
        while self.pump(0.2):
            pass

    def idle(self, timeout=30):
        """Wait until no tab is running a command: keys reach the prompt, not
        the command, from here on."""
        deadline = time.monotonic() + timeout
        while True:
            self.settle()
            bar = self.terminal.bar()
            if bar and '•' not in bar:
                return
            assert time.monotonic() < deadline, f'still running:\n{self.terminal.text()}'

    def opened(self):
        """Wait out the listing a new interface starts with."""
        self.see('wallet list')
        self.idle()
        return self

    def keys(self, text):
        os.write(self.fd, text.encode())

    def resize(self, rows, cols):
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack('HHHH', rows, cols, 0, 0))

    def run(self, line, then):
        """Type a command at the prompt, wait until `then` shows and, unless
        the command is waiting on its terminal, until it has ended."""
        self.keys(line + '\r')
        screen = self.see(then)
        if ':' not in then:
            self.idle()
            screen = self.terminal.text()
        return screen

    def quit(self):
        self.idle()
        self.keys(f'{LEADER}Q')
        assert self.exit_code() == 0

    def exit_code(self, timeout=10):
        """Wait for the process to end, reading what it writes on the way."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.pump(0.05)
            pid, status = os.waitpid(self.pid, os.WNOHANG)
            if pid:
                while self.pump(0.05):
                    pass
                self.status = os.waitstatus_to_exitcode(status)
                return self.status
        raise AssertionError(f'did not exit:\n{self.terminal.text()}')


def only_at_a_terminal(spectra):
    """Anywhere else, a missing command is the usage error it always was."""
    base = [str(BINARY), '--data-dir', str(spectra.directory)]
    result = subprocess.run(base, capture_output=True, text=True, stdin=subprocess.DEVNULL, timeout=30)
    assert result.returncode == 2, result
    assert 'a command is required' in result.stderr, result.stderr
    assert not (spectra.directory / 'spectra.sqlite').exists(), 'refusing touches nothing'


def json_is_never_interactive(spectra):
    with Interface(spectra, '--json') as interface:
        assert interface.exit_code() == 2
        assert 1049 not in interface.terminal.modes, 'never took the screen'
        assert 'a command is required' in interface.terminal.text()


def commands_run_as_at_a_shell(spectra):
    with Interface(spectra) as interface:
        interface.opened()
        assert 'no wallets yet' in interface.terminal.text()
        assert 1049 in interface.terminal.modes, 'the alternate screen'
        assert interface.terminal.bar().startswith(' 1 wallet list')

        # Core's refusal, the CLI's message and its exit code, all unchanged.
        interface.run('address validate --chain Ethereum 0xnothex', 'exit 3')
        interface.run('wallet show "no such wallet"', 'exit 1')
        assert 'no wallet matching "no such wallet"' in interface.terminal.text()
        # A copied command runs as written, and a usage error is the CLI's.
        interface.run('spectra chains --tag sidechain', 'exit 2')
        interface.quit()
        assert 1049 not in interface.terminal.modes, 'the screen given back'
        assert 1000 not in interface.terminal.modes, 'mouse reporting off'


def no_second_interface_inside_a_window(spectra):
    with Interface(spectra) as interface:
        interface.opened()
        interface.run('spectra', 'exit 2')
        assert 'a command is required' in interface.terminal.text()
        interface.quit()


def a_hidden_prompt_stays_hidden(spectra):
    """The one thing a pipe could not do: `wallet import` asks on its terminal.
    Neither answer is drawn, and the wallet is sealed with the one typed."""
    password = 'typed-at-the-prompt'
    with Interface(spectra) as interface:
        interface.opened()
        # A prompt is drawn just before it turns the terminal's echo off, so wait
        # for the drawing to stop before answering, as a person would.
        interface.run('wallet import --chain Solana --name Windowed', 'seed phrase:')
        interface.settle()
        interface.keys(LEGAL + '\r')
        interface.see('password:')
        interface.settle()
        interface.keys(password + '\r')
        screen = interface.run('', 'Windowed')
        assert password not in screen
        assert 'legal winner' not in screen
        interface.quit()

    shown = spectra.run('wallet', 'show', 'Windowed')['wallet']
    assert shown['address'] == 'BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX', shown
    spectra.fail('wallet', 'export', 'Windowed', '--yes', code=3, env={'SPECTRA_PASSWORD': 'wrong'})
    export = spectra.run('wallet', 'export', 'Windowed', '--yes', env={'SPECTRA_PASSWORD': password})
    assert export['seedPhrase'] == LEGAL


def tabs_and_windows_keep_their_own_output(spectra):
    with Interface(spectra) as interface:
        interface.opened()
        interface.keys(f'{LEADER}\r')
        interface.see('tall 2/2')
        interface.run('donations', 'Bitcoin')
        interface.keys(f'{LEADER}t')
        interface.see(' 2 spectra')
        screen = interface.see('tall 1/1')
        assert 'wallet list' not in screen.split('\n')[0], 'a new tab is empty'
        interface.run('chains', 'chains\n›')
        interface.keys(f'{LEADER}1')
        screen = interface.see('tall 2/2')
        assert '› wallet list' in screen, 'the first tab kept its output'
        interface.keys(f'{LEADER}l')
        interface.see('fat 2/2')
        interface.quit()


def closing_a_running_command_takes_a_yes(spectra):
    with Interface(spectra) as interface:
        interface.opened()
        interface.keys(f'{LEADER}\r')
        interface.see('tall 2/2')
        interface.run('wallet new --chain Bitcoin --name Abandoned', 'password:')
        interface.keys(f'{LEADER}w')
        interface.see('1 command is still running')
        interface.keys('n')
        interface.see('tall 2/2')
        interface.keys(f'{LEADER}w')
        interface.see('1 command is still running')
        interface.keys('y')
        interface.see('tall 1/1')
        interface.quit()
    assert not any(wallet['name'] == 'Abandoned' for wallet in spectra.run('wallet', 'list')['wallets'])


def a_closed_window_ends_its_command(spectra):
    """Nothing is left running: a process the interface no longer shows could
    still be holding a prompt or finishing a write."""
    name = f'Orphan{os.getpid()}'
    running = lambda: subprocess.run(['pgrep', '-f', name], capture_output=True).returncode == 0
    with Interface(spectra) as interface:
        interface.opened()
        interface.run(f'wallet new --chain Bitcoin --name {name}', 'password:')
        assert running(), 'it was started'
        interface.keys(f'{LEADER}Q')
        interface.see('1 command is still running')
        interface.keys('y')
        assert interface.exit_code() == 0
    deadline = time.monotonic() + 10
    while running():
        assert time.monotonic() < deadline, 'still running after the interface closed'
        time.sleep(0.05)


def interrupting_ends_only_the_command(spectra):
    with Interface(spectra) as interface:
        interface.opened()
        interface.run('wallet new --chain Bitcoin --name Interrupted', 'password:')
        interface.keys('\x03')
        interface.see('exit ')
        interface.idle()
        interface.run('donations', 'Bitcoin')
        interface.quit()
    assert not any(wallet['name'] == 'Interrupted' for wallet in spectra.run('wallet', 'list')['wallets'])


def a_label_copies_the_whole_address(spectra):
    address = spectra.run('wallet', 'show', 'Windowed')['wallet']['address']
    with Interface(spectra) as interface:
        interface.opened()
        interface.see(address)
        interface.keys(f'{LEADER}e')
        interface.see('press a label')
        interface.keys('a')
        interface.see('copied')
        interface.settle()
        assert interface.terminal.copied == [address], interface.terminal.copied
        interface.quit()


def narrowing_the_terminal_wraps_rather_than_cuts(spectra):
    address = spectra.run('wallet', 'show', 'Windowed')['wallet']['address']
    with Interface(spectra) as interface:
        interface.opened()
        interface.see(address)
        interface.resize(ROWS, 30)
        interface.see(address[:25] + '\n')
        interface.resize(ROWS, COLS)
        interface.see(address)
        interface.quit()


def kittys_own_keys_work_where_the_terminal_reports_them(spectra):
    with Interface(spectra, kitty_keys=True) as interface:
        interface.opened()
        assert interface.terminal.keyboard_pushes == 1, 'asked for the protocol'
        interface.keys('\x1b[116;6u')  # Ctrl+Shift+T
        interface.see(' 2 spectra')
        interface.keys('\x1b[13;6u')  # Ctrl+Shift+Enter
        interface.see('tall 2/2')
        interface.keys('\x1b[108;6u')  # Ctrl+Shift+L
        interface.see('fat 2/2')
        interface.keys('\x1b[98;5u')  # Ctrl+B, the leader, in the protocol's spelling
        interface.keys('Q')
        assert interface.exit_code() == 0
        assert interface.terminal.keyboard_pushes == 0, 'gave the keyboard back'


def main():
    with scratch('tui') as directory:
        spectra = Spectra(directory / 'plain')
        only_at_a_terminal(spectra)
        json_is_never_interactive(spectra)
        commands_run_as_at_a_shell(spectra)
        no_second_interface_inside_a_window(spectra)
        a_hidden_prompt_stays_hidden(spectra)
        tabs_and_windows_keep_their_own_output(spectra)
        closing_a_running_command_takes_a_yes(spectra)
        a_closed_window_ends_its_command(spectra)
        interrupting_ends_only_the_command(spectra)
        a_label_copies_the_whole_address(spectra)
        narrowing_the_terminal_wraps_rather_than_cuts(spectra)
        kittys_own_keys_work_where_the_terminal_reports_them(spectra)
        print('The windowed interface runs the real commands')


if __name__ == '__main__':
    main()
