// Independent Safe{Wallet} multisig vectors, for core/tests/fixtures/safe-multisig.json.
// Install the pinned libraries in a temporary directory:
// npm install --prefix /tmp/spectra-safe-multisig --ignore-scripts @safe-global/protocol-kit@6.1.2 @safe-global/safe-deployments@1.37.63 @safe-global/types-kit@3.1.0 viem@2.56.9
// NODE_PATH=/tmp/spectra-safe-multisig/node_modules node scripts/generate-safe-multisig-vectors.cjs > core/tests/fixtures/safe-multisig.json
//
// Three BIP-39 test phrases own a Safe at a fixed address, each through its
// EVM account at m/44'/60'/0'/0/0 (viem's mnemonicToAccount). For a native
// payment, an ERC-20 transfer and a DELEGATECALL, on Sepolia and Ethereum:
// the EIP-712 domain separator, the SafeTx struct hash and the safeTxHash,
// each owner's EIP-712 signature (v 27/28) and eth_sign signature (v 31/32),
// the owner-ascending signature bytes for two and three owners and the
// execTransaction calldata for two. Nothing touches a network: protocol-kit's
// SafeProvider gets an in-process EIP-1193 stub that answers eth_chainId and
// throws on anything else. Every hash is computed three ways (protocol-kit's
// preimageSafeTransactionHash, its generateTypedData through viem's
// hashTypedData, and a Solidity-shaped encoding of Safe.sol's
// encodeTransactionData) for 1.3.0 and 1.4.1, which hash alike; every
// signature is RFC 6979 and is recovered the way checkNSignatures recovers it.
// The singleton addresses are safe-deployments' Safe and SafeL2 for 1.3.0 and
// 1.4.1, every deployment type, per chain id.
const protocolKit = require('@safe-global/protocol-kit');
const deployments = require('@safe-global/safe-deployments');
const viem = require('viem');
const { mnemonicToAccount } = require('viem/accounts');

const {
  EthSafeSignature, SafeProvider, buildSignatureBytes, generateEIP712Signature, generateSignature,
  generateTypedData, preimageSafeTransactionHash,
} = protocolKit;
const {
  concat, encodeAbiParameters, encodeFunctionData, erc20Abi, getAddress, hashMessage, hashTypedData, keccak256,
  recoverAddress, toHex,
} = viem;

const PHRASES = [
  'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about',
  'legal winner thank year wave sausage worth useful legal winner thank yellow',
  'letter advice cage absurd amount doctor acoustic avoid letter advice cage above',
];
const PATH = "m/44'/60'/0'/0/0";
const SAFE = getAddress('0x5afe000000000000000000000000000000005afe');
const CHAIN_IDS = [11155111, 1];
const VERSIONS = ['1.3.0', '1.4.1'];
const SINGLETON_CHAIN_IDS = [1, 11155111, 10, 8453, 42161, 137, 56, 324];
const ZERO = '0x0000000000000000000000000000000000000000';
const DEAD = '0x000000000000000000000000000000000000dEaD';
// Safe.sol (1.3.0 and later) DOMAIN_SEPARATOR_TYPEHASH and SAFE_TX_TYPEHASH.
const DOMAIN_TYPEHASH = keccak256(toHex('EIP712Domain(uint256 chainId,address verifyingContract)'));
const SAFE_TX_TYPEHASH = keccak256(toHex(
  'SafeTx(address to,uint256 value,bytes data,uint8 operation,uint256 safeTxGas,uint256 baseGas,uint256 gasPrice,address gasToken,address refundReceiver,uint256 nonce)',
));

// SafeTx's fields in its type's order; no gas refund.
function transaction({ to, value, data, operation, nonce }) {
  return { to, value, data, operation, safeTxGas: '0', baseGas: '0', gasPrice: '0', gasToken: ZERO,
    refundReceiver: ZERO, nonce };
}
const TRANSACTIONS = [
  { name: 'native', signed: true,
    tx: transaction({ to: DEAD, value: '10000000000000000', data: '0x', operation: 0, nonce: 7 }) },
  { name: 'erc20', signed: true,
    tx: transaction({ to: getAddress('0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238'), value: '0',
      data: encodeFunctionData({ abi: erc20Abi, functionName: 'transfer', args: [DEAD, 1234567n] }),
      operation: 0, nonce: 8 }) },
  { name: 'delegatecall', signed: false,
    tx: transaction({ to: DEAD, value: '10000000000000000', data: '0x', operation: 1, nonce: 7 }) },
];

