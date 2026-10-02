# 1. Build smart contract binary
anchor build

# 2. Deploy to Solana Devnet / Mainnet
solana config set --url https://api.devnet.solana.com
solana airdrop 2
anchor deploy --program-name dust_sweeper_xp

# 3. Initialize Program Configuration
ts-node scripts/init-protocol.ts \
  --admin ~/.config/solana/id.json \
  --oracle ./keys/oracle-keypair.json \
  --fee 250