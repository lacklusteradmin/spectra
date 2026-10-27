"""What every acceptance suite shares: the binary under test, its data
directories, the loopback nodes it talks to and the fixtures core's tests
read.

A suite is a script run as `python3 -B tests/acceptance/<suite>.py
[path/to/spectra]`; `tests/acceptance/__main__.py` runs them all. Each checks
with `assert`, so this module refuses to load with assertions disabled.

A suite run on its own is confined to loopback as the driver confines it:
core refuses and journals any request to another host, and a journaled
request fails the run.
"""
import atexit
import contextlib
import http.server
import importlib.util
import json
import os
import pathlib
import sqlite3
import subprocess
import sys
import tempfile
import threading

if not __debug__:
    raise SystemExit('the acceptance suites check with assert: run them without -O or PYTHONOPTIMIZE')

ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURES = ROOT / 'core/tests/fixtures'
# A suite's first argument, when it is not an option, is the binary; a
# unittest suite passes the rest to unittest (`unittest_main`).
GIVEN = sys.argv[1:2] if sys.argv[1:2] and not sys.argv[1].startswith('-') else []
BINARY = pathlib.Path(GIVEN[0] if GIVEN else ROOT / 'target/debug/spectra').resolve()
# The suite's own arguments: what follows the binary.
ARGS = sys.argv[1 + len(GIVEN):]


def _confine_to_loopback():
    """Journal what core refuses beyond loopback, and fail the run on it."""
    descriptor, name = tempfile.mkstemp(prefix='spectra-loopback-')
    os.close(descriptor)
    journal = pathlib.Path(name)
    os.environ['SPECTRA_LOOPBACK_ONLY'] = str(journal)

    def check():
        reached = journal.read_text()
        journal.unlink()
        if reached:
            print(f'requests beyond loopback were refused:\n{reached}', file=sys.stderr)
            os._exit(1)
    atexit.register(check)


if 'SPECTRA_LOOPBACK_ONLY' not in os.environ:
    _confine_to_loopback()

# The BIP-39 phrases most fixtures were generated from.
ABANDON = ' '.join(['abandon'] * 11 + ['about'])
LEGAL = 'legal winner thank year wave sausage worth useful legal winner thank yellow'


def fixture(name):
    """A reference file core's tests read, as JSON."""
    return json.loads((FIXTURES / name).read_text())


def companion(name):
    """A fixture program built beside the CLI (`spectra-zcash-fixture`), built
    where cargo builds it when it is not there."""
    beside = BINARY.parent / name
    if beside.exists():
        return beside
    package = name.replace('-', '_')
    subprocess.run(['cargo', 'build', '-p', package, '--quiet'], cwd=ROOT, check=True)
    return ROOT / 'target/debug' / name


