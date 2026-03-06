import { Hono } from "hono";
import { bytesToHex, hexToBytes } from "../frame-tx.js";
import { encodeVerifyCalldata } from "../webauthn.js";
import { sendRawTransaction, waitForReceipt, buildTxResponse } from "../rpc.js";
import { credentials } from "./register.js";
import { pendingSkeletons } from "./sig-hash.js";
import { SPONSOR_ADDRESS } from "../types.js";
import type { SponsoredSendRequest } from "../types.js";

const app = new Hono();

app.post("/sponsored-send", async (c) => {
  try {
    const body = (await c.req.json()) as SponsoredSendRequest;
    const address = body.address.toLowerCase();
    const sponsorAddress =
      body.sponsorAddress?.toLowerCase() ?? SPONSOR_ADDRESS;

    const credential = credentials.get(address);
    if (!credential) {
      return c.json({ error: `No credential registered for ${address}` }, 400);
    }

    // Retrieve the cached skeleton (built during sig-hash)
    const tx = pendingSkeletons.get(address);
    if (!tx) {
      return c.json({ error: "No pending skeleton — call /sig-hash first" }, 400);
    }
    pendingSkeletons.delete(address);

    // Frame 0: VERIFY for sender (scope=0, just identity proof)
    const verifyCalldata = encodeVerifyCalldata(
      { r: BigInt(body.signature.r), s: BigInt(body.signature.s) },
      {
        authenticatorData: hexToBytes(body.webauthn.authenticatorData),
        clientDataJSON: body.webauthn.clientDataJSON,
        challengeIndex: body.webauthn.challengeIndex,
        typeIndex: body.webauthn.typeIndex,
        userVerificationRequired: body.webauthn.userVerificationRequired,
      }
    );
    tx.setVerifyFrameData(0, verifyCalldata);

    // Frame 1: VERIFY for sponsor — GasSponsor.verify() has no args
    // selector: 0xfc735e99
    tx.setVerifyFrameData(1, hexToBytes("0xfc735e99"));

    // Encode and send
    const rawTx = tx.encodeCanonical();
    const txHashHex = bytesToHex(tx.txHash());
    console.log(`[sponsored-send] Sending tx ${txHashHex}`);

    const submittedHash = await sendRawTransaction(rawTx);
    console.log(`[sponsored-send] Submitted: ${submittedHash}`);

    const receipt = await waitForReceipt(submittedHash);
    console.log(
      `[sponsored-send] Receipt: status=${receipt.status}, gasUsed=${receipt.gasUsed}`
    );

    // Frame modes: [VERIFY sender, VERIFY payer, SENDER]
    return c.json(buildTxResponse(receipt, submittedHash, [1, 1, 2]));
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    console.error(`[sponsored-send] Error: ${msg}`);
    return c.json({ error: msg }, 500);
  }
});

export default app;
