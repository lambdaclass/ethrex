import { Hono } from "hono";
import { bytesToHex, hexToBytes } from "../frame-tx.js";
import { encodeVerifyAndPayCalldata } from "../webauthn.js";
import { sendRawTransaction, waitForReceipt, buildTxResponse } from "../rpc.js";
import { credentials } from "./register.js";
import { pendingSkeletons } from "./sig-hash.js";
import type { DeployExecuteRequest } from "../types.js";

const app = new Hono();

app.post("/deploy-execute", async (c) => {
  try {
    const body = (await c.req.json()) as DeployExecuteRequest;
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
    console.log(`[deploy-execute] Sending tx ${txHashHex}`);

    const submittedHash = await sendRawTransaction(rawTx);
    console.log(`[deploy-execute] Submitted: ${submittedHash}`);

    const receipt = await waitForReceipt(submittedHash);
    console.log(
      `[deploy-execute] Receipt: status=${receipt.status}, gasUsed=${receipt.gasUsed}`
    );

    // Frame modes: [VERIFY, DEFAULT (deploy), SENDER (execute)]
    return c.json(buildTxResponse(receipt, submittedHash, [1, 0, 2]));
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    console.error(`[deploy-execute] Error: ${msg}`);
    return c.json({ error: msg }, 500);
  }
});

export default app;
