# 1. Install Solana CLI & Anchor
sh -c "$(curl -sSfL https://release.solana.com/v1.18.26/install)"
cargo install --git https://github.com/coral-xyz/anchor --tag v0.30.1 anchor-cli --locked

# 2. Build program binary
anchor build

# 3. Synchronize program keys
anchor keys sync

# 4. Deploy to Solana Devnet / Mainnet
solana config set --url https://api.devnet.solana.com
solana airdrop 2
anchor deploy --provider.cluster devnet