// Safe.sol's getTransactionHash, written out.
function solidityHash(chainId, tx) {
  const domainSeparator = keccak256(encodeAbiParameters(
    [{ type: 'bytes32' }, { type: 'uint256' }, { type: 'address' }], [DOMAIN_TYPEHASH, BigInt(chainId), SAFE]));
  const structHash = keccak256(encodeAbiParameters(
    ['bytes32', 'address', 'uint256', 'bytes32', 'uint8', 'uint256', 'uint256', 'uint256', 'address', 'address', 'uint256']
      .map((type) => ({ type })),
    [SAFE_TX_TYPEHASH, tx.to, BigInt(tx.value), keccak256(tx.data), tx.operation, BigInt(tx.safeTxGas),
      BigInt(tx.baseGas), BigInt(tx.gasPrice), tx.gasToken, tx.refundReceiver, BigInt(tx.nonce)]));
  return { domainSeparator, structHash, safeTxHash: keccak256(concat(['0x1901', domainSeparator, structHash])) };
}

// checkNSignatures' recovery of one 65-byte ECDSA or eth_sign signature.
async function recover(safeTxHash, signature) {
  const v = parseInt(signature.slice(-2), 16);
  if (v > 30) {
    return recoverAddress({ hash: hashMessage({ raw: safeTxHash }), signature: signature.slice(0, -2) + (v - 4).toString(16) });
  }
  return recoverAddress({ hash: safeTxHash, signature });
}

function check(condition, message) {
  if (!condition) throw new Error(message);
}

const owners = PHRASES.map((phrase) => {
  const account = mnemonicToAccount(phrase, { path: PATH });
  return { account, address: account.address, privateKey: toHex(account.getHdKey().privateKey) };
});
const ascending = (list) => [...list].sort((a, b) => (BigInt(a.address) < BigInt(b.address) ? -1 : 1));

function stub(chainId) {
  return {
    request: async ({ method }) => {
      if (method === 'eth_chainId') return toHex(chainId);
      throw new Error(`offline stub: unexpected RPC ${method}`);
    },
  };
}

function singletons() {
  const byChain = {};
  const kinds = {};
  for (const chainId of SINGLETON_CHAIN_IDS) {
    const network = String(chainId);
    const addresses = [];
    for (const version of VERSIONS) {
      for (const [contract, accessor] of [['Safe', deployments.getSafeSingletonDeployments],
        ['SafeL2', deployments.getSafeL2SingletonDeployments]]) {
        const deployment = accessor({ version, network, released: true });
        const listed = deployment && deployment.networkAddresses[network];
        if (listed === undefined) continue;
        for (const address of [].concat(listed)) {
          const type = Object.keys(deployment.deployments).find((t) => deployment.deployments[t].address === address);
          check(type, `${address} has no deployment type`);
          const checksummed = getAddress(address);
          check(checksummed === address, `${address} is not checksummed`);
          if (!addresses.includes(address)) addresses.push(address);
          kinds[address] = { version, contract, type };
        }
      }
    }
    byChain[network] = addresses;
  }
  return { byChain, kinds };
}