def suite(name):
    """Another suite's module, for a node it defines: `suite('staking').Node`.
    Its own run is behind `if __name__ == '__main__'`, so loading runs nothing."""
    spec = importlib.util.spec_from_file_location(f'acceptance_{name}', pathlib.Path(__file__).parent / f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@contextlib.contextmanager
def scratch(prefix):
    """A directory for the block's length, removed after it."""
    with tempfile.TemporaryDirectory(prefix=f'spectra-{prefix}-') as directory:
        yield pathlib.Path(directory)


class Spectra:
    """The binary against one data directory, with the environment every
    process it starts sees and how long one may take. `--json` is always on:
    a suite reads reports."""

    def __init__(self, directory, timeout=60, **env):
        self.directory = pathlib.Path(directory)
        self.timeout = timeout
        self.overrides = env
        self.env = {**os.environ, **env}

    def __truediv__(self, name):
        """The same environment against a data directory inside this one:
        another signer's, another device's."""
        return Spectra(self.directory / name, self.timeout, **self.overrides)

    def process(self, *args, env=None, input=None, timeout=None):
        """The finished process, whatever its exit."""
        return subprocess.run(
            [str(BINARY), '--data-dir', str(self.directory), '--json', *map(str, args)],
            capture_output=True, text=True, input=input, timeout=timeout or self.timeout,
            env={**self.env, **(env or {})})

    def run(self, *args, **options):
        """A command that must succeed: its report."""
        result = self.process(*args, **options)
        assert result.returncode == 0, (args, result.returncode, result.stdout, result.stderr)
        return json.loads(result.stdout)

    def fail(self, *args, code=None, **options):
        """A command that must fail, with `code` when given: its error document."""
        result = self.process(*args, **options)
        assert result.returncode != 0 if code is None else result.returncode == code, \
            (args, result.returncode, result.stdout, result.stderr)
        return json.loads(result.stdout) if result.stdout.strip() else {}

    def lose_status(self, transaction_id):
        """Set a stored transaction back to pending, as if the write that
        recorded its outcome had been lost: the next poll must find it again."""
        with sqlite3.connect(self.directory / 'spectra.sqlite') as store:
            query = 'SELECT payload FROM history_records WHERE id=?'
            payload = json.loads(store.execute(query, (transaction_id,)).fetchone()[0])
            payload['status'] = 'pending'
            store.execute('UPDATE history_records SET payload=? WHERE id=?', (json.dumps(payload), transaction_id))

    def sign(self, session, signer):
        """Sign a multisig session as `signer`, one of this directory's
        wallets, for the digest it shows: the session as it now stands."""
        return self.run('multisig', 'sign', session['id'], '--review-digest', session['reviewDigest'],
                        '--signer', signer)['session']


def signed_by(session):
    """The signers that have signed a multisig session, sorted."""
    return sorted(signer['signer'] for signer in session['signers'] if signer['signed'])


def checked(spectra, errors):
    """`spectra.run`, or `spectra.fail` when `success` is false: either way,
    the node, which records what it refused in `errors`, refused nothing the
    command asked of it."""
    def run(*args, success=True):
        try:
            return spectra.run(*args) if success else spectra.fail(*args)
        finally:
            assert not errors, errors
    return run


class ChainFixture:
    """One of the fixture programs (`companion`) serving a synthetic chain on
    which the wallet's phrase is `ABANDON` and an outsider's `LEGAL`. `info`
    is the line it prints once listening; its journal is what it mined."""

    def __init__(self, program, directory, *args):
        self.journal = pathlib.Path(directory) / 'fixture.jsonl'
        self.process = subprocess.Popen(
            [str(companion(program)), '--wallet-phrase-env', 'WALLET', '--outsider-phrase-env', 'OUTSIDER',
             '--journal', str(self.journal), *map(str, args)],
            stdout=subprocess.PIPE, text=True, env={**os.environ, 'WALLET': ABANDON, 'OUTSIDER': LEGAL})
        self.info = json.loads(self.process.stdout.readline())

    def accepted(self):
        """Every transaction the fixture mined, in order."""
        if not self.journal.exists():
            return []
        return [json.loads(line) for line in self.journal.read_text().splitlines()]

    def stop(self):
        self.process.kill()
        self.process.wait()
        self.process.stdout.close()


class Handler(http.server.BaseHTTPRequestHandler):
    """A loopback node's handler: quiet, reading and answering JSON. Its
    server's `state` is what `serve` was given."""

    def log_message(self, *_):
        pass

    @property
    def state(self):
        return self.server.state

    def raw_body(self):
        return self.rfile.read(int(self.headers.get('Content-Length') or 0))

    def body(self):
        raw = self.raw_body()
        return json.loads(raw) if raw else None

    def reply(self, payload, code=200):
        self.reply_bytes(json.dumps(payload).encode(), code, 'application/json')

    def reply_bytes(self, data, code=200, content_type='application/octet-stream'):
        self.send_response(code)
        self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)


class RpcError(Exception):
    """A JSON-RPC error answer."""

    def __init__(self, code, message):
        super().__init__(message)
        self.code = code
        self.message = message


class JsonRpc(Handler):
    """A JSON-RPC node: `call` answers one call, batches are answered call by
    call, and `RpcError` becomes an error object."""

    def call(self, method, params):
        raise NotImplementedError

    def do_POST(self):
        request = self.body()

        def answer(call):
            envelope = {'jsonrpc': '2.0', 'id': call.get('id')}
            try:
                return {**envelope, 'result': self.call(call['method'], call.get('params', []))}
            except RpcError as error:
                return {**envelope, 'error': {'code': error.code, 'message': error.message}}

        self.reply([answer(call) for call in request] if isinstance(request, list) else answer(request))


class Server(http.server.ThreadingHTTPServer):
    """A loopback node's server. A client that hangs up mid-request — the CLI
    abandoning a read it no longer needs — is not an error worth a traceback."""

    def handle_error(self, request, client_address):
        if not isinstance(sys.exc_info()[1], (BrokenPipeError, ConnectionResetError)):
            super().handle_error(request, client_address)


@contextlib.contextmanager
def serve(handler, **state):
    """`handler` on a loopback port for the block's length. The server's
    `url` names it; its `state` holds `state` for the handler to read and a
    suite to change."""
    server = Server(('127.0.0.1', 0), handler)
    server.state = state
    server.url = f'http://127.0.0.1:{server.server_port}'
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        yield server
    finally:
        server.shutdown()
        server.server_close()
        worker.join()


def unittest_main():
    """Run a unittest suite's tests, with any arguments after the binary."""
    import unittest
    unittest.main(argv=[sys.argv[0], *ARGS], verbosity=2)
