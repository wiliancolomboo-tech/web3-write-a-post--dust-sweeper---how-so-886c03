import {
  Connection,
  PublicKey,
  Keypair,
  Transaction,
  sendAndConfirmTransaction,
} from '@solana/web3.js';
import { TOKEN_PROGRAM_ID } from '@solana/spl-token';
import { Program, AnchorProvider, Wallet, Idl, BN } from '@coral-xyz/anchor';
import { createQuestAttestation } from './questAttester';
import { sendEnterpriseWebhook } from './telemetry';

export class DustSweeperEngine {
  private connection: Connection;
  private program: Program;
  private provider: AnchorProvider;

  constructor(
    rpcUrl: string,
    keypair: Keypair,
    programId: PublicKey,
    idl: Idl
  ) {
    this.connection = new Connection(rpcUrl, 'confirmed');
    const wallet = new Wallet(keypair);
    this.provider = new AnchorProvider(this.connection, wallet, { commitment: 'confirmed' });
    this.program = new Program(idl, this.provider);
  }

  /**
   * Identifies all empty token accounts owned by the wallet
   */
  async findDormantAccounts(owner: PublicKey): Promise<PublicKey[]> {
    const response = await this.connection.getParsedTokenAccountsByOwner(owner, {
      programId: TOKEN_PROGRAM_ID,
    });

    const emptyAccounts: PublicKey[] = [];
    for (const { pubkey, account } of response.value) {
      const amountStr = account.data.parsed.info.tokenAmount.amount;
      if (amountStr === "0") {
        emptyAccounts.push(pubkey);
      }
    }
    return emptyAccounts;
  }

  /**
   * Sweeps dormant token accounts, reclaims rent, verifies quest, and awards XP
   */
  async executeSweepAndClaimSocial(
    userKeypair: Keypair,
    attestationAuthority: Keypair,
    questId: string,
    questXp: bigint
  ): Promise<string> {
    const userPubkey = userKeypair.publicKey;
    const emptyAccounts = await this.findDormantAccounts(userPubkey);

    console.log(`[SweeperEngine] Found ${emptyAccounts.length} empty token accounts for ${userPubkey.toBase58()}`);

    const [configPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("config")],
      this.program.programId
    );
    const [userProfilePda] = PublicKey.findProgramAddressSync(
      [Buffer.from("user_profile"), userPubkey.toBuffer()],
      this.program.programId
    );

    const configAccount = await (this.program.account as any).config.fetch(configPda);
    const treasuryPubkey = configAccount.treasury as PublicKey;

    const tx = new Transaction();

    // Check if user profile is initialized; if not, initialize it
    const profileInfo = await this.connection.getAccountInfo(userProfilePda);
    if (!profileInfo) {
      console.log(`[SweeperEngine] Initializing user profile PDA...`);
      const initProfileIx = await this.program.methods
        .initializeUserProfile()
        .accounts({
          userProfile: userProfilePda,
          user: userPubkey,
          systemProgram: PublicKey.default,
        })
        .instruction();
      tx.add(initProfileIx);
    }

    // Add up to 5 close account instructions per atomic batch to prevent compute limit overflow
    const accountsToClose = emptyAccounts.slice(0, 5);
    for (const ata of accountsToClose) {
      const closeIx = await this.program.methods
        .closeEmptyTokenAccount()
        .accounts({
          config: configPda,
          userProfile: userProfilePda,
          tokenAccount: ata,
          user: userPubkey,
          treasury: treasuryPubkey,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: PublicKey.default,
        })
        .instruction();
      tx.add(closeIx);
    }

    // Attach Social Quest attestation if requested
    let xpAwarded = accountsToClose.length * 50;
    if (questId && questXp > 0n) {
      const nonce = BigInt(Date.now());
      const attestation = createQuestAttestation(
        attestationAuthority,
        userPubkey,
        questId,
        questXp,
        nonce
      );

      const [nonceRecordPda] = PublicKey.findProgramAddressSync(
        [Buffer.from("nonce_record"), userPubkey.toBuffer(), (() => {
          const b = Buffer.alloc(8);
          b.writeBigUInt64LE(nonce);
          return b;
        })()],
        this.program.programId
      );

      // Precompile must immediately precede the program instruction
      tx.add(attestation.ed25519Instruction);

      const claimIx = await this.program.methods
        .claimSocialXp(
          Array.from(attestation.questId),
          new BN(questXp.toString()),
          new BN(nonce.toString()),
          Array.from(attestation.signature)
        )
        .accounts({
          config: configPda,
          userProfile: userProfilePda,
          nonceRecord: nonceRecordPda,
          user: userPubkey,
          instructionSysvar: new PublicKey("Sysvar1nstructions1111111111111111111111111"),
          systemProgram: PublicKey.default,
        })
        .instruction();

      tx.add(claimIx);
      xpAwarded += Number(questXp);
    }

    // Send and confirm atomic bundle
    const signature = await sendAndConfirmTransaction(
      this.connection,
      tx,
      [userKeypair],
      { skipPreflight: false, commitment: "confirmed" }
    );

    const reclaimedSol = accountsToClose.length * 0.00203928 * 0.95; // net after 5% fee
    console.log(`[SweeperEngine] Batch executed! Tx: ${signature}`);

    await sendEnterpriseWebhook({
      userAddress: userPubkey.toBase58(),
      accountsClosed: accountsToClose.length,
      reclaimedSol,
      xpEarned: xpAwarded,
      txSignature: signature,
    });

    return signature;
  }
}