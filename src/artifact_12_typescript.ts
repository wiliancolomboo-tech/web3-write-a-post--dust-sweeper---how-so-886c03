import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { DustSweeperXp } from "../target/types/dust_sweeper_xp";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_INSTRUCTIONS_PUBKEY,
  sendAndConfirmTransaction,
  Transaction,
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

describe("dust_sweeper_xp", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.DustSweeperXp as Program<DustSweeperXp>;

  const admin = Keypair.generate();
  const attestationAuthority = Keypair.generate();
  const treasury = Keypair.generate();
  const user = Keypair.generate();

  let mintPubkey: PublicKey;
  let emptyTokenAccount: PublicKey;
  let populatedTokenAccount: PublicKey;

  const [configPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("config")],
    program.programId
  );

  const [userProfilePda] = PublicKey.findProgramAddressSync(
    [Buffer.from("user_profile"), user.publicKey.toBuffer()],
    program.programId
  );

  before(async () => {
    // Airdrop SOL to test participants
    for (const target of [admin, user, treasury]) {
      const sig = await provider.connection.requestAirdrop(target.publicKey, 5 * anchor.web3.LAMPORTS_PER_SOL);
      const latestBlockhash = await provider.connection.getLatestBlockhash();
      await provider.connection.confirmTransaction({
        signature: sig,
        ...latestBlockhash,
      });
    }

    // Deploy mock SPL Token and create two accounts: one empty, one with positive balance
    mintPubkey = await createMint(
      provider.connection,
      admin,
      admin.publicKey,
      null,
      6
    );

    emptyTokenAccount = await createAccount(
      provider.connection,
      user,
      mintPubkey,
      user.publicKey
    );

    populatedTokenAccount = await createAccount(
      provider.connection,
      user,
      mintPubkey,
      user.publicKey
    );

    await mintTo(
      provider.connection,
      admin,
      mintPubkey,
      populatedTokenAccount,
      admin,
      1000000 // 1 token
    );
  });

  it("1. Initializes global protocol configuration", async () => {
    const feeBasisPoints = 500; // 5%

    await program.methods
      .initializeConfig(attestationAuthority.publicKey, feeBasisPoints)
      .accounts({
        config: configPda,
        treasury: treasury.publicKey,
        admin: admin.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([admin])
      .rpc();

    const config = await program.account.config.fetch(configPda);
    expect(config.admin.toBase58()).to.equal(admin.publicKey.toBase58());
    expect(config.treasury.toBase58()).to.equal(treasury.publicKey.toBase58());
    expect(config.attestationAuthority.toBase58()).to.equal(attestationAuthority.publicKey.toBase58());
    expect(config.feeBasisPoints).to.equal(500);
  });

  it("2. Initializes User Soulbound Profile PDA", async () => {
    await program.methods
      .initializeUserProfile()
      .accounts({
        userProfile: userProfilePda,
        user: user.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([user])
      .rpc();

    const profile = await program.account.userProfile.fetch(userProfilePda);
    expect(profile.owner.toBase58()).to.equal(user.publicKey.toBase58());
    expect(profile.experiencePoints.toNumber()).to.equal(0);
    expect(profile.accountsClosed.toNumber()).to.equal(0);
  });

  it("3. Successfully closes an empty account, splits rent, and awards base XP", async () => {
    const treasuryBalanceBefore = await provider.connection.getBalance(treasury.publicKey);
    const userBalanceBefore = await provider.connection.getBalance(user.publicKey);

    await program.methods
      .closeEmptyTokenAccount()
      .accounts({
        config: configPda,
        userProfile: userProfilePda,
        tokenAccount: emptyTokenAccount,
        user: user.publicKey,
        treasury: treasury.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .signers([user])
      .rpc();

    // Verify token account closed
    const accountInfo = await provider.connection.getAccountInfo(emptyTokenAccount);
    expect(accountInfo).to.be.null;

    // Verify user profile updated
    const profile = await program.account.userProfile.fetch(userProfilePda);
    expect(profile.accountsClosed.toNumber()).to.equal(1);
    expect(profile.experiencePoints.toNumber()).to.equal(50); // 50 XP awarded
    expect(profile.rentReclaimedLamports.toNumber()).to.be.greaterThan(0);

    // Verify treasury received fee split
    const treasuryBalanceAfter = await provider.connection.getBalance(treasury.publicKey);
    expect(treasuryBalanceAfter).to.be.greaterThan(treasuryBalanceBefore);
  });

  it("4. Fails to close a token account with non-zero dust balance", async () => {
    try {
      await program.methods
        .closeEmptyTokenAccount()
        .accounts({
          config: configPda,
          userProfile: userProfilePda,
          tokenAccount: populatedTokenAccount,
          user: user.publicKey,
          treasury: treasury.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .signers([user])
        .rpc();
      expect.fail("Should have thrown TokenAccountNotEmpty");
    } catch (err: any) {
      expect(err.toString()).to.include("TokenAccountNotEmpty");
    }
  });

  it("5. Successfully verifies Ed25519 quest signature and awards Social XP", async () => {
    const questIdStr = "QUEST_DISCORD_01";
    const questId = Buffer.alloc(16);
    Buffer.from(questIdStr).copy(questId);

    const xpAmount = new anchor.BN(300);
    const nonce = new anchor.BN(42);

    // Build payload: [32 user | 16 quest | 8 xp | 8 nonce]
    const xpBuf = Buffer.alloc(8);
    xpBuf.writeBigUInt64LE(BigInt(xpAmount.toString()));
    const nonceBuf = Buffer.alloc(8);
    nonceBuf.writeBigUInt64LE(BigInt(nonce.toString()));
    const message = Buffer.concat([user.publicKey.toBuffer(), questId, xpBuf, nonceBuf]);

    // Sign with attestation authority
    const sig = nacl.sign.detached(message, attestationAuthority.secretKey);

    const ed25519Ix = anchor.web3.Ed25519Program.createInstructionWithPublicKey({
      publicKey: attestationAuthority.publicKey.toBytes(),
      message,
      signature: sig,
    });

    const [nonceRecordPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("nonce_record"), user.publicKey.toBuffer(), nonceBuf],
      program.programId
    );

    const claimIx = await program.methods
      .claimSocialXp(
        Array.from(questId),
        xpAmount,
        nonce,
        Array.from(sig)
      )
      .accounts({
        config: configPda,
        userProfile: userProfilePda,
        nonceRecord: nonceRecordPda,
        user: user.publicKey,
        instructionSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
        systemProgram: SystemProgram.programId,
      })
      .instruction();

    const tx = new Transaction().add(ed25519Ix).add(claimIx);
    await sendAndConfirmTransaction(provider.connection, tx, [user]);

    const profile = await program.account.userProfile.fetch(userProfilePda);
    expect(profile.experiencePoints.toNumber()).to.equal(350); // 50 (from sweep) + 300 (quest)
    expect(profile.questsCompleted.toNumber()).to.equal(1);
  });

  it("6. Replay Attack Prevention: Fails when replaying identical nonce", async () => {
    const questId = Buffer.alloc(16);
    Buffer.from("QUEST_DISCORD_01").copy(questId);
    const xpAmount = new anchor.BN(300);
    const nonce = new anchor.BN(42); // Same nonce

    const xpBuf = Buffer.alloc(8);
    xpBuf.writeBigUInt64LE(BigInt(xpAmount.toString()));
    const nonceBuf = Buffer.alloc(8);
    nonceBuf.writeBigUInt64LE(BigInt(nonce.toString()));
    const message = Buffer.concat([user.publicKey.toBuffer(), questId, xpBuf, nonceBuf]);

    const sig = nacl.sign.detached(message, attestationAuthority.secretKey);
    const ed25519Ix = anchor.web3.Ed25519Program.createInstructionWithPublicKey({
      publicKey: attestationAuthority.publicKey.toBytes(),
      message,
      signature: sig,
    });

    const [nonceRecordPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("nonce_record"), user.publicKey.toBuffer(), nonceBuf],
      program.programId
    );

    const claimIx = await program.methods
      .claimSocialXp(
        Array.from(questId),
        xpAmount,
        nonce,
        Array.from(sig)
      )
      .accounts({
        config: configPda,
        userProfile: userProfilePda,
        nonceRecord: nonceRecordPda,
        user: user.publicKey,
        instructionSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
        systemProgram: SystemProgram.programId,
      })
      .instruction();

    const tx = new Transaction().add(ed25519Ix).add(claimIx);

    try {
      await sendAndConfirmTransaction(provider.connection, tx, [user]);
      expect.fail("Replay should have triggered account already in use");
    } catch (err: any) {
      expect(err.toString()).to.include("already in use");
    }
  });
});