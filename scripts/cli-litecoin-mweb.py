#!/usr/bin/env python3
"""Litecoin MWEB funds against a loopback Litecoin node and indexer.

`spectra-litecoin-mweb-fixture` (tools/litecoin-mweb-fixture) serves a
synthetic chain whose first block holds forty thousand MWEB outputs, some
spent, among which real outputs pay the wallet's receive address and a later
one; transparent outputs pay its legacy and native SegWit addresses. It
serves light clients over the peer-to-peer protocol — headers, MWEB headers
proved through the block's HogEx, leafsets, and pages of unspent outputs with
the hashes that prove them, assembled as Litecoin Core assembles them — and
the wallet's indexer over Esplora's API. Its MWEB is written apart from
core's and its transactions decoded by the `litecoin` crate: every
transaction handed to it is checked as a node checks one — each signature and
range proof, both balances, the order of its parts, a fee no less than its
weight's, its canonical inputs' signatures and the peg-in rules of
`IsStandardTx` — before it is mined and journaled, with what it paid whom as
the wallet's and an outsider's keys read it.

The wallet scans for its MWEB funds, pays an MWEB address, pegs out to a
transparent one, pegs its transparent funds in, and pays an MWEB address from
transparent funds by a peg-in; a second data directory restored from the
same phrase, at another path, finds what is left. Refused: a scan with no
node, with the wrong password, of a key-only wallet, of a second wallet of the
same phrase, and of a node on another network; a payment past the balance,
to another network's MWEB address, or below a peg-out's dust threshold;
watching an MWEB address."""
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
PHRASE = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'
OUTSIDER = 'legal winner thank year wave sausage worth useful legal winner thank yellow'
INDEXER_CAPABILITIES = 'balance,history,utxo,fee,broadcast,verification'


def fixture_binary():
    """The fixture beside the CLI, or built where cargo builds it."""
    beside = pathlib.Path(binary).parent / 'spectra-litecoin-mweb-fixture'
    if beside.exists():
        return beside
    subprocess.run(['cargo', 'build', '-p', 'spectra_litecoin_mweb_fixture', '--quiet'], cwd=ROOT, check=True)
    return ROOT / 'target/debug/spectra-litecoin-mweb-fixture'


class Fixture:
    """One fixture process: its chain, its ports and what it journals."""

    def __init__(self, directory):
        self.journal = pathlib.Path(directory) / 'fixture.jsonl'
        self.process = subprocess.Popen(
            [str(fixture_binary()), '--wallet-phrase-env', 'WALLET', '--outsider-phrase-env', 'OUTSIDER',
             '--journal', str(self.journal)],
            stdout=subprocess.PIPE, text=True, env={**os.environ, 'WALLET': PHRASE, 'OUTSIDER': OUTSIDER})
        self.info = json.loads(self.process.stdout.readline())
        self.url = f"http://127.0.0.1:{self.info['http']}"
        self.node = f"tcp://127.0.0.1:{self.info['p2p']}"

    def accepted(self):
        """Every transaction the fixture mined, in order."""
        if not self.journal.exists():
            return []
        return [json.loads(line) for line in self.journal.read_text().splitlines()]

    def stop(self):
        self.process.kill()
        self.process.wait()
        self.process.stdout.close()


class MwebTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.scratch = tempfile.TemporaryDirectory(prefix='spectra-mweb-')
        cls.chain = Fixture(cls.scratch.name)

    @classmethod
    def tearDownClass(cls):
        cls.chain.stop()
        cls.scratch.cleanup()

    def setUp(self):
        self.directories = []

    def tearDown(self):
        for directory in self.directories:
            journal = pathlib.Path(directory.name) / 'network.jsonl'
            assert not journal.exists() or not journal.read_text().strip(), journal.read_text()
            directory.cleanup()

    def data_directory(self):
        directory = tempfile.TemporaryDirectory(prefix='spectra-mweb-data-')
        self.directories.append(directory)
        return directory.name

    def run_cli(self, directory, *args, success=True, env=None):
        p = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True,
                           timeout=600,
                           env={**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(pathlib.Path(directory) / 'network.jsonl'),
                                'SPECTRA_PASSWORD': 'fixture-password', 'SPECTRA_SEED': PHRASE, **(env or {})})
        assert (p.returncode == 0) == success, (args, p.returncode, p.stdout, p.stderr)
        return json.loads(p.stdout) if success else (p.returncode, p.stdout + p.stderr)

    def refuses(self, directory, words, *args, code=3, env=None):
        returned, output = self.run_cli(directory, *args, success=False, env=env)
        assert returned == code and words in output, (words, returned, output)

    def endpoints(self, directory, chain='litecoin', node=True):
        """The fixture's indexer, and its node unless `node` is false, as the
        network's only endpoints."""
        self.run_cli(directory, 'endpoints', '--chain', chain, '--custom-only', 'true')
        self.run_cli(directory, 'endpoints', '--chain', chain, '--api', 'esplora', '--capabilities',
                     INDEXER_CAPABILITIES, '--add', self.chain.url)
        if node:
            self.add_node(directory, chain)

    def add_node(self, directory, chain='litecoin'):
        self.run_cli(directory, 'endpoints', '--chain', chain, '--api', 'litecoin-p2p', '--capabilities',
                     'history', '--add', self.chain.node)

    def sync(self, directory, name='MWEB'):
        """Scan to the fixture's tip: each transaction it mined is a block."""
        status = self.run_cli(directory, 'wallet', 'mweb-sync', name)['mweb']
        tip = self.chain.info['tip'] + len(self.chain.accepted())
        assert status['complete'] and status['progressPermille'] == 1000, status
        assert status['scannedHeight'] == tip, (status, tip)
        return status

    def status(self, directory, name='MWEB'):
        return self.run_cli(directory, 'wallet', 'mweb-status', name)['mweb']

    def history(self, directory, name='MWEB'):
        rows = self.run_cli(directory, 'txs', '--page', '--wallet', name)['page']['records']
        return sorted((r['kind'], r['amount'], r['status'], r['address']) for r in rows)

    def send(self, directory, artifact):
        """Sign a built artifact as reviewed and hand it to the fixture."""
        before = len(self.chain.accepted())
        signed = self.run_cli(directory, 'send', 'sign', artifact['id'],
                              '--review-digest', artifact['review_digest'])['artifact']
        assert signed['stage'] == 'Signed' and len(signed['transaction_hash']) == 64, signed
        sent = self.run_cli(directory, 'send', 'broadcast-signed', artifact['id'],
                            '--endpoint', self.chain.url, '--yes')['artifact']
        assert [a['outcome'] for a in sent['attempts']] == ['Accepted'], sent['attempts']
        mined = self.chain.accepted()
        assert len(mined) == before + 1, mined
        assert mined[-1]['txid'] == signed['transaction_hash'], (mined[-1], signed)
        return mined[-1]

    def test_mweb_funds_move_and_are_recovered(self):
        data = self.data_directory()
        info = self.chain.info
        wallet, outsider = info['wallet'], info['outsider']

        # Without a node there is nothing to scan from.
        self.run_cli(data, 'wallet', 'import', '--chain', 'litecoin', '--name', 'MWEB')
        self.endpoints(data, node=False)
        self.refuses(data, 'needs a Litecoin node', 'wallet', 'mweb-sync', 'MWEB')
        self.add_node(data)
        offers = [o['action'] for o in self.run_cli(data, 'wallet', 'actions', 'MWEB')['actions']['actions']]
        assert 'mwebFunds' in offers, offers
        status = self.status(data)
        assert (status['ready'], status['address'], status['spendable']) == (False, None, '0'), status
        for build in (['send-mweb', 'MWEB', '--to', outsider['mweb'], '--amount', '0.1'],
                      ['mweb-pegin', 'MWEB', '--amount', '0.1']):
            self.refuses(data, 'Sync the MWEB funds', 'wallet', *build)

        # The first batch derives the keys from the seed: a wrong password
        # derives nothing.
        self.refuses(data, 'password', 'wallet', 'mweb-sync', 'MWEB', env={'SPECTRA_PASSWORD': 'wrong'})
        assert not self.status(data)['ready']

        # One batch reads eight pages, less than the unspent set; the rest
        # follow, and the wallet's outputs are among them, at the address
        # the fixture derived.
        status = self.run_cli(data, 'wallet', 'mweb-sync', 'MWEB', '--once')['mweb']
        assert status['ready'] and not status['complete'] and 0 < status['progressPermille'] < 1000, status
        status = self.sync(data)
        assert (status['spendable'], status['pending'], status['address']) == ('1.85', '0', wallet['mweb']), status
        assert self.history(data) == [('receive', '0.1', 'confirmed', ''), ('receive', '0.25', 'confirmed', ''),
                                      ('receive', '1.5', 'confirmed', '')], self.history(data)
        assert self.run_cli(data, 'wallet', 'show', 'MWEB')['wallet']['address'] == wallet['legacy']
        # The balance is the transparent output and the MWEB funds.
        assert self.run_cli(data, 'balance', 'MWEB')['amount'] == '2.35'

        # A payment inside MWEB: the largest output, change to the address
        # kept for it, and its weight's fee.
        built = self.run_cli(data, 'wallet', 'send-mweb', 'MWEB', '--to', outsider['mweb'],
                             '--amount', '0.4')['artifact']
        assert (built['recipient'], built['amount'], built['operation']) == \
            (outsider['mweb'], '0.4', {'kind': 'shielded_payment', 'memo': None, 'network_fee': '0.000039'}), built
        assert [w['code'] for w in built['review']['warnings']] == ['new_address'], built['review']
        prepared = json.loads(built['prepared_details'])['LitecoinMweb']
        assert ([i['value'] for i in prepared['inputs']], prepared['change'], prepared['fee']) == \
            ([150_000_000], 109_996_100, 3_900), prepared
        mined = self.send(data, built)
        assert (mined['canonical_fee'], mined['kernel_fee'], mined['mweb_inputs'], mined['pegouts']) == \
            (0, 3_900, 1, []), mined
        assert sorted(mined['mweb_outputs'], key=lambda o: o['owner']) == [
            {'owner': 'outsider', 'index': 2, 'value': 40_000_000},
            {'owner': 'wallet', 'index': 0, 'value': 109_996_100}], mined
        # Until a scan sees it mined, its input is spoken for and its change
        # on the way.
        status = self.status(data)
        assert (status['spendable'], status['pending']) == ('0.35', '1.099961'), status
        status = self.sync(data)
        assert (status['spendable'], status['pending']) == ('1.449961', '0'), status
        assert ('send', '0.4', 'confirmed', outsider['mweb']) in self.history(data), self.history(data)

        # A peg-out: the amount leaves by the kernel to a transparent
        # address, with its bytes' fee beside the weight's.
        built = self.run_cli(data, 'wallet', 'send-mweb', 'MWEB', '--to', outsider['legacy'],
                             '--amount', '0.1')['artifact']
        assert built['operation']['network_fee'] == '0.0000254', built['operation']
        mined = self.send(data, built)
        assert (mined['kernel_fee'], mined['pegouts']) == \
            (2_540, [{'address': outsider['legacy'], 'value': 10_000_000}]), mined
        assert mined['mweb_outputs'] == [{'owner': 'wallet', 'index': 0, 'value': 99_993_560}], mined
        assert self.sync(data)['spendable'] == '1.3499356'

        # A peg-in from the wallet's transparent output to the address it
        # keeps for them: the canonical output names the kernel and carries
        # its fee.
        built = self.run_cli(data, 'wallet', 'mweb-pegin', 'MWEB', '--amount', '0.2')['artifact']
        assert (built['recipient'], built['amount'], built['operation']) == \
            (wallet['pegin'], '0.2', {'kind': 'shield_transparent', 'amount': '0.2', 'network_fee': '0.000031'}), \
            built
        mined = self.send(data, built)
        assert (mined['canonical_fee'], mined['kernel_fee'], mined['pegin']) == (1_000, 2_100, 20_002_100), mined
        assert mined['mweb_outputs'] == [{'owner': 'wallet', 'index': 1, 'value': 20_000_000}], mined
        assert {'address': wallet['legacy'], 'value': 29_996_900} in mined['canonical_outputs'], mined
        assert self.sync(data)['spendable'] == '1.5499356'
        # The indexer's row of it confirms the wallet's record of the peg-in.
        self.run_cli(data, 'history', 'MWEB', '--save')
        assert ('shield', '0.2', 'confirmed', wallet['pegin']) in self.history(data), self.history(data)
        assert self.run_cli(data, 'balance', 'MWEB')['amount'] == '1.8499046'

        # From transparent funds, an MWEB address is paid by a peg-in to it.
        built = self.run_cli(data, 'send', 'build', '--from', 'MWEB', '--to', outsider['mweb'],
                             '--amount', '0.05', '--endpoint', self.chain.url)['artifact']
        prepared = json.loads(built['prepared_details'])['LitecoinPegIn']
        assert (prepared['recipient'], prepared['amount'], prepared['mweb_fee']) == \
            (outsider['mweb'], 5_000_000, 2_100), prepared
        mined = self.send(data, built)
        assert mined['mweb_outputs'] == [{'owner': 'outsider', 'index': 2, 'value': 5_000_000}], mined

        # Refused before anything is signed: more than the balance, another
        # network's MWEB address, dust to a transparent one, more than the
        # transparent funds into MWEB.
        self.refuses(data, 'cannot cover', 'wallet', 'send-mweb', 'MWEB', '--to', outsider['mweb'],
                     '--amount', '5')
        self.refuses(data, 'Not a Litecoin address on this network', 'wallet', 'send-mweb', 'MWEB', '--to',
                     outsider['other_network_mweb'], '--amount', '0.1')
        self.refuses(data, 'dust threshold', 'wallet', 'send-mweb', 'MWEB', '--to', outsider['legacy'],
                     '--amount', '0.00001')
        returned, output = self.run_cli(data, 'wallet', 'mweb-pegin', 'MWEB', '--amount', '5', success=False)
        assert returned != 0 and 'nsufficient' in output, (returned, output)

        # Another device, restored from the phrase at another path, finds
        # what is left: the outputs received at the addresses the wallet
        # gives out, its change and its peg-in.
        recovered = self.data_directory()
        self.run_cli(recovered, 'wallet', 'import', '--chain', 'litecoin', '--name', 'Restored',
                     '--profile', 'nativeSegWit')
        self.endpoints(recovered)
        status = self.sync(recovered, 'Restored')
        assert (status['spendable'], status['address']) == ('1.5499356', wallet['mweb']), status
        assert self.history(recovered, 'Restored') == [('receive', '0.1', 'confirmed', ''),
                                                       ('receive', '0.25', 'confirmed', '')], \
            self.history(recovered, 'Restored')
        # One wallet of a phrase holds its MWEB funds.
        self.run_cli(recovered, 'wallet', 'import', '--chain', 'litecoin', '--name', 'Again')
        self.refuses(recovered, 'already holds this phrase', 'wallet', 'mweb-sync', 'Again')

    def test_what_holds_no_mweb_funds_is_refused(self):
        data = self.data_directory()
        outsider = self.chain.info['outsider']
        # A key-only wallet has no phrase to derive MWEB keys from.
        self.run_cli(data, 'wallet', 'import', '--chain', 'litecoin', '--name', 'Key',
                     '--private-key-env', 'KEY', env={'KEY': '11' * 32})
        self.endpoints(data)
        offers = [o['action'] for o in self.run_cli(data, 'wallet', 'actions', 'Key')['actions']['actions']]
        assert 'mwebFunds' not in offers, offers
        self.refuses(data, 'restored from its seed phrase', 'wallet', 'mweb-sync', 'Key')
        # An MWEB address shows nothing anyone could watch.
        self.refuses(data, 'valid address', 'wallet', 'watch', '--chain', 'litecoin', '--address', outsider['mweb'])
        # A node on another network does not finish the handshake.
        self.run_cli(data, 'wallet', 'import', '--chain', 'litecoin-testnet', '--name', 'Testnet')
        self.endpoints(data, chain='litecoin-testnet')
        returned, output = self.run_cli(data, 'wallet', 'mweb-sync', 'Testnet', success=False)
        assert returned == 1 and 'Litecoin node' in output, (returned, output)
        assert not self.status(data, 'Testnet')['ready']


if __name__ == '__main__':
    unittest.main(argv=sys.argv[:1], verbosity=2)
