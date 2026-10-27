"""Drives the real `spectra` binary end to end against throwaway data
directories: `python3 -B tests/acceptance [path/to/spectra]`.

Rules are tested in core with `cargo test`. This covers what only the built
binary shows: state that crosses a process boundary, what lands on disk (the
SQLite store and the file secret store), the exit-code contract, the CLI's own
confirmation gates, the staged sends and staking that the suites beside this
file drive through loopback nodes, and the network guard.

No external network. Core enforces this: `SPECTRA_LOOPBACK_ONLY` refuses and
journals every request to a non-loopback host, and any journaled request
fails the run.
"""
import json
import os
import pathlib
import shutil
import sqlite3
import subprocess
import sys
import tempfile

from harness import LEGAL, ROOT

HERE = pathlib.Path(__file__).resolve().parent

# Exit codes are part of the interface: 0 done, 1 failed, 2 the caller asked
# wrongly, 3 core considered it and said no.
OK, FAILED, USAGE, REJECTED = 0, 1, 2, 3

# Each suite owns its data directories and its loopback nodes, so they all run
# at once, beside the checks below, and the slowest sets the pace instead of
# their sum.
SUITES = {
    'send_stages': 'staged EVM sends across processes',
    'send_polkadot': 'Polkadot Asset Hub staged sends and finalized outcomes',
    'send_cardano': 'Cardano staged send broadcast once as signed',
    'multisig_psbt': 'a 2-of-3 Bitcoin PSBT across three data directories',
    'multisig_safe': 'Safe owners sign, join and execute a Safe transaction',
    'multisig_tron': 'Tron active-permission spends signed in two data directories',
    'multisig_xrp': 'XRP signer-list payments signed across data directories',
    'multisig_stellar': 'Stellar payments signed across data directories',
    'multisig_aptos': 'Aptos MultiKey transfers signed across data directories',
    'multisig_cardano': 'Cardano native scripts witnessed by CIP-1854 keys',
    'multisig_substrate': 'Asset Hub pallet-multisig approvals execute a transfer',
    'multisig_ton': 'TON multisig v2 orders proposed and approved by signers',
    'wallet_operations': 'wallet operations: closing accounts, deleting keys',
    'issued_assets': 'XRP Ledger and Stellar trust lines opened, paid and removed',
    'nfts': 'ERC-721 and ERC-1155 transfers broadcast as signed',
    'zcash_shielded': 'Zcash shielded scanning, transfers and recovery',
    'litecoin_mweb': 'Litecoin MWEB scanning, peg-ins, peg-outs and recovery',
    'trc10': 'TRC-10 staged send, broadcast and confirmation',
    'staking': 'staking signed, retried and recovered across processes',
    'icp_staking': 'ICP neuron staking reopened and refused on a forged certificate',
    'send_near': 'NEAR durable retries and final status recovery',
    'send': 'a password-protected owned send',
    'send_tokens': 'a TON jetton send resolved from its trace across processes',
    'send_icp_zcash': 'ICP and transparent Zcash sends with an identical retry',
    'endpoints': 'the endpoint listing and health verdict',
}


class Run:
    """Counts what passed and failed, and prints each check as it lands."""

    def __init__(self, binary, data):
        self.binary = binary
        self.data = data
        self.passed = 0
        self.failed = 0

    @staticmethod
    def section(title):
        print(f'\n\033[1m{title}\033[0m')

    def verdict(self, description, ok, detail=''):
        if ok:
            self.passed += 1
            print(f'  \033[32m✓\033[0m {description}')
        else:
            self.failed += 1
            print(f'  \033[31m✗\033[0m {description}')
            if detail:
                print('    ' + detail.strip().replace('\n', '\n    '))

    def spectra(self, *args, data=None, **env):
        """Run the binary against `data` (the shared directory unless named);
        its exit status and its stdout and stderr together."""
        result = subprocess.run(
            [str(self.binary), '--data-dir', str(data or self.data), *args],
            capture_output=True, text=True, timeout=120, env={**os.environ, **env})
        return result.returncode, result.stdout + result.stderr

    def check(self, description, expected, *args, contains=None, data=None, **env):
        """The command exits `expected`, and says `contains` when given."""
        status, output = self.spectra(*args, data=data, **env)
        ok = status == expected and (contains is None or contains in output)
        wanted = f'exit {expected}' + (f' saying {contains!r}' if contains else '')
        self.verdict(description, ok, f'exit {status}, wanted {wanted}\n{output}')
        return output

    def holds(self, description, ok, detail=''):
        self.verdict(description, ok, detail)