async function main() {
  const abis = Object.fromEntries(VERSIONS.map((version) => [version,
    deployments.getSafeSingletonDeployment({ version, released: true }).abi]));
  const transactions = [];
  for (const { name, signed, tx } of TRANSACTIONS) {
    const entry = { name, ...Object.fromEntries(Object.entries(tx).map(([key, value]) =>
      [key.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`), value])), chains: [] };
    for (const chainId of CHAIN_IDS) {
      const manual = solidityHash(chainId, tx);
      const byVersion = {};
      for (const version of VERSIONS) {
        const preimage = keccak256(preimageSafeTransactionHash(SAFE, tx, version, BigInt(chainId)));
        const typed = generateTypedData({ safeAddress: SAFE, safeVersion: version, chainId: BigInt(chainId), data: tx });
        const viaTyped = hashTypedData({
          domain: typed.domain, types: { SafeTx: typed.types.SafeTx }, primaryType: 'SafeTx', message: typed.message,
        });
        check(preimage === manual.safeTxHash && viaTyped === manual.safeTxHash, `${name} ${chainId} ${version}: hash`);
        byVersion[version] = preimage;
      }
      const vector = {
        chain_id: chainId,
        domain_separator: manual.domainSeparator,
        struct_hash: manual.structHash,
        safe_tx_hash: manual.safeTxHash,
        safe_tx_hash_by_version: byVersion,
      };
      if (signed) {
        const safeTxHash = manual.safeTxHash;
        const signatures = [];
        for (const owner of owners) {
          const provider = new SafeProvider({ provider: stub(chainId), signer: owner.privateKey });
          const perVersion = [];
          for (const version of VERSIONS) {
            perVersion.push((await generateEIP712Signature(provider,
              { safeAddress: SAFE, safeVersion: version, chainId: BigInt(chainId), data: tx }, 'v4')).data);
          }
          check(perVersion.every((s) => s === perVersion[0]), `${name}: EIP-712 signature varies by version`);
          const ecdsa = perVersion[0];
          const ethSign = (await generateSignature(provider, safeTxHash)).data;
          check(ecdsa === await owner.account.sign({ hash: safeTxHash }), `${name}: EIP-712 is not the raw hash signature`);
          const personal = await owner.account.signMessage({ message: { raw: safeTxHash } });
          check(ethSign === personal.slice(0, -2) + (parseInt(personal.slice(-2), 16) + 4).toString(16),
            `${name}: eth_sign is not personal_sign with v + 4`);
          for (const signature of [ecdsa, ethSign]) {
            check(await recover(safeTxHash, signature) === owner.address, `${name}: recovery`);
          }
          check([27, 28].includes(parseInt(ecdsa.slice(-2), 16)), 'ECDSA v');
          check([31, 32].includes(parseInt(ethSign.slice(-2), 16)), 'eth_sign v');
          signatures.push({ owner: owner.address, ecdsa, eth_sign: ethSign });
        }
        vector.signatures = signatures;
        for (const [key, set] of [['signatures_p0_p1', [0, 1]], ['signatures_p0_p1_p2', [0, 1, 2]]]) {
          const chosen = set.map((i) => ({ address: owners[i].address, signature: signatures[i].ecdsa }));
          const bytes = buildSignatureBytes(chosen.map((c) => new EthSafeSignature(c.address, c.signature)));
          check(bytes === concat(ascending(chosen).map((c) => c.signature)), `${key}: not owner-ascending`);
          vector[key] = bytes;
        }
        const calldata = VERSIONS.map((version) => encodeFunctionData({
          abi: abis[version], functionName: 'execTransaction',
          args: [tx.to, BigInt(tx.value), tx.data, tx.operation, BigInt(tx.safeTxGas), BigInt(tx.baseGas),
            BigInt(tx.gasPrice), tx.gasToken, tx.refundReceiver, vector.signatures_p0_p1],
        }));
        check(calldata.every((c) => c === calldata[0]) && calldata[0].startsWith('0x6a761202'), 'execTransaction');
        vector.exec_transaction_p0_p1 = calldata[0];
      }
      entry.chains.push(vector);
    }
    transactions.push(entry);
  }
  const { byChain, kinds } = singletons();
  console.log(JSON.stringify({
    provenance: '@safe-global/protocol-kit 6.1.2, @safe-global/safe-deployments 1.37.63, @safe-global/types-kit 3.1.0, viem 2.56.9 (scripts/generate-safe-multisig-vectors.cjs)',
    phrases: PHRASES,
    path: PATH,
    owners: owners.map((o) => ({ address: o.address, private_key: o.privateKey })),
    owners_ascending: ascending(owners).map((o) => o.address),
    safe: SAFE,
    versions: VERSIONS,
    domain_typehash: DOMAIN_TYPEHASH,
    safe_tx_typehash: SAFE_TX_TYPEHASH,
    exec_transaction_selector: '0x6a761202',
    transactions,
    singletons: byChain,
    singleton_kinds: kinds,
  }, null, 2));
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
