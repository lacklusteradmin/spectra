#!/usr/bin/env python3
"""ERC-721 and ERC-1155 transfers as staged sends against a loopback EVM
node, across processes: each is built, signed and broadcast, and the node
receives exactly the transaction ethers.js signs for the same fields
(core/tests/fixtures/nft-transfer-vectors.json). Standards, holdings,
refusals, reviews and history are tested in core."""
import unittest

from harness import JsonRpc, RpcError, Spectra, fixture, scratch, serve, unittest_main

VECTORS = fixture('nft-transfer-vectors.json')['signed']
OWNER, RECIPIENT = VECTORS['signer'], VECTORS['recipient']
APES, ITEMS = VECTORS['erc721']['to'], VECTORS['erc1155']['to']
SIGNED = {VECTORS['erc721']['raw']: VECTORS['erc721']['hash'], VECTORS['erc1155']['raw']: VECTORS['erc1155']['hash']}


def word(value):
    return '0x' + format(value, '064x')


class Node(JsonRpc):
    """An Ethereum node where the wallet owns Apes #1234 and five of item 7."""

    def eth_call(self, to, data):
        """An `eth_call`'s answer; anything else reverts."""
        selector, argument = data[2:10], data[10:]
        if selector == '01ffc9a7':
            return word(argument[:8] == {APES: '80ac58cd', ITEMS: 'd9b67a26'}.get(to))
        if selector == '6352211e' and to == APES and int(argument[:64], 16) == 1234:
            return word(int(OWNER, 16))
        if selector == '00fdd58e' and to == ITEMS:
            return word(5 if int(argument[64:128], 16) == 7 else 0)
        raise RpcError(3, 'execution reverted')

    def call(self, method, params):
        if method == 'eth_call':
            return self.eth_call(params[0]['to'].lower(), params[0]['data'])
        if method == 'eth_sendRawTransaction':
            raw = params[0]
            # Only the transactions ethers signs for the same fields.
            assert raw in SIGNED, raw
            self.state['submitted'].append(raw)
            return SIGNED[raw]
        values = {'eth_chainId': '0x1', 'eth_getBalance': hex(10 * 10**18), 'eth_getCode': '0x',
                  'eth_getTransactionCount': '0x3', 'eth_estimateGas': '0x15f90', 'eth_blockNumber': '0x20',
                  'eth_feeHistory': {'baseFeePerGas': ['0x3b9aca00'], 'reward': [['0x77359400']]}}
        assert method in values, method
        return values[method]


class NftTests(unittest.TestCase):
    def setUp(self):
        node = self.enterContext(serve(Node, submitted=[]))
        self.submitted = node.state['submitted']
        self.rpc = node.url + '/rpc'
        self.spectra = Spectra(self.enterContext(scratch('nfts')))

    def send(self, contract, token_id, quantity, vector):
        """Builds, signs and broadcasts the transfer: the node receives the
        signed bytes, which are ethers' for the same fields."""
        before = len(self.submitted)
        built = self.spectra.run('wallet', 'send-nft', 'Collector', '--contract', contract, '--token-id', token_id,
                                 '--quantity', quantity, '--to', RECIPIENT)['artifact']
        signed = self.spectra.run('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                                  '--endpoint', self.rpc)['artifact']
        assert signed['signed_payload'] == vector['raw'], signed['signed_payload']
        assert len(self.submitted) == before, 'signing broadcasts nothing'
        self.spectra.run('send', 'broadcast-signed', signed['id'], '--endpoint', self.rpc, '--yes')

    def test_erc721_and_erc1155_transfers_are_broadcast_as_signed(self):
        imported = self.spectra.run('wallet', 'import', '--chain', 'ethereum', '--name', 'Collector', '--no-password',
                                    '--private-key-env', 'KEY', env={'KEY': '01' * 32})
        assert imported['wallet']['address'].lower() == OWNER, imported
        self.spectra.run('endpoints', '--chain', 'ethereum', '--api', 'evm-json-rpc', '--capabilities',
                         'balance,fee,broadcast,verification,token-balance', '--add', self.rpc)
        self.spectra.run('endpoints', '--chain', 'ethereum', '--custom-only', 'true')
        self.send(APES, '1234', '1', VECTORS['erc721'])
        self.send(ITEMS, '7', '3', VECTORS['erc1155'])
        assert self.submitted == [VECTORS['erc721']['raw'], VECTORS['erc1155']['raw']], self.submitted


if __name__ == '__main__':
    unittest_main()
