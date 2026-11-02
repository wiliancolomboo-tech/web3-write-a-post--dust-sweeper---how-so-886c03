# 1. Clone repository and install dependencies
git clone https://github.com/sponsor-org/solana-dust-sweeper.git
cd solana-dust-sweeper
yarn install

# 2. Build Anchor smart contract binaries
anchor build

# 3. Synchronize program keys
anchor keys sync

# 4. Deploy to Solana Devnet / Mainnet
solana config set --url https://api.devnet.solana.com
solana airdrop 2 ~/.config/solana/id.json
anchor deploy --provider.cluster Devnet

# 5. Initialize Protocol Global State
ts-node scripts/initialize-protocol.ts \
  --oracle-pubkey <ORACLE_ED25519_PUBKEY> \
  --fee-bps 250