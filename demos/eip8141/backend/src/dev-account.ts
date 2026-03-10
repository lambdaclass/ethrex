import {
  createWalletClient,
  createPublicClient,
  http,
  encodeFunctionData,
  defineChain,
  parseEther,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { MOCK_ERC20_ADDRESS, FACTORY_ADDRESS } from "./types.js";
import * as fs from "fs";
import * as path from "path";
import { fileURLToPath } from "url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const RPC_URL = process.env.RPC_URL ?? "http://localhost:8545";

// Hardhat account #0 — well-known dev key, pre-funded in genesis
const DEV_PRIVATE_KEY =
  "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80" as const;

const devAccount = privateKeyToAccount(DEV_PRIVATE_KEY);

const demoChain = defineChain({
  id: 1729,
  name: "ethrex-demo",
  nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
  rpcUrls: {
    default: { http: [RPC_URL] },
  },
});

const walletClient = createWalletClient({
  account: devAccount,
  chain: demoChain,
  transport: http(RPC_URL),
});

const publicClient = createPublicClient({
  chain: demoChain,
  transport: http(RPC_URL),
});

// ── Account initcode (loaded from compiled Yul output) ──

let accountInitcode: `0x${string}` | null = null;

function loadAccountInitcode(): `0x${string}` {
  if (accountInitcode) return accountInitcode;

  // Try loading from the compiled output
  const candidates = [
    path.resolve(__dirname, "../../contracts/out/WebAuthnP256Account.initcode.hex"),
    path.resolve(__dirname, "../../../contracts/out/WebAuthnP256Account.initcode.hex"),
  ];

  for (const p of candidates) {
    try {
      const hex = fs.readFileSync(p, "utf8").trim();
      if (hex.startsWith("0x") && hex.length > 10) {
        accountInitcode = hex as `0x${string}`;
        console.log(`[dev-account] Loaded account initcode from ${p} (${(hex.length - 2) / 2} bytes)`);
        return accountInitcode;
      }
    } catch { /* continue */ }
  }

  throw new Error(
    "WebAuthnP256Account initcode not found. Run 'make genesis' in the demo directory first."
  );
}

// ── Factory ABI ──

const factoryAbi = [
  {
    type: "function",
    name: "initialized",
    inputs: [],
    outputs: [{ name: "", type: "bool" }],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "initialize",
    inputs: [{ name: "initcode_", type: "bytes" }],
    outputs: [],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "deploy",
    inputs: [
      { name: "pubKeyX", type: "uint256" },
      { name: "pubKeyY", type: "uint256" },
    ],
    outputs: [{ name: "account", type: "address" }],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "getAddress",
    inputs: [
      { name: "pubKeyX", type: "uint256" },
      { name: "pubKeyY", type: "uint256" },
    ],
    outputs: [{ name: "", type: "address" }],
    stateMutability: "view",
  },
] as const;

/**
 * Ensure the factory has been initialized with the account initcode.
 * Called once at backend startup. Idempotent — skips if already initialized.
 */
export async function ensureFactoryInitialized(): Promise<void> {
  const initcode = loadAccountInitcode();

  const isInit = await publicClient.readContract({
    address: FACTORY_ADDRESS as `0x${string}`,
    abi: factoryAbi,
    functionName: "initialized",
  });

  if (isInit) {
    console.log("[dev-account] Factory already initialized");
    return;
  }

  console.log("[dev-account] Initializing factory with account initcode...");
  const data = encodeFunctionData({
    abi: factoryAbi,
    functionName: "initialize",
    args: [initcode],
  });

  const txHash = await walletClient.sendTransaction({
    to: FACTORY_ADDRESS as `0x${string}`,
    data,
  });
  await publicClient.waitForTransactionReceipt({ hash: txHash });
  console.log(`[dev-account] Factory initialized, tx: ${txHash}`);
}

/**
 * Deploy a new WebAuthnP256Account via the factory.
 * Returns the deployed account address and the deploy transaction hash.
 */
export async function deployAccount(
  pubKeyX: bigint,
  pubKeyY: bigint
): Promise<{ address: `0x${string}`; txHash: `0x${string}` }> {
  const data = encodeFunctionData({
    abi: factoryAbi,
    functionName: "deploy",
    args: [pubKeyX, pubKeyY],
  });

  const txHash = await walletClient.sendTransaction({
    to: FACTORY_ADDRESS as `0x${string}`,
    data,
  });

  const receipt = await publicClient.waitForTransactionReceipt({ hash: txHash });

  // Extract the deployed account address from the AccountDeployed event log
  // Event: AccountDeployed(address indexed account, uint256 pubKeyX, uint256 pubKeyY)
  // Topic 0 = event signature hash, Topic 1 = account address
  const deployLog = receipt.logs.find(
    (log) => log.address.toLowerCase() === FACTORY_ADDRESS.toLowerCase() && log.topics.length >= 2
  );

  if (!deployLog || !deployLog.topics[1]) {
    throw new Error("AccountDeployed event not found in receipt");
  }

  // Topic 1 is the indexed address, padded to 32 bytes
  const accountAddress = ("0x" + deployLog.topics[1].slice(26)) as `0x${string}`;
  return { address: accountAddress, txHash };
}

/**
 * Fund an account with ETH from the dev account.
 */
export async function fundAccount(
  address: string,
  amount: bigint = parseEther("10")
): Promise<`0x${string}`> {
  const txHash = await walletClient.sendTransaction({
    to: address as `0x${string}`,
    value: amount,
  });
  await publicClient.waitForTransactionReceipt({ hash: txHash });
  return txHash;
}

/**
 * Call MockERC20.mint(to, amount) via the dev account.
 */
export async function mintTokens(
  to: string,
  amount: bigint
): Promise<`0x${string}`> {
  const data = encodeFunctionData({
    abi: [
      {
        type: "function",
        name: "mint",
        inputs: [
          { name: "to", type: "address" },
          { name: "amount", type: "uint256" },
        ],
        outputs: [],
        stateMutability: "nonpayable",
      },
    ],
    functionName: "mint",
    args: [to as `0x${string}`, amount],
  });

  const txHash = await walletClient.sendTransaction({
    to: MOCK_ERC20_ADDRESS as `0x${string}`,
    data,
  });

  await publicClient.waitForTransactionReceipt({ hash: txHash });
  return txHash;
}

/**
 * Read MockERC20.balanceOf(address) via eth_call.
 */
export async function getTokenBalance(address: string): Promise<bigint> {
  const data = encodeFunctionData({
    abi: [
      {
        type: "function",
        name: "balanceOf",
        inputs: [{ name: "account", type: "address" }],
        outputs: [{ name: "", type: "uint256" }],
        stateMutability: "view",
      },
    ],
    functionName: "balanceOf",
    args: [address as `0x${string}`],
  });

  const result = await publicClient.call({
    to: MOCK_ERC20_ADDRESS as `0x${string}`,
    data,
  });

  return result.data ? BigInt(result.data) : 0n;
}

export { devAccount, DEV_PRIVATE_KEY };
