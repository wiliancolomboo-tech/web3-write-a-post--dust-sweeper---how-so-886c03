# 1. Install prerequisites
sh -c "$(curl -sSfL https://release.anza.xyz/stable/install)"
cargo install --git https://github.com/coral-xyz/anchor avm --locked --force
avm install 0.30.1
avm use 0.30.1

# 2. Build Anchor Artifacts
anchor build

# 3. Synchronize Program ID
solana address -k target/deploy/dust_sweeper-keypair.json
# (Replace declare_id! in programs/dust-sweeper/src/lib.rs with this pubkey)

# 4. Deploy to Solana Devnet / Mainnet
solana config set --url https://api.devnet.solana.com
solana airdrop 2
anchor deploy --provider.cluster devnet

# 5. Initialize Registry and Seed Oracle
ts-node src/cli.ts initialize