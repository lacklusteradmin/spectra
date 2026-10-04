// Offline staking wire/signature fixtures from independent official SDKs.
// npm install --prefix /tmp/spectra-staking-sdk --ignore-scripts @solana/web3.js@1.98.4 @near-js/transactions@2.5.1 @near-js/crypto@2.5.1 @aptos-labs/ts-sdk@1.39.0 @mysten/sui@1.45.2
// NODE_PATH=/tmp/spectra-staking-sdk/node_modules node scripts/generate-account-staking-vectors.cjs
const fs = require('node:fs');
const crypto = require('node:crypto');
const sol = require('@solana/web3.js');
const apt = require('@aptos-labs/ts-sdk');
const near = require('@near-js/transactions');
const { PublicKey } = require('@near-js/crypto');
const { Transaction } = require('@mysten/sui/transactions');
const { Ed25519Keypair } = require('@mysten/sui/keypairs/ed25519');
const hex = bytes => Buffer.from(bytes).toString('hex');
const address = value => '0x' + value.repeat(32);
async function main() {
  const seed = Buffer.alloc(32, 1), amount = 2000000000n;
  const fixtures = { provenance: '@solana/web3.js 1.98.4; @near-js/transactions 2.5.1; @near-js/crypto 2.5.1; @aptos-labs/ts-sdk 1.39.0; @mysten/sui 1.45.2', solana: [], aptos: [], sui: [], near: [] };
  const key = sol.Keypair.fromSeed(seed), vote = new sol.PublicKey(Buffer.alloc(32,0x22)), blockhash = new sol.PublicKey(Buffer.alloc(32,0x33)).toBase58();
  const program = sol.StakeProgram.programId, accountSeed = 'spectra-offline-staking';
  const account = await sol.PublicKey.createWithSeed(key.publicKey, accountSeed, program);
  const meta = (pubkey, isWritable, isSigner = false) => ({pubkey,isWritable,isSigner});
  const instruction = (keys, data) => new sol.TransactionInstruction({programId:program,keys,data});
  // Current official stake/interface source removes sysvar accounts from these
  // instructions. SDK serialization/signing verifies that precise account list.
  const initialize = Buffer.alloc(116); initialize.writeUInt32LE(0); key.publicKey.toBuffer().copy(initialize,4); key.publicKey.toBuffer().copy(initialize,36);
  const delegate = Buffer.alloc(4); delegate.writeUInt32LE(2);
  const deactivate = Buffer.alloc(4); deactivate.writeUInt32LE(5);
  const withdraw = Buffer.alloc(12); withdraw.writeUInt32LE(4); withdraw.writeBigUInt64LE(amount,4);
  for (const [name,instructions] of [
    ['stake', [sol.SystemProgram.createAccountWithSeed({fromPubkey:key.publicKey,newAccountPubkey:account,basePubkey:key.publicKey,seed:accountSeed,lamports:Number(amount)+2282880,space:200,programId:program}),instruction([meta(account,true)],initialize),instruction([meta(account,true),meta(vote,false),meta(key.publicKey,false,true)],delegate)]],
    ['unstake', [instruction([meta(account,true),meta(key.publicKey,false,true)],deactivate)]],
    ['withdraw', [instruction([meta(account,true),meta(key.publicKey,true),meta(key.publicKey,false,true)],withdraw)]] ]) {
    const message = new sol.TransactionMessage({payerKey:key.publicKey,recentBlockhash:blockhash,instructions}).compileToLegacyMessage();
    const tx = new sol.VersionedTransaction(message); tx.sign([key]);
    fixtures.solana.push({name,owner:key.publicKey.toBase58(),vote:vote.toBase58(),account:account.toBase58(),seed:accountSeed,blockhash,message:hex(message.serialize()),signed:hex(tx.serialize())});
  }
  const aptos = apt.Account.fromPrivateKey({privateKey:new apt.Ed25519PrivateKey(hex(seed)),legacy:true});
  for (const name of ['add_stake','unlock','withdraw']) {
    const payload = new apt.TransactionPayloadEntryFunction(apt.EntryFunction.build('0x1::delegation_pool',name,[],[apt.AccountAddress.fromString(address('22')),new apt.U64(amount)]));
    const raw = new apt.RawTransaction(aptos.accountAddress,7n,payload,12000n,100n,1800000000n,new apt.ChainId(1));
    const message = apt.generateSigningMessage(raw.bcsToBytes(),'APTOS::RawTransaction'), signature = aptos.sign(message);
    const hash = apt.generateUserTransactionHash({transaction:new apt.SimpleTransaction(raw),senderAuthenticator:new apt.AccountAuthenticatorEd25519(aptos.publicKey,signature)});
    fixtures.aptos.push({name,owner:aptos.accountAddress.toString(),message:hex(message),signature:signature.toString().replace(/^0x/,''),hash});
  }
  const sui = Ed25519Keypair.fromSecretKey(seed), object = (id,version) => ({objectId:address(id),version:String(version),digest:'11111111111111111111111111111111'});
  for (const name of ['stake','withdraw']) {
    const tx = new Transaction(); tx.setSender(sui.toSuiAddress()); tx.setGasPrice(1000); tx.setGasBudget(10000000); tx.setGasPayment([object('33',7)]);
    const system = tx.sharedObjectRef({objectId:'0x5',initialSharedVersion:'1',mutable:true});
    if (name === 'stake') {
      const [coin] = tx.splitCoins(tx.gas,[tx.pure.u64(amount)]);
      tx.moveCall({target:'0x3::sui_system::request_add_stake',arguments:[system,coin,tx.pure.address(address('22'))]});
    } else {
      tx.moveCall({target:'0x3::sui_system::request_withdraw_stake',arguments:[system,tx.objectRef(object('44',8))]});
    }
    const bytes = await tx.build(), signature = (await sui.signTransaction(bytes)).signature;
    fixtures.sui.push({name,owner:sui.toSuiAddress(),raw:hex(bytes),signature,hash:await tx.getDigest()});
  }
  const publicKey = new PublicKey({keyType:0,data:key.publicKey.toBytes()});
  const secret = crypto.createPrivateKey({key:Buffer.concat([Buffer.from('302e020100300506032b657004220420','hex'),seed]),format:'der',type:'pkcs8'});
  for (const name of ['deposit_and_stake','unstake','withdraw']) {
    const args = Buffer.from(name === 'deposit_and_stake' ? '{}' : '{"amount":"2000000000"}');
    const action = near.actionCreators.functionCall(name,args,100000000000000n,name === 'deposit_and_stake' ? amount : 0n);
    const tx = near.createTransaction('alice.near',publicKey,'validator.poolv1.near',7n,[action],Buffer.alloc(32,0x33));
    const bytes = near.encodeTransaction(tx), hash = crypto.createHash('sha256').update(bytes).digest();
    const signature = crypto.sign(null,hash,secret), signed = new near.SignedTransaction({transaction:tx,signature:new near.Signature({keyType:0,data:signature})});
    fixtures.near.push({name,message:hex(bytes),signed:hex(signed.encode()),hash:new sol.PublicKey(hash).toBase58()});
  }
  fs.writeFileSync('core/tests/fixtures/account-staking-vectors.json',JSON.stringify(fixtures,null,2)+'\n');
}
main().catch(error => {console.error(error);process.exitCode=1;});
