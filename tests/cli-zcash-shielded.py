#!/usr/bin/env python3
"""Zcash shielded funds against a loopback lightwalletd.

`spectra-zcash-fixture` (tools/zcash-fixture) serves a synthetic chain that
funds the wallet with real transactions — an Ironwood note with a memo, a
Sapling note and a transparent output — and checks each transaction it is
handed as a node would before mining it: the consensus branch and expiry,
transparent signatures over ZIP-244, Orchard-family anchors, nullifiers,
proofs and signatures, and ZIP-317's fee. It journals what each one paid
whom, decrypted with the wallet's and the recipient's viewing keys.

The wallet scans from its restore height, shields its transparent funds,
pays a shielded address with a memo and a transparent one, and a second data
directory restored from the same phrase finds the same funds and history.
Spending the Sapling note needs the published Sapling parameters, which only
download.z.cash serves: that is the one request beyond loopback, and it is
refused. A restore height past the server's tip scans nothing. What a
payment or a sync refuses before any network is read is tested in core."""
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
START = 3_430_000
# The guard journals the origin it refused.
PARAMETERS = 'https://download.z.cash/'


def fixture_binary():
    """The fixture beside the CLI, or built where cargo builds it."""
    beside = pathlib.Path(binary).parent / 'spectra-zcash-fixture'
    if beside.exists():
        return beside
    subprocess.run(['cargo', 'build', '-p', 'spectra_zcash_fixture', '--quiet'], cwd=ROOT, check=True)
    return ROOT / 'target/debug/spectra-zcash-fixture'


class Fixture:
    """One fixture process: its chain, its port and what it journals."""

    def __init__(self, directory):
        self.journal = pathlib.Path(directory) / 'fixture.jsonl'
        self.process = subprocess.Popen(
            [str(fixture_binary()), '--wallet-phrase-env', 'WALLET', '--outsider-phrase-env', 'OUTSIDER',
             '--start', str(START), '--blocks', '30', '--journal', str(self.journal)],
            stdout=subprocess.PIPE, text=True, env={**os.environ, 'WALLET': PHRASE, 'OUTSIDER': OUTSIDER})
        self.info = json.loads(self.process.stdout.readline())
        self.url = f"http://127.0.0.1:{self.info['port']}"

    def accepted(self):
        """Every transaction the fixture mined, in order."""
        if not self.journal.exists():
            return []
        return [json.loads(line) for line in self.journal.read_text().splitlines()]

    def stop(self):
        self.process.kill()
        self.process.wait()
        self.process.stdout.close()


class ShieldedTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.scratch = tempfile.TemporaryDirectory(prefix='spectra-zcash-')
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
        directory = tempfile.TemporaryDirectory(prefix='spectra-zcash-data-')
        self.directories.append(directory)
        return directory.name

    def run_cli(self, directory, *args, success=True):
        # Its own journal: the refused parameters download is expected in it,
        # and each test asserts it is empty otherwise.
        p = subprocess.run([binary, '--data-dir', directory, '--json', *args], capture_output=True, text=True,
                           timeout=600,
                           env={**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(pathlib.Path(directory) / 'network.jsonl'),
                                'SPECTRA_PASSWORD': 'fixture-password', 'SPECTRA_SEED': PHRASE})
        assert (p.returncode == 0) == success, (args, p.returncode, p.stdout, p.stderr)
        return json.loads(p.stdout) if success else (p.returncode, p.stdout + p.stderr)

    def refuses(self, directory, words, *args, code=3):
        returned, output = self.run_cli(directory, *args, success=False)
        assert returned == code and words in output, (words, returned, output)

    def wallet(self, directory, name='Shielded', restore_height=START):
        """A Zcash wallet restored from the phrase, and the server it scans."""
        self.run_cli(directory, 'wallet', 'import', '--chain', 'zcash', '--name', name,
                     '--restore-height', str(restore_height))
        self.run_cli(directory, 'endpoints', '--chain', 'zcash', '--custom-only', 'true')
        self.run_cli(directory, 'endpoints', '--chain', 'zcash', '--api', 'lightwalletd', '--capabilities',
                     'history,broadcast,verification', '--add', self.chain.url)

    def sync(self, directory, name='Shielded'):
        status = self.run_cli(directory, 'wallet', 'zcash-sync', name)['shielded']
        assert status['complete'] and status['scannedHeight'] == status['chainTipHeight'], status
        return status

    def history(self, directory, name='Shielded'):
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

    def test_shielded_funds_move_and_are_recovered(self):
        data = self.data_directory()
        info = self.chain.info
        assert info['pool'] == 'ironwood', info
        self.wallet(data)

        # The scan finds the Ironwood and Sapling notes, and the transparent
        # output to shield, at the address the fixture derived.
        status = self.sync(data)
        assert (status['spendable'], status['pending'], status['shieldable']) == ('1.75', '0', '0.5'), status
        assert status['address'] == info['wallet_address'], (status, info)
        assert self.history(data) == [('receive', '0.25', 'confirmed', ''),
                                      ('receive', '1.5', 'confirmed', '')], self.history(data)
        wallet = self.run_cli(data, 'wallet', 'show', 'Shielded')['wallet']
        assert wallet['address'] == info['wallet_transparent'], (wallet, info)

        # Shielding: every transparent output into the wallet's own
        # Ironwood pool, less ZIP-317's fee, paying no one.
        built = self.run_cli(data, 'wallet', 'shield', 'Shielded')['artifact']
        assert built['operation'] == {'kind': 'shield_transparent', 'amount': '0.49985',
                                      'network_fee': '0.00015'}, built['operation']
        assert (built['recipient'], built['amount']) == (info['wallet_address'], '0.49985'), built
        mined = self.send(data, built)
        assert mined['transparent_inputs'] == [{'address': info['wallet_transparent'], 'value': 50_000_000}], mined
        assert mined['transparent_outputs'] == [] and mined['fee'] == 15_000, mined
        assert [(p['pool'], p['account'], p['value'], p['transfer']) for p in mined['paid']] == \
            [('ironwood', 'wallet', 49_985_000, 'change')], mined['paid']
        status = self.sync(data)
        assert (status['spendable'], status['shieldable']) == ('2.24985', '0'), status
        # The record of the shielding names where it went, and the scan's
        # row of it confirms that record.
        assert ('shield', '0.49985', 'confirmed', info['wallet_address']) in self.history(data), \
            self.history(data)

        # A shielded payment carries its memo to the recipient, who alone
        # reads it; the review shows it and the fee as signed.
        memo = 'thanks from spectra'
        built = self.run_cli(data, 'wallet', 'send-shielded', 'Shielded', '--to', info['outsider_address'],
                             '--amount', '0.3', '--memo', memo)['artifact']
        assert (built['recipient'], built['amount'], built['operation']) == \
            (info['outsider_address'], '0.3', {'kind': 'shielded_payment', 'memo': memo, 'network_fee': '0.0001'}), \
            built
        # Reviewed as any send: never paid before, and a valid address.
        assert [w['code'] for w in built['review']['warnings']] == ['new_address'], built['review']
        prepared = json.loads(built['prepared_details'])['ZcashShielded']
        assert (prepared['uses_sapling'], prepared['transparent_in_zat'], prepared['fee_zat']) == \
            (False, 0, 10_000), prepared
        mined = self.send(data, built)
        # The recipient reads the payment and its memo; the sender reads it
        # back with its outgoing viewing key; the change is the wallet's.
        assert sorted((p['pool'], p['account'], p['value'], p['memo'], p['transfer']) for p in mined['paid']) == [
            ('ironwood', 'outsider', 30_000_000, memo, 'incoming'),
            ('ironwood', 'wallet', 30_000_000, memo, 'outgoing'),
            ('ironwood', 'wallet', 119_990_000, None, 'change')], mined['paid']
        assert mined['fee'] == 10_000 and mined['transparent_outputs'] == [], mined
        status = self.sync(data)
        assert status['spendable'] == '1.94975', status
        assert ('send', '0.3', 'confirmed', info['outsider_address']) in self.history(data), self.history(data)

        # Paying a transparent address spends the Sapling note first, whose
        # proof needs the Sapling parameters: offline, the download is the
        # one request refused, and nothing is signed.
        built = self.run_cli(data, 'wallet', 'send-shielded', 'Shielded', '--to', info['outsider_transparent'],
                             '--amount', '0.1')['artifact']
        assert json.loads(built['prepared_details'])['ZcashShielded']['spends_sapling'], built
        journal = pathlib.Path(data) / 'network.jsonl'
        self.refuses(data, 'Sapling parameters', 'send', 'sign', built['id'],
                     '--review-digest', built['review_digest'], code=1)
        assert {line.split('\t')[0] for line in journal.read_text().splitlines()} == {PARAMETERS}, \
            journal.read_text()
        journal.unlink()
        assert self.run_cli(data, 'send', 'inspect', built['id'])['artifact']['stage'] == 'Prepared'

        # More than the Sapling note holds comes from Ironwood alone, with no
        # parameters, out to the transparent address.
        built = self.run_cli(data, 'wallet', 'send-shielded', 'Shielded', '--to', info['outsider_transparent'],
                             '--amount', '0.5')['artifact']
        prepared = json.loads(built['prepared_details'])['ZcashShielded']
        assert not prepared['uses_sapling'], prepared
        mined = self.send(data, built)
        assert mined['transparent_outputs'] == [{'address': info['outsider_transparent'], 'value': 50_000_000}], mined
        status = self.sync(data)
        fee = prepared['fee_zat']
        assert status['spendable'] == f"{(194_975_000 - 50_000_000 - fee) / 1e8:.8f}".rstrip('0'), (status, fee)
        assert ('send', '0.5', 'confirmed', info['outsider_transparent']) in self.history(data), self.history(data)

        # The same phrase in a second data directory finds the same notes
        # and the same history: what it received, shielded and sent. A
        # shielded payment is recovered from the chain with its outgoing
        # viewing key, which gives the receiver it reached, not the whole
        # address it was sent to; a shielding, which only this device
        # recorded, names no address.
        recovered = self.data_directory()
        self.wallet(recovered)
        status = self.sync(recovered)
        assert status['spendable'] == self.run_cli(data, 'wallet', 'zcash-status', 'Shielded')['shielded']['spendable']
        assert status['address'] == info['wallet_address'], status
        recovered_to = {info['outsider_address']: info['outsider_orchard'], info['wallet_address']: ''}
        expected = sorted((kind, amount, state, recovered_to.get(to, to))
                          for kind, amount, state, to in self.history(data))
        assert self.history(recovered) == expected, (self.history(recovered), expected)

    def test_a_restore_height_past_the_servers_tip_scans_nothing(self):
        late = self.data_directory()
        self.wallet(late, restore_height=START + 10_000)
        self.refuses(late, 'past the chain', 'wallet', 'zcash-sync', 'Shielded')


if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('Run without -O or PYTHONOPTIMIZE: assertions must remain enabled.')
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]], verbosity=2)
