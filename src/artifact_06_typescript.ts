import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { expect } from "chai";
import {
  Keypair,
  PublicKey,
  LAMPORTS_PER_SOL,
  SystemProgram,
} from "@solana/web3.js";
import {
  createMint,
  createAccount,
  mintTo,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";

describe("dust_sweeper_engine tests", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.DustSweeperEngine as Program<any>;

  const adminKeypair = Keypair.generate();
  const oracleKeypair = Keypair.generate();
  const userKeypair = Keypair.generate();

  let mintA: PublicKey;
  let userTokenAccountEmpty: PublicKey;
  let userTokenAccountFunded: PublicKey;

  let configPda: PublicKey;
  let userProfilePda: PublicKey;

  before(async () => {
    // Fund test wallets from provider
    const airdropTx1 = await provider.connection.requestAirdrop(
      adminKeypair.publicKey,
      5 * LAMPORTS_PER_SOL
    );
    const airdropTx2 = await provider.connection.requestAirdrop(
      userKeypair.publicKey,
      5 * LAMPORTS_PER_SOL
    );
    await provider.connection.confirmTransaction(airdropTx1);
    await provider.connection.confirmTransaction(airdropTx2);

    [configPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("config")],
      program.programId
    );

    [userProfilePda] = PublicKey.findProgramAddressSync(
      [Buffer.from("user_profile"), userKeypair.publicKey.toBuffer()],
      program.programId
    );

    // Create an SPL Token Mint for test scenarios
    mintA = await createMint(
      provider.connection,
      adminKeypair,
      adminKeypair.publicKey,
      null,
      6
    );

    // Create 1 empty token account for the user
    userTokenAccountEmpty = await createAccount(
      provider.connection,
      userKeypair,
      mintA,
      userKeypair.publicKey
    );

    // Create 1 funded token account for the user
    userTokenAccountFunded = await createAccount(
      provider.connection,
      userKeypair,
      mintA,
      userKeypair.publicKey
    );

    await mintTo(
      provider.connection,
      adminKeypair,
      mintA,
      userTokenAccountFunded,
      adminKeypair.publicKey,
      1_000_000 // 1 token
    );
  });

  it("1. Initializes the protocol configuration", async () => {
    await program.methods
      .initializeProtocol(oracleKeypair.publicKey, 250) // 2.5% max fee
      .accounts({
        config: configPda,
        admin: adminKeypair.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([adminKeypair])
      .rpc();

    const configAccount = await program.account.protocolConfig.fetch(configPda);
    expect(configAccount.admin.toBase58()).to.equal(adminKeypair.publicKey.toBase58());
    expect(configAccount.oracleAuthority.toBase58()).to.equal(oracleKeypair.publicKey.toBase58());
    expect(configAccount.feeBasisPoints).to.equal(250);
  });

  it("2. Initializes user quest profile", async () => {
    await program.methods
      .initializeUserProfile()
      .accounts({
        userProfile: userProfilePda,
        owner: userKeypair.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([userKeypair])
      .rpc();

    const profile = await program.account.userProfile.fetch(userProfilePda);
    expect(profile.owner.toBase58()).to.equal(userKeypair.publicKey.toBase58());
    expect(profile.xp.toNumber()).to.equal(0);
    expect(profile.tier).to.equal(1);
    expect(profile.accountsClosed).to.equal(0);
  });

  it("3. Fails to sweep a token account with remaining balance", async () => {
    try {
      await program.methods
        .sweepAndCloseAccount()
        .accounts({
          config: configPda,
          userProfile: userProfilePda,
          tokenAccount: userTokenAccountFunded,
          owner: userKeypair.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([userKeypair])
        .rpc();
      expect.fail("Should have thrown TokenAccountNotEmpty");
    } catch (err: any) {
      expect(err.toString()).to.include("TokenAccountNotEmpty");
    }
  });

  it("4. Successfully sweeps an empty account and earns XP", async () => {
    const balanceBefore = await provider.connection.getBalance(userKeypair.publicKey);

    const tx = await program.methods
      .sweepAndCloseAccount()
      .accounts({
        config: configPda,
        userProfile: userProfilePda,
        tokenAccount: userTokenAccountEmpty,
        owner: userKeypair.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([userKeypair])
      .rpc();

    const balanceAfter = await provider.connection.getBalance(userKeypair.publicKey);
    expect(balanceAfter).to.be.gt(balanceBefore); // Rent lamports reclaimed

    const profile = await program.account.userProfile.fetch(userProfilePda);
    expect(profile.accountsClosed).to.equal(1);
    expect(profile.xp.toNumber()).to.equal(25); // +25 XP
  });

  it("5. Completes a social quest and handles anti-replay", async () => {
    const questId = 3;
    const xpValue = 150;
    const dummySignature = new Array(64).fill(0);

    // Initial quest claim
    await program.methods
      .verifyAndCreditQuest(questId, new anchor.BN(xpValue), dummySignature)
      .accounts({
        config: configPda,
        userProfile: userProfilePda,
        oracleSigner: oracleKeypair.publicKey,
      })
      .signers([oracleKeypair])
      .rpc();

    let profile = await program.account.userProfile.fetch(userProfilePda);
    expect(profile.xp.toNumber()).to.equal(175); // 25 from sweep + 150 from quest
    expect(profile.tier).to.equal(2); // Advanced to Tier 2 (Scavenger)

    // Replay Attempt -> must fail
    try {
      await program.methods
        .verifyAndCreditQuest(questId, new anchor.BN(xpValue), dummySignature)
        .accounts({
          config: configPda,
          userProfile: userProfilePda,
          oracleSigner: oracleKeypair.publicKey,
        })
        .signers([oracleKeypair])
        .rpc();
      expect.fail("Should have failed on duplicate quest completion");
    } catch (err: any) {
      expect(err.toString()).to.include("QuestAlreadyCompleted");
    }
  });

  it("6. Rejects unauthenticated oracle quest submissions", async () => {
    const rogueKeypair = Keypair.generate();
    await provider.connection.requestAirdrop(rogueKeypair.publicKey, 1 * LAMPORTS_PER_SOL);

    try {
      await program.methods
        .verifyAndCreditQuest(4, new anchor.BN(100), new Array(64).fill(0))
        .accounts({
          config: configPda,
          userProfile: userProfilePda,
          oracleSigner: rogueKeypair.publicKey,
        })
        .signers([rogueKeypair])
        .rpc();
      expect.fail("Should have rejected rogue oracle");
    } catch (err: any) {
      expect(err.toString()).to.include("InvalidOracleAuthority");
    }
  });
});