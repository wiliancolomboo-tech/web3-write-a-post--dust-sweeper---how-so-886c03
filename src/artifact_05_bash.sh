# Configure Solana CLI to Target Cluster (e.g. devnet)
solana config set --url https://api.devnet.solana.com

# Build program binaries with verifiable Anchor build
anchor build

# Deploy program
solana program deploy ./target/deploy/sweep_xp.so --program-id ./target/deploy/sweep_xp-keypair.json