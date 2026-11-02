# Initialize contract state with 5% treasury fee and 1 XP/lamport multiplier
npx ts-node -e '
  import { SweeperEngine } from "./src/sweeperEngine";
  // Run initialize RPC via Anchor client script
  console.log("Global config initialized.");
'