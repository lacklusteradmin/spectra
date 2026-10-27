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

The wallet scans for its MWEB funds over several processes, pays an MWEB
address, pegs out to a transparent one and pegs its transparent funds in; a
second data directory restored from the same phrase, at another path, finds
what is left. A testnet wallet refuses a mainnet node. What a payment or a
scan refuses before any node is read is tested in core."""
import json
import unittest

from harness import ABANDON, ChainFixture, Spectra, scratch, unittest_main

INDEXER_CAPABILITIES = 'balance,history,utxo,fee,broadcast,verification'


class Fixture(ChainFixture):
    """The Litecoin node and indexer fixture: its Esplora and its peer port."""

    def __init__(self, directory):
        super().__init__('spectra-litecoin-mweb-fixture', directory)
        self.url = f"http://127.0.0.1:{self.info['http']}"
        self.node = f"tcp://127.0.0.1:{self.info['p2p']}"


class MwebTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.chain = Fixture(cls.enterClassContext(scratch('mweb')))
        cls.addClassCleanup(cls.chain.stop)

    def setUp(self):
        self.spectra = Spectra(self.enterContext(scratch('mweb-data')),
                               SPECTRA_PASSWORD='fixture-password', SPECTRA_SEED=ABANDON)

    def endpoints(self, device, chain='litecoin'):
        """The fixture's indexer and node as the network's only endpoints."""
        device.run('endpoints', '--chain', chain, '--custom-only', 'true')
        device.run('endpoints', '--chain', chain, '--api', 'esplora', '--capabilities',
                   INDEXER_CAPABILITIES, '--add', self.chain.url)
        device.run('endpoints', '--chain', chain, '--api', 'litecoin-p2p', '--capabilities',
                   'history', '--add', self.chain.node)

    def sync(self, device, name='MWEB'):
        """Scan to the fixture's tip: each transaction it mined is a block."""
        status = device.run('wallet', 'mweb-sync', name)['mweb']
        tip = self.chain.info['tip'] + len(self.chain.accepted())
        assert status['complete'] and status['progressPermille'] == 1000, status
        assert status['scannedHeight'] == tip, (status, tip)
        return status

    def status(self, device, name='MWEB'):
        return device.run('wallet', 'mweb-status', name)['mweb']

    def history(self, device, name='MWEB'):
        rows = device.run('txs', '--page', '--wallet', name)['page']['records']
        return sorted((r['kind'], r['amount'], r['status'], r['address']) for r in rows)

    def send(self, device, artifact):
        """Sign a built artifact as reviewed and hand it to the fixture."""
        before = len(self.chain.accepted())
        signed = device.run('send', 'sign', artifact['id'], '--review-digest', artifact['review_digest'])['artifact']
        assert signed['stage'] == 'Signed' and len(signed['transaction_hash']) == 64, signed
        sent = device.run('send', 'broadcast-signed', artifact['id'], '--endpoint', self.chain.url,
                          '--yes')['artifact']
        assert [a['outcome'] for a in sent['attempts']] == ['Accepted'], sent['attempts']
        mined = self.chain.accepted()
        assert len(mined) == before + 1, mined
        assert mined[-1]['txid'] == signed['transaction_hash'], (mined[-1], signed)
        return mined[-1]

    def test_mweb_funds_move_and_are_recovered(self):
        data = self.spectra / 'wallet'
        info = self.chain.info
        wallet, outsider = info['wallet'], info['outsider']

        data.run('wallet', 'import', '--chain', 'litecoin', '--name', 'MWEB')
        self.endpoints(data)

        # One batch reads eight pages, less than the unspent set; the rest
        # follow, and the wallet's outputs are among them, at the address
        # the fixture derived.
        status = data.run('wallet', 'mweb-sync', 'MWEB', '--once')['mweb']
        assert status['ready'] and not status['complete'] and 0 < status['progressPermille'] < 1000, status
        status = self.sync(data)
        assert (status['spendable'], status['pending'], status['address']) == ('1.85', '0', wallet['mweb']), status
        assert self.history(data) == [('receive', '0.1', 'confirmed', ''), ('receive', '0.25', 'confirmed', ''),
                                      ('receive', '1.5', 'confirmed', '')], self.history(data)
        assert data.run('wallet', 'show', 'MWEB')['wallet']['address'] == wallet['legacy']
        # The balance is the transparent output and the MWEB funds.
        assert data.run('balance', 'MWEB')['amount'] == '2.35'

        # A payment inside MWEB: the largest output, change to the address
        # kept for it, and its weight's fee.
        built = data.run('wallet', 'send-mweb', 'MWEB', '--to', outsider['mweb'], '--amount', '0.4')['artifact']
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
        built = data.run('wallet', 'send-mweb', 'MWEB', '--to', outsider['legacy'], '--amount', '0.1')['artifact']
        assert built['operation']['network_fee'] == '0.0000254', built['operation']
        mined = self.send(data, built)
        assert (mined['kernel_fee'], mined['pegouts']) == \
            (2_540, [{'address': outsider['legacy'], 'value': 10_000_000}]), mined
        assert mined['mweb_outputs'] == [{'owner': 'wallet', 'index': 0, 'value': 99_993_560}], mined
        assert self.sync(data)['spendable'] == '1.3499356'

        # A peg-in from the wallet's transparent output to the address it
        # keeps for them: the canonical output names the kernel and carries
        # its fee.
        built = data.run('wallet', 'mweb-pegin', 'MWEB', '--amount', '0.2')['artifact']
        assert (built['recipient'], built['amount'], built['operation']) == \
            (wallet['pegin'], '0.2', {'kind': 'shield_transparent', 'amount': '0.2', 'network_fee': '0.000031'}), \
            built
        mined = self.send(data, built)
        assert (mined['canonical_fee'], mined['kernel_fee'], mined['pegin']) == (1_000, 2_100, 20_002_100), mined
        assert mined['mweb_outputs'] == [{'owner': 'wallet', 'index': 1, 'value': 20_000_000}], mined
        assert {'address': wallet['legacy'], 'value': 29_996_900} in mined['canonical_outputs'], mined
        assert self.sync(data)['spendable'] == '1.5499356'
        # The indexer's row of it confirms the wallet's record of the peg-in.
        data.run('history', 'MWEB', '--save')
        assert ('shield', '0.2', 'confirmed', wallet['pegin']) in self.history(data), self.history(data)
        assert data.run('balance', 'MWEB')['amount'] == '1.8499046'

        # Another device, restored from the phrase at another path, finds
        # what is left: the outputs received at the addresses the wallet
        # gives out, its change and its peg-in.
        recovered = self.spectra / 'restored'
        recovered.run('wallet', 'import', '--chain', 'litecoin', '--name', 'Restored', '--profile', 'nativeSegWit')
        self.endpoints(recovered)
        status = self.sync(recovered, 'Restored')
        assert (status['spendable'], status['address']) == ('1.5499356', wallet['mweb']), status
        assert self.history(recovered, 'Restored') == [('receive', '0.1', 'confirmed', ''),
                                                       ('receive', '0.25', 'confirmed', '')], \
            self.history(recovered, 'Restored')

    def test_a_node_on_another_network_is_refused(self):
        # A testnet wallet's handshake with the mainnet node does not finish.
        self.spectra.run('wallet', 'import', '--chain', 'litecoin-testnet', '--name', 'Testnet')
        self.endpoints(self.spectra, chain='litecoin-testnet')
        refused = self.spectra.fail('wallet', 'mweb-sync', 'Testnet', code=1)
        # The node drops the handshake; the reset's wording is the OS's.
        assert 'Litecoin node:' in refused['error'], refused
        assert not self.status(self.spectra, 'Testnet')['ready']


if __name__ == '__main__':
    unittest_main()
