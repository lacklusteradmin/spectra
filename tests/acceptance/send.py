#!/usr/bin/env python3
"""A password-protected owned send, reviewed and broadcast in one process.

`send owned-broadcast` is refused without --yes; with it, and the wallet's
password from the environment, the node receives the signed transaction
exactly once. Review, fees and the history it records are core's rules,
tested with `cargo test`.

Run: python3 -B tests/acceptance/send.py [path/to/spectra] [TestClass.test_name]
Uses a temporary store and a loopback node; no public network is required.
"""
import json
import sqlite3
import unittest

from harness import ABANDON, JsonRpc, Spectra, scratch, serve, unittest_main


class Node(JsonRpc):
    def call(self, method, params):
        if method == 'eth_sendRawTransaction':
            self.state['submitted'].append(params[0])
            # The hash the stored transaction was signed to, as a node answers.
            with sqlite3.connect(self.state['store']) as db:
                artifacts = [json.loads(row[0]) for row in db.execute('SELECT payload FROM send_artifacts')]
            return next(a for a in artifacts if a['view']['signed_payload'] == params[0])['view']['transaction_hash']
        values = {'eth_chainId': '0x1', 'eth_getBalance': '0x8ac7230489e80000', 'eth_estimateGas': '0x5208',
                  'eth_getCode': '0x', 'eth_getTransactionCount': '0x7',
                  'eth_feeHistory': {'baseFeePerGas': ['0x3b9aca00'], 'reward': [['0x77359400']]}}
        assert method in values, method
        return values[method]


class SendTests(unittest.TestCase):
    def test_password_protected_broadcast(self):
        """Refused without --yes; with it and the password, sent exactly once."""
        with scratch('send-password') as directory, \
                serve(Node, store=directory / 'spectra.sqlite', submitted=[]) as node:
            submitted = node.state['submitted']
            spectra = Spectra(directory, SPECTRA_PASSWORD='fixture-wallet-password')
            spectra.run('endpoints', '--chain', 'ethereum', '--api', 'evm-json-rpc',
                        '--capabilities', 'balance,fee,broadcast,verification,token-balance', '--add', node.url)
            spectra.run('wallet', 'import', '--chain', 'ethereum', '--name', 'Sealed', env={'SPECTRA_SEED': ABANDON})
            # The holding a balance refresh would have stored.
            with sqlite3.connect(directory / 'spectra.sqlite') as db:
                wid, payload = db.execute('SELECT id,payload FROM wallets').fetchone()
                wallet = json.loads(payload)
                wallet['holdings'] = [dict(name='Ethereum', symbol='ETH', coingeckoId='ethereum', chainId='ethereum',
                                           tokenStandard=None, contractAddress=None, amount='10')]
                db.execute('UPDATE wallets SET payload=? WHERE id=?', (json.dumps(wallet), wid))
            args = ('send', 'owned-broadcast', 'Sealed', '--holding', 'ethereum:native', '--amount', '1',
                    '--destination', '0x' + '22' * 20)
            refused = spectra.fail(*args, code=2)
            assert '--yes' in refused['error'], refused
            assert not submitted, submitted
            sent = spectra.run(*args, '--yes')
            assert sent['transactionHash'].startswith('0x') and len(sent['transactionHash']) == 66, sent
            assert len(submitted) == 1, submitted


if __name__ == '__main__':
    unittest_main()