def build():
    """The CLI, and the fixture programs the Zcash and MWEB suites run beside it."""
    subprocess.run(
        ['cargo', 'build', '-p', 'spectra_cli', '-p', 'spectra_zcash_fixture',
         '-p', 'spectra_litecoin_mweb_fixture', '--quiet'],
        cwd=ROOT, check=True)
    return ROOT / 'target/debug/spectra'


# Run by hand only: a real daemon, and the driver's own helpers.
NOT_SUITES = {'monero_regtest', 'harness', '__main__'}


def start_suites(binary, logs):
    """Every suite, started at once; each writes its output to its own log."""
    present = {path.stem for path in HERE.glob('*.py')} - NOT_SUITES
    assert present == set(SUITES), f'suites not listed or listed but missing: {present ^ set(SUITES)}'
    running = {}
    for name in SUITES:
        log = open(logs / f'{name}.log', 'w')
        running[name] = (subprocess.Popen(
            [sys.executable, '-B', str(HERE / f'{name}.py'), str(binary)],
            stdout=log, stderr=subprocess.STDOUT), log)
    return running


def wallet_id(run, name):
    status, output = run.spectra('--json', 'wallet', 'show', name)
    assert status == OK, output
    return json.loads(output)['wallet']['id']


def checks(run):
    run.section('exit codes')
    run.check('a valid request succeeds', OK,
              'address', 'validate', '--chain', 'Bitcoin', 'bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4')
    run.check('an unknown wallet fails', FAILED, 'wallet', 'show', 'no such wallet')
    run.check('an unknown option value is a usage error', USAGE, 'chains', '--tag', 'sidechain')
    run.check('a ticker is not a network name', USAGE, 'token', 'catalog', '--chain', 'ETH')
    run.check('a refusal by core is rejected', REJECTED, 'address', 'validate', '--chain', 'Ethereum', '0xnothex')
    run.check('every command reports in text', OK, 'wallet', 'list', contains='no wallets yet')

    run.section('wallets across processes')
    run.check('imports a known mnemonic', OK,
              'wallet', 'import', '--chain', 'Solana', '--name', 'Acceptance SOL', SPECTRA_SEED=LEGAL)
    run.check('a new process reads back its derived address', OK,
              '--json', 'wallet', 'show', 'Acceptance SOL', contains='BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX')
    run.check('an asset the wallet does not hold cannot be hidden', REJECTED,
              'wallet', 'hide', 'Acceptance SOL', 'DAI')
    run.check('creates a wallet', OK, 'wallet', 'new', '--chain', 'Bitcoin', '--name', 'Acceptance BTC')

    # A store a build cannot read is refused, never rewritten, until the user
    # discards it. Discarding runs on a copy: the store and secrets are still
    # needed below.
    run.section('an unreadable store')
    with tempfile.TemporaryDirectory() as copy:
        shutil.copytree(run.data, copy, dirs_exist_ok=True)
        fresh = '"derivationOverrides":{"passphrase":null,"hmacKey":null}'
        stale = '"derivationOverrides":{"passphrase":null,"mnemonicWordlist":null,"hmacKey":null}'
        with sqlite3.connect(pathlib.Path(copy) / 'spectra.sqlite') as store:
            store.execute('UPDATE wallets SET payload = REPLACE(payload, ?, ?)', (fresh, stale))
        run.check('an unreadable store names the way out', FAILED,
                  'wallet', 'list', data=copy, contains='settings discard --yes')
        run.check('discarding it asks for --yes', USAGE, 'settings', 'discard', data=copy)
        run.check('discards it', OK, 'settings', 'discard', '--yes', data=copy)
        run.check('and starts an empty store', OK, 'wallet', 'list', data=copy, contains='no wallets yet')

    run.section('sealed secrets')
    run.check('export returns the phrase it sealed', OK,
              '--json', 'wallet', 'export', 'Acceptance SOL', '--yes', contains=f'"seedPhrase":"{LEGAL}"')
    run.check('a wrong password cannot unseal it', REJECTED,
              'wallet', 'export', 'Acceptance SOL', '--yes', SPECTRA_PASSWORD='wrong')
    run.check('a key export asks for --yes', USAGE, 'wallet', 'export', 'Acceptance SOL', '--key', 'private-key')
    run.check('imports without a password', OK,
              'wallet', 'import', '--chain', 'Solana', '--account', '1', '--name', 'Open SOL', '--no-password',
              SPECTRA_SEED=LEGAL)
    # No password is no plaintext: core seals the phrase under its device key,
    # so neither the words nor their base64 are in the seed bucket.
    secrets = run.data / 'secrets'
    sealed = [path.read_bytes() for path in (secrets / 'seed').iterdir()]
    run.holds('every phrase is stored sealed',
              any((secrets / 'device_key').iterdir())
              and not any(b'legal winner' in raw or b'bGVnYWwgd2lubmVy' in raw for raw in sealed))
    run.check('a whitespace-only password is refused', REJECTED,
              'wallet', 'import', '--chain', 'Solana', '--name', 'Blank SOL',
              SPECTRA_PASSWORD='   ', SPECTRA_SEED=LEGAL)
    run.check('and so is an empty password file', REJECTED,
              'wallet', 'new', '--chain', 'Solana', '--name', 'Blank SOL', '--password-file', '/dev/null')

    run.section('private-key wallets')
    key_file = run.data / 'pk.hex'
    key_file.write_text('4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318\n')
    run.check('imports a key from a file', OK,
              'wallet', 'import', '--chain', 'Ethereum', '--name', 'PK Wallet', '--private-key-file', str(key_file),
              SPECTRA_PASSWORD='correct horse')

    # Commands that disclose a secret, move funds or destroy state need --yes.
    run.section('confirmation gates')
    run.check('export will not print a seed without --yes', USAGE, 'wallet', 'export', 'Acceptance BTC')
    run.check('a broadcast without --yes is refused', USAGE,
              'send', 'owned-broadcast', 'Acceptance BTC', '--holding', 'bitcoin:native',
              '--amount', '0.001', '--destination', 'bc1qgkju4yvvtuz0s8vqn837q396jezu2h8ex7gk98')
    run.check('delete will not run without --yes', USAGE, 'wallet', 'delete', 'Acceptance BTC')
    run.check('reset will not run without --yes', USAGE, 'settings', 'reset')

    # Checked on disk rather than through `export`, which stops at "no such
    # wallet" before it reaches the secret store: a wallet row can go while
    # its sealed material stays behind, and that is the leak worth asserting
    # against. The file store percent-encodes `.`, so the key `<id>.seed` is
    # `seed/<id>%2Eseed`. Each file must exist before deletion, or its absence
    # afterwards proves nothing about where the check looked.
    run.section('deletion')
    seed_file = secrets / 'seed' / f'{wallet_id(run, "Acceptance BTC")}%2Eseed'
    key_secret = secrets / 'private_key' / f'{wallet_id(run, "PK Wallet")}%2Eprivatekey'
    run.holds('the sealed seed is on disk', seed_file.is_file())
    run.holds('the sealed key is on disk', key_secret.is_file())
    run.check('deletes the seed wallet', OK, 'wallet', 'delete', 'Acceptance BTC', '--yes')
    run.check('deletes the key wallet', OK, 'wallet', 'delete', 'PK Wallet', '--yes')
    run.check('the deleted wallet is gone', FAILED, 'wallet', 'show', 'Acceptance BTC')
    run.holds('its sealed seed went with it', not seed_file.exists())
    run.holds('its sealed key went with it', not key_secret.exists())


