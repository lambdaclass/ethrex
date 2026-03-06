import { Hono } from "hono";
import { FrameTransaction, bytesToHex, hexToBytes } from "../frame-tx.js";
import { encodeVerifyAndPayCalldata } from "../webauthn.js";
import { sendRawTransaction, waitForReceipt, buildTxResponse } from "../rpc.js";
import { credentials } from "./register.js";
import { pendingSkeletons } from "./sig-hash.js";
import type { SimpleSendRequest } from "../types.js";

const app = new Hono();

app.post("/simple-send", async (c) => {
  try {
    const body = (await c.req.json()) as SimpleSendRequest;
    const address = body.address.toLowerCase();

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

    // Debug: log sigHash and WebAuthn data for diagnosis
    const sigHash = tx.computeSigHash();
    console.log(`[simple-send] sigHash=${bytesToHex(sigHash)}`);
    console.log(`[simple-send] sig r=${body.signature.r}`);
    console.log(`[simple-send] sig s=${body.signature.s}`);
    console.log(`[simple-send] authenticatorData=${body.webauthn.authenticatorData}`);
    console.log(`[simple-send] clientDataJSON=${body.webauthn.clientDataJSON}`);
    console.log(`[simple-send] challengeIndex=${body.webauthn.challengeIndex}, typeIndex=${body.webauthn.typeIndex}`);

    // Encode the WebAuthn verify+pay calldata (sender pays their own gas)
    const verifyCalldata = encodeVerifyAndPayCalldata(
      { r: BigInt(body.signature.r), s: BigInt(body.signature.s) },
      {
        authenticatorData: hexToBytes(body.webauthn.authenticatorData),
        clientDataJSON: body.webauthn.clientDataJSON,
        challengeIndex: body.webauthn.challengeIndex,
        typeIndex: body.webauthn.typeIndex,
        userVerificationRequired: body.webauthn.userVerificationRequired,
      }
    );

    // Fill in VERIFY frame data
    tx.setVerifyFrameData(0, verifyCalldata);

    // Encode and send
    const rawTx = tx.encodeCanonical();
    const txHashBytes = tx.txHash();
    const txHashHex = bytesToHex(txHashBytes);
    console.log(`[simple-send] Sending tx ${txHashHex}`);

    const submittedHash = await sendRawTransaction(rawTx);
    console.log(`[simple-send] Submitted: ${submittedHash}`);

    const receipt = await waitForReceipt(submittedHash);
    console.log(
      `[simple-send] Receipt: status=${receipt.status}, gasUsed=${receipt.gasUsed}`
    );

    // Frame modes: [VERIFY, SENDER]
    return c.json(buildTxResponse(receipt, submittedHash, [1, 2]));
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    console.error(`[simple-send] Error: ${msg}`);
    return c.json({ error: msg }, 500);
  }
});

export default app;
