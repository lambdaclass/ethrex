import { Hono } from "hono";
import { streamSSE } from "hono/streaming";
import {
  createWalletClient,
  createPublicClient,
  http,
  encodeFunctionData,
  defineChain,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";
import type { Credential } from "../types.js";
import { deployAccount, fundAccount, mintTokens, getTokenBalance, DEV_PRIVATE_KEY } from "../dev-account.js";
import { deriveKey, ephemeralAccounts, type EphemeralState } from "../ephemeral-state.js";

const RPC_URL = process.env.RPC_URL ?? "http://localhost:8545";
const devAccount = privateKeyToAccount(DEV_PRIVATE_KEY);
const demoChain = defineChain({
  id: 1729,
  name: "ethrex-demo",
  nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
  rpcUrls: { default: { http: [RPC_URL] } },
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

// In-memory credential store keyed by account address
export const credentials = new Map<string, Credential>();

const app = new Hono();

function sseStep(step: string, status: string, extra?: Record<string, string>) {
  return { event: "step", data: JSON.stringify({ step, status, ...extra }) };
}

app.post("/register", async (c) => {
  const body = await c.req.json().catch(() => null) as {
    credentialId: string;
    publicKey: { x: string; y: string };
  } | null;

  if (!body?.credentialId || !body?.publicKey?.x || !body?.publicKey?.y) {
    return c.json({ error: "Missing required fields" }, 400);
  }

  const pubKeyX = BigInt(body.publicKey.x);
  const pubKeyY = BigInt(body.publicKey.y);

  return streamSSE(c, async (stream) => {
    // Step 1: Deploy account
    await stream.writeSSE(sseStep("deploy", "pending"));
    console.log(`[register] Deploying new account for pubkey...`);
    console.log(`[register]   x = ${body.publicKey.x}`);
    console.log(`[register]   y = ${body.publicKey.y}`);

    const { address, txHash: deployTxHash } = await deployAccount(pubKeyX, pubKeyY);
    console.log(`[register] Account deployed at ${address}`);
    await stream.writeSSE(sseStep("deploy", "done", { address, txHash: deployTxHash }));

    // Step 2: Fund with ETH
    await stream.writeSSE(sseStep("fund", "pending"));
    const fundTxHash = await fundAccount(address);
    console.log(`[register] Funded with 10 ETH, tx: ${fundTxHash}`);
    await stream.writeSSE(sseStep("fund", "done", { txHash: fundTxHash }));

    // Step 3: Mint demo tokens
    const currentBalance = await getTokenBalance(address);
    if (currentBalance === 0n) {
      await stream.writeSSE(sseStep("mint", "pending"));
      const INITIAL_TOKENS = 1_000_000n * 10n ** 18n;
      const mintTxHash = await mintTokens(address, INITIAL_TOKENS);
      console.log(`[register] Minted 1,000,000 demo tokens, tx: ${mintTxHash}`);
      await stream.writeSSE(sseStep("mint", "done", { txHash: mintTxHash }));
    } else {
      console.log(`[register] Account already has tokens: ${currentBalance}`);
      await stream.writeSSE(sseStep("mint", "done", { skipped: "true" }));
    }

    // Step 4: Generate ephemeral seed and register initial signer
    await stream.writeSSE(sseStep("register-signer", "pending"));

    const seedBytes = new Uint8Array(32);
    crypto.getRandomValues(seedBytes);
    const seed = ("0x" + Array.from(seedBytes).map(b => b.toString(16).padStart(2, "0")).join("")) as `0x${string}`;
    const key0 = deriveKey(seed, 0);
    console.log(`[register] Derived ephemeral key0: ${key0.address}`);

    // Call account.execute(address(this), 0, rotate(key0.address)) to register signer
    const rotateCalldata = encodeFunctionData({
      abi: [{
        type: "function",
        name: "rotate",
        inputs: [{ name: "newSigner", type: "address" }],
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
      args: [address as `0x${string}`, 0n, rotateCalldata],
    });

    const registerTxHash = await walletClient.sendTransaction({
      to: address as `0x${string}`,
      data: executeCalldata,
    });
    await publicClient.waitForTransactionReceipt({ hash: registerTxHash });
    console.log(`[register] Ephemeral signer registered: ${key0.address}, tx: ${registerTxHash}`);
    await stream.writeSSE(sseStep("register-signer", "done", {
      signer: key0.address,
      txHash: registerTxHash,
    }));

    // Store ephemeral state
    const ephState: EphemeralState = {
      seed,
      keyIndex: 0,
      address: address.toLowerCase() as `0x${string}`,
    };
    ephemeralAccounts.set(address.toLowerCase(), ephState);

    // Store credential in-memory
    const credential: Credential = {
      credentialId: body.credentialId,
      publicKey: body.publicKey,
      address: address.toLowerCase(),
    };
    credentials.set(address.toLowerCase(), credential);
    console.log(`[register] Credential stored for ${address}`);

    // Complete
    await stream.writeSSE({ event: "complete", data: JSON.stringify({
      address: address.toLowerCase(),
      currentSigner: key0.address,
    }) });
  }, async (err, stream) => {
    const msg = err instanceof Error ? err.message : String(err);
    console.error(`[register] Error: ${msg}`);
    await stream.writeSSE({ event: "error", data: JSON.stringify({ message: msg }) });
  });
});

app.get("/token-balance/:address", async (c) => {
  try {
    const address = c.req.param("address").toLowerCase();
    const balance = await getTokenBalance(address);
    return c.json({
      address,
      balance: balance.toString(),
      formatted: (Number(balance) / 1e18).toFixed(2),
    });
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    return c.json({ error: msg }, 500);
  }
});

export default app;