def main():
    binary = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else build()
    with tempfile.TemporaryDirectory() as scratch:
        scratch = pathlib.Path(scratch)
        data, logs = scratch / 'data', scratch / 'logs'
        data.mkdir()
        logs.mkdir()
        # The harness confined this process to loopback when it loaded; every
        # process started here, the suites' included, inherits the journal.
        journal = pathlib.Path(os.environ['SPECTRA_LOOPBACK_ONLY'])
        canary = scratch / 'canary.journal'
        # Wallets created here are throwaway, so the password is too.
        os.environ['SPECTRA_PASSWORD'] = 'acceptance-password'

        running = start_suites(binary, logs)
        run = Run(binary, data)
        checks(run)

        run.section('loopback suites')
        for name, (process, log) in running.items():
            status = process.wait()
            log.close()
            run.verdict(SUITES[name], status == 0, (logs / f'{name}.log').read_text())

        run.section('no external network')
        # The guard must catch a real attempt, or an empty journal proves
        # nothing. Health probes call every built-in provider directly, so
        # this one is refused.
        run.check('a probe of remote providers still exits', OK,
                  '--json', 'endpoints', '--chain', 'bitcoin', SPECTRA_LOOPBACK_ONLY=str(canary))
        refused = canary.read_text() if canary.exists() else ''
        run.holds('and the refusal is journaled by host', 'https://blockstream.info/' in refused, refused)
        reached = journal.read_text()
        run.holds('no command reached beyond loopback', not reached, f'refused, then journaled:\n{reached}')

    print()
    if run.failed:
        print(f'\033[31m{run.failed} failed\033[0m, {run.passed} passed')
        sys.exit(1)
    print(f'\033[32m{run.passed} passed\033[0m')


main()
