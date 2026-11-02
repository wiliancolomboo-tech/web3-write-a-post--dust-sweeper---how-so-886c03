import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { DustSweeperQuests } from "../target/types/dust_sweeper_quests";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_INSTRUCTIONS_PUBKEY,
  Ed25519Program,
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

describe("dust_sweeper_quests", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.DustSweeperQuests as Program<DustSweeperQuests>;

  const admin = Keypair.generate();
  const oracle = Keypair.generate();
  const user = Keypair.generate();

  let mint: PublicKey;
  let userTokenAccount: PublicKey;

  const [configPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("config_state")],
    program.programId
  );
  const [profilePda] = PublicKey.findProgramAddressSync(
    [Buffer.from("user_profile"), user.publicKey.toBuffer()],
    program.programId
  );

  before(async () => {
    // Airdrop SOL to participants
    const airdropAdmin = await provider.connection.requestAirdrop(admin.publicKey, 5e9);
    const airdropUser = await provider.connection.requestAirdrop(user.publicKey, 5e9);
    await provider.connection.confirmTransaction(airdropAdmin);
    await provider.connection.confirmTransaction(airdropUser);

    // Initialize Mint & User ATA
    mint = await createMint(provider.connection, admin, admin.publicKey, null, 6);
    userTokenAccount = await createAccount(provider.connection, user, mint, user.publicKey);

    // Mint dust (50 units) to user ATA
    await mintTo(provider.connection, admin, mint, userTokenAccount, admin, 50);
  });

  it("Initializes protocol config correctly", async () => {
    await program.methods
      .initializeProtocol(oracle.publicKey, new anchor.BN(100))
      .accounts({
        config: configPda,
        admin: admin.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([admin])
      .rpc();

    const configAccount = await program.account.configState.fetch(configPda);
    expect(configAccount.admin.toBase58()).to.equal(admin.publicKey.toBase58());
    expect(configAccount.oracleSigner.toBase58()).to.equal(oracle.publicKey.toBase58());
    expect(configAccount.xpPerSweep.toNumber()).to.equal(100);
  });

  it("Initializes user profile state correctly", async () => {
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
    expect(profile.authority.toBase58()).to.equal(user.publicKey.toBase58());
    expect(profile.xpBalance.toNumber()).to.equal(0);
    expect(profile.totalAccountsClosed).to.equal(0);
    expect(profile.questNonce.toNumber()).to.equal(0);
  });

  it("Sweeps dust, burns balance, closes ATA, awards XP, and reclaims rent", async () => {
    const preBal = await provider.connection.getBalance(user.publicKey);

    await program.methods
      .sweepAndCloseAccount(new anchor.BN(100))
      .accounts({
        config: configPda,
        profile: profilePda,
        tokenAccount: userTokenAccount,
        mint: mint,
        user: user.publicKey,
        authority: user.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .signers([user])
      .rpc();

    const postBal = await provider.connection.getBalance(user.publicKey);
    expect(postBal).to.be.greaterThan(preBal); // Rent reclaimed exceeds gas cost

    const profile = await program.account.userProfile.fetch(profilePda);
    expect(profile.totalAccountsClosed).to.equal(1);
    expect(profile.xpBalance.toNumber()).to.equal(100);
    expect(profile.totalRentReclaimedLamports.toNumber()).to.be.greaterThan(0);

    // Verify token account no longer exists
    const closedAccountInfo = await provider.connection.getAccountInfo(userTokenAccount);
    expect(closedAccountInfo).to.be.null;
  });

  it("Claims a social quest with a valid Ed25519 oracle voucher", async () => {
    const questId = 1; // e.g. Follow X Protocol Account
    const xpReward = 500;
    const nonce = 0;
    const expiry = Math.floor(Date.now() / 1000) + 1200;

    const message = Buffer.concat([
      user.publicKey.toBuffer(),
      Buffer.from([questId]),
      new anchor.BN(xpReward).toArrayLike(Buffer, "le", 8),
      new anchor.BN(nonce).toArrayLike(Buffer, "le", 8),
      new anchor.BN(expiry).toArrayLike(Buffer, "le", 8),
    ]);

    const signature = nacl.sign.detached(message, oracle.secretKey);

    const ed25519Ix = Ed25519Program.createInstructionWithPublicKey({
      publicKey: oracle.publicKey.toBytes(),
      message: message,
      signature: signature,
    });

    const claimIx = await program.methods
      .claimSocialQuest(
        questId,
        new anchor.BN(xpReward),
        new anchor.BN(nonce),
        new anchor.BN(expiry),
        Array.from(signature)
      )
      .accounts({
        config: configPda,
        profile: profilePda,
        user: user.publicKey,
        authority: user.publicKey,
        sysvarInstructions: SYSVAR_INSTRUCTIONS_PUBKEY,
      })
      .instruction();

    const tx = new Transaction().add(ed25519Ix).add(claimIx);
    await provider.sendAndConfirm(tx, [user]);

    const profile = await program.account.userProfile.fetch(profilePda);
    expect(profile.xpBalance.toNumber()).to.equal(600); // 100 from sweep + 500 from quest
    expect(profile.questNonce.toNumber()).to.equal(1);
  });

  it("Rejects replay attack for already claimed quest ID", async () => {
    const questId = 1;
    const xpReward = 500;
    const nonce = 1; // Current nonce
    const expiry = Math.floor(Date.now() / 1000) + 1200;

    const message = Buffer.concat([
      user.publicKey.toBuffer(),
      Buffer.from([questId]),
      new anchor.BN(xpReward).toArrayLike(Buffer, "le", 8),
      new anchor.BN(nonce).toArrayLike(Buffer, "le", 8),
      new anchor.BN(expiry).toArrayLike(Buffer, "le", 8),
    ]);

    const signature = nacl.sign.detached(message, oracle.secretKey);

    const ed25519Ix = Ed25519Program.createInstructionWithPublicKey({
      publicKey: oracle.publicKey.toBytes(),
      message: message,
      signature: signature,
    });

    const claimIx = await program.methods
      .claimSocialQuest(
        questId,
        new anchor.BN(xpReward),
        new anchor.BN(nonce),
        new anchor.BN(expiry),
        Array.from(signature)
      )
      .accounts({
        config: configPda,
        profile: profilePda,
        user: user.publicKey,
        authority: user.publicKey,
        sysvarInstructions: SYSVAR_INSTRUCTIONS_PUBKEY,
      })
      .instruction();

    const tx = new Transaction().add(ed25519Ix).add(claimIx);
    try {
      await provider.sendAndConfirm(tx, [user]);
      expect.fail("Expected transaction to fail due to duplicate quest completion");
    } catch (err: any) {
      expect(err.toString()).to.include("QuestAlreadyCompleted");
    }
  });

  it("Rejects quest voucher signed by unauthorized rogue keypair", async () => {
    const rogueOracle = Keypair.generate();
    const questId = 2;
    const xpReward = 250;
    const nonce = 1;
    const expiry = Math.floor(Date.now() / 1000) + 1200;

    const message = Buffer.concat([
      user.publicKey.toBuffer(),
      Buffer.from([questId]),
      new anchor.BN(xpReward).toArrayLike(Buffer, "le", 8),
      new anchor.BN(nonce).toArrayLike(Buffer, "le", 8),
      new anchor.BN(expiry).toArrayLike(Buffer, "le", 8),
    ]);

    const signature = nacl.sign.detached(message, rogueOracle.secretKey);

    const ed25519Ix = Ed25519Program.createInstructionWithPublicKey({
      publicKey: rogueOracle.publicKey.toBytes(),
      message: message,
      signature: signature,
    });

    const claimIx = await program.methods
      .claimSocialQuest(
        questId,
        new anchor.BN(xpReward),
        new anchor.BN(nonce),
        new anchor.BN(expiry),
        Array.from(signature)
      )
      .accounts({
        config: configPda,
        profile: profilePda,
        user: user.publicKey,
        authority: user.publicKey,
        sysvarInstructions: SYSVAR_INSTRUCTIONS_PUBKEY,
      })
      .instruction();

    const tx = new Transaction().add(ed25519Ix).add(claimIx);
    try {
      await provider.sendAndConfirm(tx, [user]);
      expect.fail("Expected transaction to fail due to signature from rogue keypair");
    } catch (err: any) {
      expect(err.toString()).to.include("InvalidOracleSignature");
    }
  });
});