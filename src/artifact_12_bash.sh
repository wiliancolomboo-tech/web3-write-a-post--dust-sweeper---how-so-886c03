# 1. Build verified Anchor binary
anchor build

# 2. Deploy program to Solana Devnet
solana program deploy \
  --program-id SweepXPW1nner1111111111111111111111111111111 \
  --url https://api.devnet.solana.com \
  target/deploy/dust_sweeper.so

# 3. Initialize protocol state with oracle authority and treasury
ts-node scripts/initialize.ts \
  --oracle-authority <ORACLE_PUBKEY> \
  --treasury <TREASURY_PUBKEY> \
  --fee-lamports 5000