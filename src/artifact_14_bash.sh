# 1. Clone repository and build Solana Anchor Program
git clone https://github.com/solsweep/solsweep-core.git
cd solsweep-core
anchor build

# 2. Synchronize Program ID with target deploy key
solana address -k target/deploy/solsweep_core-keypair.json
# (Replace declare_id! in src/lib.rs if re-generating)

# 3. Deploy to Solana Devnet / Mainnet
solana program deploy \
  --program-id target/deploy/solsweep_core-keypair.json \
  target/deploy/solsweep_core.so \
  --url devnet

# 4. Initialize Protocol PDA
anchor run initialize --provider.cluster devnet

# 5. Launch CLI Dust Sweep & Rent Reclaim
npm install
npm run build
node dist/cli.js sweep --vault <VAULT_TOKEN_ACCOUNT_PUBKEY>

# 6. Verify Social Quests & Earn XP
node dist/cli.js claim-quest --quest 1 --xp 250