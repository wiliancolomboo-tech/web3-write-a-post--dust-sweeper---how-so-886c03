# Set Solana CLI config to Devnet or Mainnet
solana config set --url https://api.devnet.solana.com

# Build program binaries with deterministic Verifiable BPF
anchor build

# Sync Program ID into lib.rs and Anchor.toml
anchor keys sync

# Deploy Anchor bytecode
anchor deploy --provider.cluster devnet