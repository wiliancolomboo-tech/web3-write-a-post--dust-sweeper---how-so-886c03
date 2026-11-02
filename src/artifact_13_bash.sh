# 1. Build Anchor Program binaries
anchor build

# 2. Synchronize Program ID with target keypair
solana address -k target/deploy/dust_sweeper_xp-keypair.json
# Ensure declare_id! in src/lib.rs matches the output above

# 3. Deploy to Solana Devnet / Mainnet-Beta
solana program deploy \
  --url https://api.devnet.solana.com \
  --program-id target/deploy/dust_sweeper_xp-keypair.json \
  target/deploy/dust_sweeper_xp.so

# 4. Initialize Program Configuration
ts-node client/src/index.ts initialize \
  --admin-keypair ~/.config/solana/id.json \
  --treasury <TREASURY_PUBKEY> \
  --fee-bps 500