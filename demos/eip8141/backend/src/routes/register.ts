import { Hono } from "hono";
import type { Credential } from "../types.js";
import { ACCOUNT_ADDRESS } from "../types.js";
import { setPublicKey, mintTokens, getTokenBalance } from "../dev-account.js";

// In-memory credential store keyed by account address
export const credentials = new Map<string, Credential>();

const app = new Hono();

app.post("/register", async (c) => {
  try {
    const body = (await c.req.json()) as {
      credentialId: string;
      publicKey: { x: string; y: string };
    };

    if (!body.credentialId || !body.publicKey?.x || !body.publicKey?.y) {
      return c.json({ error: "Missing required fields" }, 400);
    }

    // The demo uses a single pre-deployed account contract
    const address = ACCOUNT_ADDRESS.toLowerCase();

    // Set the public key on-chain via a regular TX from the dev account
    const pubKeyX = BigInt(body.publicKey.x);
    const pubKeyY = BigInt(body.publicKey.y);

    console.log(`[register] Setting public key on ${address}...`);
    console.log(`[register]   x = ${body.publicKey.x}`);
    console.log(`[register]   y = ${body.publicKey.y}`);

    const txHash = await setPublicKey(pubKeyX, pubKeyY);
    console.log(`[register] Public key set on-chain, tx: ${txHash}`);

    // Mint demo ERC20 tokens to the account (1,000,000 tokens)
    const currentBalance = await getTokenBalance(address);
    if (currentBalance === 0n) {
      const INITIAL_TOKENS = 1_000_000n * 10n ** 18n;
      const mintTx = await mintTokens(address, INITIAL_TOKENS);
      console.log(`[register] Minted 1,000,000 demo tokens, tx: ${mintTx}`);
    } else {
      console.log(`[register] Account already has tokens: ${currentBalance}`);
    }

    // Store credential in-memory
    const credential: Credential = {
      credentialId: body.credentialId,
      publicKey: body.publicKey,
      address,
    };

    credentials.set(address, credential);
    console.log(`[register] Credential stored for ${address}`);

    return c.json({ success: true, address });
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    console.error(`[register] Error: ${msg}`);
    return c.json({ error: msg }, 500);
  }
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
