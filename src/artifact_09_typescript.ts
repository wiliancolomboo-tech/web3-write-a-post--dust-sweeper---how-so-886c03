import { Keypair, PublicKey, Ed25519Program, TransactionInstruction } from '@solana/web3.js';
import nacl from 'tweetnacl';

export interface QuestAttestation {
  user: PublicKey;
  questId: Buffer; // 16 bytes
  xpAmount: bigint;
  nonce: bigint;
  signature: Uint8Array;
  ed25519Instruction: TransactionInstruction;
}

export function createQuestAttestation(
  attestationAuthority: Keypair,
  user: PublicKey,
  questIdStr: string,
  xpAmount: bigint,
  nonce: bigint
): QuestAttestation {
  // Pad or hash questId to exactly 16 bytes
  const questId = Buffer.alloc(16);
  Buffer.from(questIdStr.padEnd(16, '0').slice(0, 16), 'utf-8').copy(questId);

  // Construct binary message: [32 bytes user | 16 bytes questId | 8 bytes xp (LE) | 8 bytes nonce (LE)]
  const xpBuffer = Buffer.alloc(8);
  xpBuffer.writeBigUInt64LE(xpAmount);

  const nonceBuffer = Buffer.alloc(8);
  nonceBuffer.writeBigUInt64LE(nonce);

  const message = Buffer.concat([user.toBuffer(), questId, xpBuffer, nonceBuffer]);

  // Sign message with Ed25519 authority
  const signature = nacl.sign.detached(message, attestationAuthority.secretKey);

  // Build the native Solana Ed25519 instruction
  const ed25519Instruction = Ed25519Program.createInstructionWithPublicKey({
    publicKey: attestationAuthority.publicKey.toBytes(),
    message,
    signature,
  });

  return {
    user,
    questId,
    xpAmount,
    nonce,
    signature,
    ed25519Instruction,
  };
}