// Independent, offline SDK fixtures. Install outside the repository:
// npm install --prefix /tmp/spectra-audit-sdk --ignore-scripts @mysten/sui@1.45.2 @aptos-labs/ts-sdk@1.39.0 @solana/web3.js@1.98.4 @solana/spl-token@0.4.14 tronweb@6.0.4 ed25519-hd-key@1.3.0 bip39@3.1.0
// NODE_PATH=/tmp/spectra-audit-sdk/node_modules node scripts/generate-send-audit-vectors.cjs
const fs = require('node:fs');
const {derivePath} = require('ed25519-hd-key');
const {mnemonicToSeedSync} = require('bip39');
const { Transaction: SuiTransaction } = require('@mysten/sui/transactions');
const { Ed25519Keypair } = require('@mysten/sui/keypairs/ed25519');
const apt = require('@aptos-labs/ts-sdk');
const sol = require('@solana/web3.js');
const spl = require('@solana/spl-token');
const { TronWeb, utils: tronUtils } = require('tronweb');
const mnemonic = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';
const hex = b => Buffer.from(b).toString('hex');
async function main() {
 const fixtures = { provenance: '@mysten/sui 1.45.2; @aptos-labs/ts-sdk 1.39.0; @solana/web3.js 1.98.4; @solana/spl-token 0.4.14; tronweb 6.0.4; ed25519-hd-key 1.3.0; bip39 3.1.0', mnemonic };
 const sui = Ed25519Keypair.deriveKeypair(mnemonic);
 const tx = new SuiTransaction(); tx.setSender(sui.toSuiAddress()); tx.setGasPrice(1000); tx.setGasBudget(10000000);
 const digest = '11111111111111111111111111111111';
 tx.setGasPayment([{ objectId:'0x'+'33'.repeat(32),version:'7',digest }]);
 const [coin] = tx.splitCoins(tx.gas, [tx.pure.u64(123456789)]); tx.transferObjects([coin],tx.pure.address('0x'+'22'.repeat(32)));
 const bytes = await tx.build(); const signed = await sui.signTransaction(bytes);
 fixtures.sui = { address:sui.toSuiAddress(), public_key:hex(sui.getPublicKey().toRawBytes()), raw:hex(bytes), signature:signed.signature };
 const aptos = apt.Account.fromDerivationPath({ mnemonic, path:"m/44'/637'/0'/0'/0'", legacy:true });
 const recipient = apt.AccountAddress.fromString('0x'+'22'.repeat(32));
 const payload = new apt.TransactionPayloadEntryFunction(apt.EntryFunction.build('0x1::coin','transfer',[apt.parseTypeTag('0x1::aptos_coin::AptosCoin')],[recipient,new apt.U64(123456789n)]));
 const raw = new apt.RawTransaction(aptos.accountAddress,7n,payload,10000n,100n,1800000000n,new apt.ChainId(1));
 const message = apt.generateSigningMessage(raw.bcsToBytes(),'APTOS::RawTransaction');
 fixtures.aptos = { address:aptos.accountAddress.toString(), public_key:aptos.publicKey.toString().replace(/^0x/,''), message:hex(message), signature:aptos.sign(message).toString().replace(/^0x/,'') };
 const seed = derivePath("m/44'/501'/0'/0'", mnemonicToSeedSync(mnemonic).toString("hex")).key;
 const solana = sol.Keypair.fromSeed(seed); const dest = new sol.PublicKey(Buffer.alloc(32,0x22));
 const blockhash = new sol.PublicKey(Buffer.alloc(32,0x33)).toBase58();
 const solTx = new sol.Transaction({ feePayer:solana.publicKey, recentBlockhash:blockhash }).add(sol.SystemProgram.transfer({fromPubkey:solana.publicKey,toPubkey:dest,lamports:123456789})); solTx.sign(solana);
 const mint = new sol.PublicKey(Buffer.alloc(32,0x44)); const sourceAta = spl.getAssociatedTokenAddressSync(mint, solana.publicKey); const destAta = spl.getAssociatedTokenAddressSync(mint,dest,true);
 const splTx = new sol.Transaction({feePayer:solana.publicKey,recentBlockhash:blockhash}).add(spl.createAssociatedTokenAccountIdempotentInstruction(solana.publicKey,destAta,dest,mint),spl.createTransferCheckedInstruction(sourceAta,mint,destAta,solana.publicKey,123456789n,6)); splTx.sign(solana);
 fixtures.solana = { path:"m/44'/501'/0'/0'", address:solana.publicKey.toBase58(), public_key:hex(solana.publicKey.toBytes()), blockhash, native:hex(solTx.serialize()), spl:hex(splTx.serialize()), source_ata:hex(sourceAta.toBytes()),dest_ata:hex(destAta.toBytes()) };
 const key = TronWeb.fromMnemonic(mnemonic, "m/44'/195'/0'/0/0").privateKey.replace(/^0x/,''); const from = TronWeb.address.fromPrivateKey(key); const to = TronWeb.address.fromHex('41'+'22'.repeat(20)); const contract = TronWeb.address.fromHex('41'+'44'.repeat(20));
 fixtures.tron = { key, from, to, contract, transactions:[] };
 for (const token of [false,true]) {
   const name = token?'TriggerSmartContract':'TransferContract';
   const value = token? {owner_address:TronWeb.address.toHex(from),contract_address:TronWeb.address.toHex(contract),data:'a9059cbb'+'00'.repeat(12)+'22'.repeat(20)+'00'.repeat(28)+'075bcd15'} : {owner_address:TronWeb.address.toHex(from),to_address:TronWeb.address.toHex(to),amount:123456789};
   const transaction = { visible:false, raw_data:{ref_block_bytes:'0007',ref_block_hash:'33'.repeat(8),expiration:1800000060000,timestamp:1800000000000,contract:[{type:name,parameter:{type_url:'type.googleapis.com/protocol.'+name,value}}]} };
   if(token) transaction.raw_data.fee_limit=100000000;
   const pb=tronUtils.transaction.txJsonToPb(transaction); transaction.raw_data_hex=tronUtils.transaction.txPbToRawDataHex(pb).toLowerCase(); transaction.txID=tronUtils.transaction.txPbToTxID(pb).replace(/^0x/,'').toLowerCase();
   tronUtils.crypto.signTransaction(Buffer.from(key,'hex'),transaction);
   fixtures.tron.transactions.push(transaction);
 }
 fs.writeFileSync('core/testdata/send-audit-vectors.json', JSON.stringify(fixtures,null,2)+'\n');
}
main().catch(e=>{console.error(e);process.exitCode=1;});
