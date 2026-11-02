# 1. Build and compile the Anchor program with verifiable BPF build
anchor build

# 2. Sync program ID across declare_id! and Anchor.toml
anchor keys sync

# 3. Deploy to Solana Devnet or Mainnet-Beta
solana config set --url https://api.devnet.solana.com
solana airdrop 2
anchor deploy --provider.cluster devnet

# 4. Initialize protocol configuration via CLI
yarn ts-node src/cli.ts issue-quest --help