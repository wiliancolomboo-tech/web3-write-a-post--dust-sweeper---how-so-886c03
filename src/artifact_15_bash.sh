# 1. Build optimized BPF program binary
anchor build

# 2. Deploy to Devnet / Mainnet-Beta
solana program deploy target/deploy/dust_sweeper.so \
  --program-id target/deploy/dust_sweeper-keypair.json \
  --url https://api.devnet.solana.com

# 3. Initialize Protocol State
ts-node scripts/init-protocol.ts \
  --authority-keypair ~/.config/solana/quest-authority.json \
  --fee-collector ~/.config/solana/treasury.json \
  --fee-bps 50