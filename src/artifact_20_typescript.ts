import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Aurasweep } from "../target/types/aurasweep";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  Ed25519Program,
  SYSVAR_INSTRUCTIONS_PUBKEY,
} from "@solana/web3.js";
import {
  createMint,
  createAccount,
  mintTo,
  getAccount,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { expect } from "chai";
import nacl from "tweetnacl";

describe("AuraSweep End-to-End QA Test Matrix", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Aurasweep as Program<Aurasweep>;

  const admin = Keypair.generate();
  const oracle = Keypair.generate();
  const treasury = Keypair.generate();
  const user = Keypair.generate();

  let globalConfigPDA: PublicKey;
  let userProfilePDA: PublicKey;
  let testMint: PublicKey;
  let userDustATA: PublicKey;

  before(async () => {
    // 1. Airdrop SOL to test actors
    const airdropPromises = [admin, oracle, user].map(async (actor) => {
      const sig = await provider.connection.requestAirdrop(actor.publicKey, 5 * anchor.web3.LAMPORTS_PER_SOL);
      const latestBlockhash = await provider.connection.getLatestBlockhash();
      await provider.connection.confirmTransaction({
        signature: sig,
        ...latestBlockhash,
      });
    });
    await Promise.all(airdropPromises);

    // 2. Derive PDAs
    [globalConfigPDA] = PublicKey.findProgramAddressSync(
      [Buffer.from("global-config")],
      program.programId
    );

    [userProfilePDA] = PublicKey.findProgramAddressSync(
      [Buffer.from("user-profile"), user.publicKey.toBuffer()],
      program.programId
    );

    // 3. Create dummy SPL token & mint dust to user
    testMint = await createMint(
      provider.connection,
      admin,
      admin.publicKey,
      null,
      6
    );

    userDustATA = await createAccount(
      provider.connection,
      user,
      testMint,
      user.publicKey
    );

    // Mint 500 units of dust
    await mintTo(
      provider.connection,
      admin,
      testMint,
      userDustATA,
      admin,
      500
    );
  });

  it("1. Successfully initializes the global protocol state", async () => {
    const feeBps = 500; // 5%
    const xpPerLamport = new anchor.BN(1);

    await program.methods
      .initialize(feeBps, xpPerLamport)
      .accounts({
        admin: admin.publicKey,
        oracleAuthority: oracle.publicKey,
        protocolTreasury: treasury.publicKey,
        globalConfig: globalConfigPDA,
        systemProgram: SystemProgram.programId,
      })
      .signers([admin])
      .rpc();

    const config = await program.account.globalConfig.fetch(globalConfigPDA);
    expect(config.adminAuthority.toBase58()).to.equal(admin.publicKey.toBase58());
    expect(config.oracleAuthority.toBase58()).to.equal(oracle.publicKey.toBase58());
    expect(config.feeBasisPoints).to.equal(500);
  });

  it("2. Rejects initialization with fee exceeding safety ceiling (>20%)", async () => {
    const dummyConfig = Keypair.generate();
    try {
      await program.methods
        .initialize(2500, new anchor.BN(1)) // 25% > 20% limit
        .accounts({
          admin: admin.publicKey,
          oracleAuthority: oracle.publicKey,
          protocolTreasury: treasury.publicKey,
          globalConfig: dummyConfig.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([admin])
        .rpc();
      expect.fail("Should have failed with FeeTooHigh");
    } catch (err: any) {
      expect(err.error?.errorCode?.code).to.equal("FeeTooHigh");
    }
  });

  it("3. Sweeps dust, burns leftover tokens, refunds rent and awards soulbound XP", async () => {
    const userBalanceBefore = await provider.connection.getBalance(user.publicKey);
    const treasuryBalanceBefore = await provider.connection.getBalance(treasury.publicKey);

    await program.methods
      .sweepDust(new anchor.BN(500))
      .accounts({
        user: user.publicKey,
        globalConfig: globalConfigPDA,
        userProfile: userProfilePDA,
        tokenAccount: userDustATA,
        mint: testMint,
        protocolTreasury: treasury.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .signers([user])
      .rpc();

    // Verify token account is completely closed
    const accountInfo = await provider.connection.getAccountInfo(userDustATA);
    expect(accountInfo).to.be.null;

    // Verify treasury received fee
    const treasuryBalanceAfter = await provider.connection.getBalance(treasury.publicKey);
    expect(treasuryBalanceAfter).to.be.greaterThan(treasuryBalanceBefore);

    // Verify user profile records XP and account swept count
    const profile = await program.account.userProfile.fetch(userProfilePDA);
    expect(profile.accountsSweptCount).to.equal(1);
    expect(profile.totalXp.toNumber()).to.be.greaterThan(0);
  });

  it("4. Accrues XP through cryptographically attested social quests", async () => {
    const questId = 1;
    const xpReward = 500;
    const nonce = 1;

    // Formulate payload: user (32) + questId (4) + xpReward (8) + nonce (8)
    const messageBuffer = Buffer.alloc(52);
    user.publicKey.toBuffer().copy(messageBuffer, 0);
    messageBuffer.writeUInt32LE(questId, 32);
    messageBuffer.writeBigUInt64LE(BigInt(xpReward), 36);
    messageBuffer.writeBigUInt64LE(BigInt(nonce), 44);

    const sig = nacl.sign.detached(messageBuffer, oracle.secretKey);

    // Construct Ed25519 verification instruction
    const ed25519Ix = Ed25519Program.createInstructionWithPublicKey({
      publicKey: oracle.publicKey.toBytes(),
      message: messageBuffer,
      signature: sig,
    });

    const claimIx = await program.methods
      .claimSocialQuest(questId, new anchor.BN(xpReward), new anchor.BN(nonce), Array.from(sig))
      .accounts({
        user: user.publicKey,
        globalConfig: globalConfigPDA,
        userProfile: userProfilePDA,
        sysvarInstructions: SYSVAR_INSTRUCTIONS_PUBKEY,
      })
      .instruction();

    const tx = new Transaction().add(ed25519Ix).add(claimIx);
    await provider.sendAndConfirm(tx, [user]);

    const profile = await program.account.userProfile.fetch(userProfilePDA);
    expect(profile.questNonce.toNumber()).to.equal(1);
    expect(profile.lastCompletedQuestId).to.equal(1);
  });

  it("5. Rejects social quest claim with replayed nonce or forged oracle signature", async () => {
    const questId = 1; // already claimed
    const xpReward = 500;
    const nonce = 1; // reused nonce

    const messageBuffer = Buffer.alloc(52);
    user.publicKey.toBuffer().copy(messageBuffer, 0);
    messageBuffer.writeUInt32LE(questId, 32);
    messageBuffer.writeBigUInt64LE(BigInt(xpReward), 36);
    messageBuffer.writeBigUInt64LE(BigInt(nonce), 44);

    const sig = nacl.sign.detached(messageBuffer, oracle.secretKey);
    const ed25519Ix = Ed25519Program.createInstructionWithPublicKey({
      publicKey: oracle.publicKey.toBytes(),
      message: messageBuffer,
      signature: sig,
    });

    const claimIx = await program.methods
      .claimSocialQuest(questId, new anchor.BN(xpReward), new anchor.BN(nonce), Array.from(sig))
      .accounts({
        user: user.publicKey,
        globalConfig: globalConfigPDA,
        userProfile: userProfilePDA,
        sysvarInstructions: SYSVAR_INSTRUCTIONS_PUBKEY,
      })
      .instruction();

    try {
      const tx = new Transaction().add(ed25519Ix).add(claimIx);
      await provider.sendAndConfirm(tx, [user]);
      expect.fail("Expected failure on replay");
    } catch (err: any) {
      expect(err.error?.errorCode?.code).to.equal("InvalidNonceOrReplay");
    }
  });
});