import { Hono } from "hono";
import { bytesToHex, hexToBytes } from "../frame-tx.js";
import { encodeVerifyAndPayCalldata } from "../webauthn.js";
import { sendRawTransaction, waitForReceipt, buildTxResponse } from "../rpc.js";
import { credentials } from "./register.js";
import { pendingSkeletons } from "./sig-hash.js";
import type { BatchOpsRequest } from "../types.js";

const app = new Hono();

app.post("/batch-ops", async (c) => {
  try {
    const body = (await c.req.json()) as BatchOpsRequest;
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

    // Fill VERIFY frame with WebAuthn signature (verify+pay, sender pays)
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
    tx.setVerifyFrameData(0, verifyCalldata);

    // Encode and send
    const rawTx = tx.encodeCanonical();
    const txHashHex = bytesToHex(tx.txHash());
    console.log(
      `[batch-ops] Sending tx ${txHashHex} with ${body.operations.length} operations`
    );

    const submittedHash = await sendRawTransaction(rawTx);
    console.log(`[batch-ops] Submitted: ${submittedHash}`);

    const receipt = await waitForReceipt(submittedHash);
    console.log(
      `[batch-ops] Receipt: status=${receipt.status}, gasUsed=${receipt.gasUsed}`
    );

    // Frame modes: [VERIFY, ...SENDER per operation]
    const frameModes = [1, ...Array(body.operations.length).fill(2)];
    return c.json(buildTxResponse(receipt, submittedHash, frameModes));
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    console.error(`[batch-ops] Error: ${msg}`);
    return c.json({ error: msg }, 500);
  }
});

export default app;
