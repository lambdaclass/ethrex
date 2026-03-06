import {
  createWalletClient,
  createPublicClient,
  http,
  encodeFunctionData,
  defineChain,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { ACCOUNT_ADDRESS, MOCK_ERC20_ADDRESS } from "./types.js";

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

/**
 * Call setPublicKey(uint256 x, uint256 y) on the WebAuthnP256Account.
 * Uses the dev account to sign and send a regular (EIP-1559) transaction.
 */
export async function setPublicKey(
  x: bigint,
  y: bigint
): Promise<`0x${string}`> {
  const data = encodeFunctionData({
    abi: [
      {
        type: "function",
        name: "setPublicKey",
        inputs: [
          { name: "x", type: "uint256" },
          { name: "y", type: "uint256" },
        ],
        outputs: [],
        stateMutability: "nonpayable",
      },
    ],
    functionName: "setPublicKey",
    args: [x, y],
  });

  const txHash = await walletClient.sendTransaction({
    to: ACCOUNT_ADDRESS as `0x${string}`,
    data,
  });

  // Wait for confirmation
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
