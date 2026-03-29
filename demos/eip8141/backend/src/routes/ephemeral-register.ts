import { Hono } from "hono";
import { streamSSE } from "hono/streaming";
import {
  createWalletClient,
  createPublicClient,
  http,
  encodeFunctionData,
  defineChain,
  parseEther,
  keccak256,
  encodePacked,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";
import {
  SIGNER_REGISTRY_ADDRESS,
  MOCK_ERC20_ADDRESS,
} from "../types.js";
import { fundAccount, mintTokens, DEV_PRIVATE_KEY } from "../dev-account.js";
import * as fs from "fs";
import * as path from "path";
import { fileURLToPath } from "url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const RPC_URL = process.env.RPC_URL ?? "http://localhost:8545";

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

// ── EphemeralKeyAccount initcode ──

let ekaInitcode: `0x${string}` | null = null;

function loadEkaInitcode(): `0x${string}` {
  if (ekaInitcode) return ekaInitcode;

  const candidates = [
    path.resolve(__dirname, "../../contracts/out/EphemeralKeyAccount.initcode.hex"),
    path.resolve(__dirname, "../../../contracts/out/EphemeralKeyAccount.initcode.hex"),
  ];

  for (const p of candidates) {
    try {
      const hex = fs.readFileSync(p, "utf8").trim();
      if (hex.startsWith("0x") && hex.length > 10) {
        ekaInitcode = hex as `0x${string}`;
        console.log(`[ephemeral] Loaded EphemeralKeyAccount initcode from ${p} (${(hex.length - 2) / 2} bytes)`);
        return ekaInitcode;
      }
    } catch { /* continue */ }
  }

  throw new Error(
    "EphemeralKeyAccount initcode not found. Run 'make genesis' in the demo directory first."
  );
}

// ── Key derivation ──

export function deriveKey(seed: `0x${string}`, index: number): { privateKey: `0x${string}`; address: `0x${string}` } {
  const privKey = keccak256(
    encodePacked(["bytes32", "uint256"], [seed, BigInt(index)])
  );
  const account = privateKeyToAccount(privKey);
  return { privateKey: privKey, address: account.address.toLowerCase() as `0x${string}` };
}

// ── Per-account ephemeral state ──

export interface EphemeralState {
  seed: `0x${string}`;
  keyIndex: number;
  address: `0x${string}`;
}

export const ephemeralAccounts = new Map<string, EphemeralState>();

// ── SSE helpers ──

function sseStep(step: string, status: string, extra?: Record<string, string>) {
  return { event: "step", data: JSON.stringify({ step, status, ...extra }) };
}

// ── Route ──

const app = new Hono();

app.post("/ephemeral-register", async (c) => {
  return streamSSE(c, async (stream) => {
    // Step 1: Generate seed and derive initial key
    const seedBytes = new Uint8Array(32);
    crypto.getRandomValues(seedBytes);
    const seed = ("0x" + Array.from(seedBytes).map(b => b.toString(16).padStart(2, "0")).join("")) as `0x${string}`;

    const key0 = deriveKey(seed, 0);
    console.log(`[ephemeral-register] Derived key0: ${key0.address}`);

    // Step 2: Deploy EphemeralKeyAccount
    await stream.writeSSE(sseStep("deploy", "pending"));

    const initcode = loadEkaInitcode();
    // Append registry address as 32-byte constructor arg
    const registryPadded = SIGNER_REGISTRY_ADDRESS.slice(2).padStart(64, "0");
    const deployData = (initcode + registryPadded) as `0x${string}`;

    const deployTxHash = await walletClient.sendTransaction({
      data: deployData,
    });
    const deployReceipt = await publicClient.waitForTransactionReceipt({ hash: deployTxHash });
    const accountAddress = deployReceipt.contractAddress;

    if (!accountAddress) {
      throw new Error("Contract deployment failed — no contract address in receipt");
    }

    console.log(`[ephemeral-register] Account deployed at ${accountAddress}`);
    await stream.writeSSE(sseStep("deploy", "done", { address: accountAddress, txHash: deployTxHash }));

    // Step 3: Fund with ETH (needed before registering signer via execute)
    await stream.writeSSE(sseStep("fund", "pending"));
    const fundTxHash = await fundAccount(accountAddress, parseEther("10"));
    console.log(`[ephemeral-register] Funded with 10 ETH, tx: ${fundTxHash}`);
    await stream.writeSSE(sseStep("fund", "done", { txHash: fundTxHash }));

    // Step 4: Register initial signer — call account.execute(registry, 0, rotate(key0))
    await stream.writeSSE(sseStep("register-signer", "pending"));

    const rotateCalldata = encodeFunctionData({
      abi: [{
        type: "function",
        name: "rotate",
        inputs: [{ name: "nextSigner", type: "address" }],
        outputs: [],
        stateMutability: "nonpayable",
      }],
      functionName: "rotate",
      args: [key0.address as `0x${string}`],
    });

    const executeCalldata = encodeFunctionData({
      abi: [{
        type: "function",
        name: "execute",
        inputs: [
          { name: "to", type: "address" },
          { name: "value", type: "uint256" },
          { name: "data", type: "bytes" },
        ],
        outputs: [],
        stateMutability: "nonpayable",
      }],
      functionName: "execute",
      args: [SIGNER_REGISTRY_ADDRESS as `0x${string}`, 0n, rotateCalldata],
    });

    const registerTxHash = await walletClient.sendTransaction({
      to: accountAddress as `0x${string}`,
      data: executeCalldata,
    });
    await publicClient.waitForTransactionReceipt({ hash: registerTxHash });
    console.log(`[ephemeral-register] Signer registered: ${key0.address}, tx: ${registerTxHash}`);
    await stream.writeSSE(sseStep("register-signer", "done", {
      signer: key0.address,
      txHash: registerTxHash,
    }));

    // Step 5: Mint demo tokens
    await stream.writeSSE(sseStep("mint", "pending"));
    const INITIAL_TOKENS = 1_000_000n * 10n ** 18n;
    const mintTxHash = await mintTokens(accountAddress, INITIAL_TOKENS);
    console.log(`[ephemeral-register] Minted tokens, tx: ${mintTxHash}`);
    await stream.writeSSE(sseStep("mint", "done", { txHash: mintTxHash }));

    // Store state
    const state: EphemeralState = {
      seed,
      keyIndex: 0,
      address: accountAddress.toLowerCase() as `0x${string}`,
    };
    ephemeralAccounts.set(accountAddress.toLowerCase(), state);

    // Complete
    await stream.writeSSE({
      event: "complete",
      data: JSON.stringify({
        address: accountAddress.toLowerCase(),
        currentSigner: key0.address,
        keyIndex: 0,
      }),
    });
  }, async (err, stream) => {
    const msg = err instanceof Error ? err.message : String(err);
    console.error(`[ephemeral-register] Error: ${msg}`);
    await stream.writeSSE({ event: "error", data: JSON.stringify({ message: msg }) });
  });
});

export default app;
