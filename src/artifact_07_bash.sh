# 1. Build optimized binary with reproducible verification flags
anchor build --verifiable

# 2. Configure target network (devnet / testnet / mainnet-beta)
solana config set --url https://api.devnet.solana.com
solana airdrop 2

# 3. Deploy the program binary
solana program deploy target/deploy/dust_sweeper_engine.so

# 4. Initialize protocol configuration
anchor run init-protocol -- --oracle <ORACLE_PUBKEY> --fee 200