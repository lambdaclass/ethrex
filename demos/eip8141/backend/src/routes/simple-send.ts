import { Hono } from "hono";
import { encodeAbiParameters, parseAbiParameters } from "viem";
import { sign } from "viem/accounts";
import { FrameTransaction, bytesToHex, hexToBytes } from "../frame-tx.js";
import { encodeVerifyAndPayCalldata } from "../webauthn.js";
import { sendRawTransaction, waitForReceipt, buildTxResponse } from "../rpc.js";
import { credentials } from "./register.js";
import { pendingSkeletons } from "./sig-hash.js";
import { ephemeralAccounts, deriveKey } from "../ephemeral-state.js";
import type { SimpleSendRequest } from "../types.js";

const app = new Hono();

// verifyEcdsaAndPay(uint8,bytes32,bytes32) selector = 0x450beed2
const VERIFY_ECDSA_AND_PAY_SELECTOR = "0x450beed2";

function encodeEcdsaVerifyCalldata(v: number, r: `0x${string}`, s: `0x${string}`): Uint8Array {
  const encoded = encodeAbiParameters(
    parseAbiParameters("uint8 v, bytes32 r, bytes32 s"),
    [v, r, s]
  );
  const selectorBytes = hexToBytes(VERIFY_ECDSA_AND_PAY_SELECTOR);
  const paramsBytes = hexToBytes(encoded);
  const result = new Uint8Array(4 + paramsBytes.length);
  result.set(selectorBytes, 0);
  result.set(paramsBytes, 4);
  return result;
}

app.post("/simple-send", async (c) => {
  try {
    const body = (await c.req.json()) as SimpleSendRequest;
    const address = body.address.toLowerCase();
    const authMethod = body.authMethod ?? "passkey";

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

    if (authMethod === "ephemeral") {
      // ── Ephemeral ECDSA path ──
      const state = ephemeralAccounts.get(address);
      if (!state) {
        return c.json({ error: `No ephemeral state for ${address}` }, 400);
      }

      const currentKey = deriveKey(state.seed, state.keyIndex);
      const nextKey = deriveKey(state.seed, state.keyIndex + 1);

      console.log(`[simple-send] Ephemeral: current=${currentKey.address} (idx ${state.keyIndex}), next=${nextKey.address}`);

      // Compute sig_hash and sign with ephemeral key
      const sigHash = tx.computeSigHash();
      const sigHashHex = bytesToHex(sigHash);
      const signature = await sign({
        hash: sigHashHex,
        privateKey: currentKey.privateKey,
      });

      const verifyCalldata = encodeEcdsaVerifyCalldata(
        Number(signature.v),
        signature.r as `0x${string}`,
        signature.s as `0x${string}`
      );
      tx.setVerifyFrameData(0, verifyCalldata);

      // Encode and send
      const rawTx = tx.encodeCanonical();
      const txHashHex = bytesToHex(tx.txHash());
      console.log(`[simple-send] Sending ephemeral tx ${txHashHex}`);

      const submittedHash = await sendRawTransaction(rawTx);
      const receipt = await waitForReceipt(submittedHash);
      console.log(`[simple-send] Receipt: status=${receipt.status}, gasUsed=${receipt.gasUsed}`);

      // Increment key index
      state.keyIndex += 1;

      // Frame modes: [VERIFY, SENDER(rotate), SENDER(transfer)]
      const baseResponse = buildTxResponse(receipt, submittedHash, [1, 2, 2]);
      return c.json({
        ...baseResponse,
        oldSigner: currentKey.address,
        newSigner: nextKey.address,
        keyIndex: state.keyIndex,
      });
    } else {
      // ── Passkey WebAuthn path ──
      if (!body.signature || !body.webauthn) {
        return c.json({ error: "Missing signature/webauthn for passkey auth" }, 400);
      }

      const sigHash = tx.computeSigHash();
      console.log(`[simple-send] sigHash=${bytesToHex(sigHash)}`);

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

      const rawTx = tx.encodeCanonical();
      const txHashHex = bytesToHex(tx.txHash());
      console.log(`[simple-send] Sending tx ${txHashHex}`);

      const submittedHash = await sendRawTransaction(rawTx);
      const receipt = await waitForReceipt(submittedHash);
      console.log(`[simple-send] Receipt: status=${receipt.status}, gasUsed=${receipt.gasUsed}`);

      // Frame modes: [VERIFY, SENDER]
      return c.json(buildTxResponse(receipt, submittedHash, [1, 2]));
    }
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    console.error(`[simple-send] Error: ${msg}`);
    return c.json({ error: msg }, 500);
  }
});

export default app;
