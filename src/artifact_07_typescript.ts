import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { DustSweeperXp } from "../target/types/dust_sweeper_xp";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  sendAndConfirmTransaction,
  Ed25519Program,
  SYSVAR_INSTRUCTIONS_PUBKEY,
  LAMPORTS_PER_SOL,
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

describe("dust_sweeper_xp test suite", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.DustSweeperXp as Program<DustSweeperXp>;
  const admin = Keypair.generate();
  const oracle = Keypair.generate();
  const user = Keypair.generate();

  let mintPubkey: PublicKey;
  let userTokenAccount: PublicKey;
  let profilePda: PublicKey;
  let configPda: PublicKey;

  before(async () => {
    // Airdrop SOL to test actors
    for (const actor of [admin, oracle, user]) {
      const airdropSig = await provider.connection.requestAirdrop(
        actor.publicKey,
        5 * LAMPORTS_PER_SOL
      );
      const latestBlockhash = await provider.connection.getLatestBlockhash();
      await provider.connection.confirmTransaction({
        signature: airdropSig,
        ...latestBlockhash,
      });
    }

    [configPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("global_config")],
      program.programId
    );

    [profilePda] = PublicKey.findProgramAddressSync(
      [Buffer.from("user_profile"), user.publicKey.toBuffer()],
      program.programId
    );

    // Initialize Mint & User Token Account
    mintPubkey = await createMint(
      provider.connection,
      admin,
      admin.publicKey,
      null,
      6
    );

    userTokenAccount = await createAccount(
      provider.connection,
      user,
      mintPubkey,
      user.publicKey
    );

    // Mint dust (5 units) into the user account
    await mintTo(
      provider.connection,
      admin,
      mintPubkey,
      userTokenAccount,
      admin,
      5
    );
  });

  it("1. Initializes global protocol configuration", async () => {
    await program.methods
      .initializeProtocol(oracle.publicKey, 250) // 2.5% fee
      .accounts({
        config: configPda,
        admin: admin.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([admin])
      .rpc();

    const configAccount = await program.account.protocolConfig.fetch(configPda);
    expect(configAccount.admin.toBase58()).to.equal(admin.publicKey.toBase58());
    expect(configAccount.oracleSigner.toBase58()).to.equal(oracle.publicKey.toBase58());
    expect(configAccount.feeBasisPoints).to.equal(250);
  });

  it("2. Initializes user profile PDA", async () => {
    await program.methods
      .initializeUserProfile()
      .accounts({
        profile: profilePda,
        user: user.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([user])
      .rpc();

    const profile = await program.account.userProfile.fetch(profilePda);
    expect(profile.owner.toBase58()).to.equal(user.publicKey.toBase58());
    expect(profile.totalXp.toNumber()).to.equal(0);
    expect(profile.accountsSwept.toNumber()).to.equal(0);
  });

  it("3. Sweeps dust, burns residual tokens, closes account, and refunds rent", async () => {
    const preBal = await provider.connection.getBalance(user.publicKey);

    await program.methods
      .sweepDustAndCloseAccount()
      .accounts({
        profile: profilePda,
        config: configPda,
        tokenAccount: userTokenAccount,
        tokenMint: mintPubkey,
        user: user.publicKey,
        owner: user.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([user])
      .rpc();

    // Verify token account is now closed
    const accountInfo = await provider.connection.getAccountInfo(userTokenAccount);
    expect(accountInfo).to.be.null;

    const postBal = await provider.connection.getBalance(user.publicKey);
    expect(postBal).to.be.greaterThan(preBal - 1000000); // User recovered ~0.002039 SOL rent

    const profile = await program.account.userProfile.fetch(profilePda);
    expect(profile.accountsSwept.toNumber()).to.equal(1);
    expect(profile.totalXp.toNumber()).to.equal(50);
  });

  it("4. Successfully claims social quest with valid Ed25519 Oracle signature", async () => {
    const questId = 3; // e.g., "Join Official Discord Guild"
    const xpReward = new anchor.BN(500);
    const nonce = new anchor.BN(1001);

    const message = Buffer.concat([
      user.publicKey.toBuffer(),
      Buffer.from([questId]),
      xpReward.toArrayLike(Buffer, "le", 8),
      nonce.toArrayLike(Buffer, "le", 8),
    ]);

    const signature = nacl.sign.detached(message, oracle.secretKey);

    const ed25519Ix = Ed25519Program.createInstructionWithPublicKey({
      publicKey: oracle.publicKey.toBytes(),
      message,
      signature,
    });

    const claimIx = await program.methods
      .verifyAndClaimSocialQuest(questId, xpReward, nonce, Array.from(signature))
      .accounts({
        profile: profilePda,
        config: configPda,
        user: user.publicKey,
        ixSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
      })
      .instruction();

    const tx = new Transaction().add(ed25519Ix).add(claimIx);
    await sendAndConfirmTransaction(provider.connection, tx, [user]);

    const profile = await program.account.userProfile.fetch(profilePda);
    expect(profile.totalXp.toNumber()).to.equal(550); // 50 (from sweep) + 500
    expect((profile.completedQuestsBitmask.toNumber() & (1 << questId))).to.not.equal(0);
  });

  it("5. Prevents replay attack on previously claimed quest", async () => {
    const questId = 3; // Attempting to claim the same quest
    const xpReward = new anchor.BN(500);
    const nonce = new anchor.BN(1002);

    const message = Buffer.concat([
      user.publicKey.toBuffer(),
      Buffer.from([questId]),
      xpReward.toArrayLike(Buffer, "le", 8),
      nonce.toArrayLike(Buffer, "le", 8),
    ]);

    const signature = nacl.sign.detached(message, oracle.secretKey);

    const ed25519Ix = Ed25519Program.createInstructionWithPublicKey({
      publicKey: oracle.publicKey.toBytes(),
      message,
      signature,
    });

    const claimIx = await program.methods
      .verifyAndClaimSocialQuest(questId, xpReward, nonce, Array.from(signature))
      .accounts({
        profile: profilePda,
        config: configPda,
        user: user.publicKey,
        ixSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
      })
      .instruction();

    const tx = new Transaction().add(ed25519Ix).add(claimIx);

    try {
      await sendAndConfirmTransaction(provider.connection, tx, [user]);
      expect.fail("Transaction should have failed with QuestAlreadyCompleted");
    } catch (err: any) {
      expect(err.toString()).to.include("QuestAlreadyCompleted");
    }
  });

  it("6. Rejects quest claims with unauthorized rogue oracle signatures", async () => {
    const rogueOracle = Keypair.generate();
    const questId = 4;
    const xpReward = new anchor.BN(250);
    const nonce = new anchor.BN(2001);

    const message = Buffer.concat([
      user.publicKey.toBuffer(),
      Buffer.from([questId]),
      xpReward.toArrayLike(Buffer, "le", 8),
      nonce.toArrayLike(Buffer, "le", 8),
    ]);

    const rogueSignature = nacl.sign.detached(message, rogueOracle.secretKey);

    const ed25519Ix = Ed25519Program.createInstructionWithPublicKey({
      publicKey: rogueOracle.publicKey.toBytes(),
      message,
      signature: rogueSignature,
    });

    const claimIx = await program.methods
      .verifyAndClaimSocialQuest(questId, xpReward, nonce, Array.from(rogueSignature))
      .accounts({
        profile: profilePda,
        config: configPda,
        user: user.publicKey,
        ixSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
      })
      .instruction();

    const tx = new Transaction().add(ed25519Ix).add(claimIx);

    try {
      await sendAndConfirmTransaction(provider.connection, tx, [user]);
      expect.fail("Transaction should have failed with UnauthorizedOracle");
    } catch (err: any) {
      expect(err.toString()).to.include("UnauthorizedOracle");
    }
  });
});