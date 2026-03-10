import { Hono } from "hono";
import { streamSSE } from "hono/streaming";
import type { Credential } from "../types.js";
import { deployAccount, fundAccount, mintTokens, getTokenBalance } from "../dev-account.js";

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

    // Store credential in-memory
    const credential: Credential = {
      credentialId: body.credentialId,
      publicKey: body.publicKey,
      address: address.toLowerCase(),
    };
    credentials.set(address.toLowerCase(), credential);
    console.log(`[register] Credential stored for ${address}`);

    // Complete
    await stream.writeSSE({ event: "complete", data: JSON.stringify({ address: address.toLowerCase() }) });
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
