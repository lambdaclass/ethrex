/**
 * End-to-end test for EIP-8141 demo.
 * Programmatically creates a P256 key pair, registers it, and exercises
 * all 4 demo flows (simple-send, sponsored-send, batch-ops, deploy-execute).
 *
 * Run: npx tsx test-e2e.ts
 */
import { createHash, randomBytes } from "crypto";
import { p256 } from "@noble/curves/p256";

const API = "http://localhost:3000/api";
const RPC = "http://localhost:8545";
const ACCOUNT = "0x1000000000000000000000000000000000000003";

// ── P256 key pair ───────────────────────────────────────────────────
const privKey = p256.utils.randomPrivateKey();
const pubKey = p256.getPublicKey(privKey, false); // uncompressed: 04 || x || y
const pubKeyX = "0x" + Buffer.from(pubKey.slice(1, 33)).toString("hex");
const pubKeyY = "0x" + Buffer.from(pubKey.slice(33, 65)).toString("hex");

// ── Helpers ─────────────────────────────────────────────────────────
async function jsonPost(url: string, body: unknown) {
  const res = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const data = await res.json();
  if (!res.ok && !data.error) throw new Error(`HTTP ${res.status}`);
  return data;
}

async function rpc(method: string, params: unknown[] = []) {
  const res = await fetch(RPC, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", method, params, id: 1 }),
  });
  const data = await res.json();
  if (data.error) throw new Error(`RPC ${method}: ${data.error.message}`);
  return data.result;
}

function base64url(buf: Buffer): string {
  return buf.toString("base64url");
}

function sha256(data: Buffer): Buffer {
  return createHash("sha256").update(data).digest();
}

/**
 * Build valid WebAuthn assertion data and P256 signature for a given sigHash.
 */
function signWebAuthn(sigHashHex: string) {
  const sigHash = Buffer.from(sigHashHex.replace("0x", ""), "hex");

  // authenticatorData: 37 bytes minimum
  // bytes 0-31: rpIdHash (sha256 of rpId, doesn't matter for demo)
  // byte 32: flags (UP=0x01, UV=0x04 → 0x05)
  // bytes 33-36: signCount (0)
  const rpIdHash = sha256(Buffer.from("localhost"));
  const flags = Buffer.from([0x05]); // UP + UV
  const signCount = Buffer.alloc(4);
  const authenticatorData = Buffer.concat([rpIdHash, flags, signCount]);

  // clientDataJSON with the challenge
  const challengeB64url = base64url(sigHash);
  const clientDataJSON = JSON.stringify({
    type: "webauthn.get",
    challenge: challengeB64url,
    origin: "http://localhost:5173",
    crossOrigin: false,
  });

  // Find indices for on-chain verification
  const typeIndex = clientDataJSON.indexOf('"type":"webauthn.get"');
  const challengeIndex = clientDataJSON.indexOf(`"challenge":"${challengeB64url}"`);

  // Compute the message to sign: sha256(authenticatorData || sha256(clientDataJSON))
  const clientDataHash = sha256(Buffer.from(clientDataJSON));
  const message = sha256(Buffer.concat([authenticatorData, clientDataHash]));

  // Sign with P256 private key
  const sig = p256.sign(message, privKey, { lowS: true });

  return {
    signature: {
      r: "0x" + sig.r.toString(16).padStart(64, "0"),
      s: "0x" + sig.s.toString(16).padStart(64, "0"),
    },
    webauthn: {
      authenticatorData: "0x" + authenticatorData.toString("hex"),
      clientDataJSON,
      challengeIndex,
      typeIndex,
      userVerificationRequired: false,
    },
  };
}

// ── Test flows ──────────────────────────────────────────────────────
async function register() {
  console.log("\n=== REGISTER ===");
  const res = await jsonPost(`${API}/register`, {
    credentialId: "test-" + randomBytes(8).toString("hex"),
    publicKey: { x: pubKeyX, y: pubKeyY },
  });
  console.log("Register:", res.success ? "OK" : res.error);
  if (!res.success) throw new Error("Registration failed: " + res.error);
  return res.address;
}

async function testSimpleSend() {
  console.log("\n=== SIMPLE SEND ===");
  // Step 1: get sig-hash
  const sigRes = await jsonPost(`${API}/sig-hash`, {
    demoType: "simple-send",
    params: {
      from: ACCOUNT,
      to: "0x000000000000000000000000000000000000dEaD",
      amount: "0.001",
    },
  });
  if (sigRes.error) throw new Error("sig-hash: " + sigRes.error);
  console.log("sigHash:", sigRes.sigHash);

  // Step 2: sign with P256
  const auth = signWebAuthn(sigRes.sigHash);

  // Step 3: submit
  const txRes = await jsonPost(`${API}/simple-send`, {
    address: ACCOUNT,
    to: "0x000000000000000000000000000000000000dEaD",
    amount: "0.001",
    ...auth,
  });

  console.log("Result:", txRes.success ? "SUCCESS" : "FAILED");
  console.log("TxHash:", txRes.txHash);
  if (txRes.frameReceipts) {
    for (const fr of txRes.frameReceipts) {
      console.log(`  Frame [${fr.mode}]: ${fr.status ? "OK" : "REVERTED"} (${fr.gasUsed} gas)`);
    }
  }
  if (txRes.error) console.log("Error:", txRes.error);
  return txRes;
}

async function testSponsoredSend() {
  console.log("\n=== SPONSORED ERC20 SEND ===");
  const sigRes = await jsonPost(`${API}/sig-hash`, {
    demoType: "sponsored-send",
    params: {
      from: ACCOUNT,
      to: "0x000000000000000000000000000000000000dEaD",
      amount: "100",
      sponsorAddress: "0x1000000000000000000000000000000000000001",
    },
  });
  if (sigRes.error) throw new Error("sig-hash: " + sigRes.error);
  console.log("sigHash:", sigRes.sigHash);

  const auth = signWebAuthn(sigRes.sigHash);

  const txRes = await jsonPost(`${API}/sponsored-send`, {
    address: ACCOUNT,
    to: "0x000000000000000000000000000000000000dEaD",
    amount: "100",
    sponsorAddress: "0x1000000000000000000000000000000000000001",
    ...auth,
  });

  console.log("Result:", txRes.success ? "SUCCESS" : "FAILED");
  console.log("TxHash:", txRes.txHash);
  if (txRes.frameReceipts) {
    for (const fr of txRes.frameReceipts) {
      console.log(`  Frame [${fr.mode}]: ${fr.status ? "OK" : "REVERTED"} (${fr.gasUsed} gas)`);
    }
  }
  if (txRes.error) console.log("Error:", txRes.error);
  return txRes;
}

async function testBatchOps() {
  console.log("\n=== BATCH OPS ===");
  const sigRes = await jsonPost(`${API}/sig-hash`, {
    demoType: "batch-ops",
    params: {
      from: ACCOUNT,
      operations: [
        { to: "0x000000000000000000000000000000000000dEaD", value: "0.001", data: "0x" },
        { to: "0x0000000000000000000000000000000000000001", value: "0.001", data: "0x" },
      ],
    },
  });
  if (sigRes.error) throw new Error("sig-hash: " + sigRes.error);
  console.log("sigHash:", sigRes.sigHash);

  const auth = signWebAuthn(sigRes.sigHash);

  const txRes = await jsonPost(`${API}/batch-ops`, {
    address: ACCOUNT,
    operations: [
      { to: "0x000000000000000000000000000000000000dEaD", value: "0.001", data: "0x" },
      { to: "0x0000000000000000000000000000000000000001", value: "0.001", data: "0x" },
    ],
    ...auth,
  });

  console.log("Result:", txRes.success ? "SUCCESS" : "FAILED");
  console.log("TxHash:", txRes.txHash);
  if (txRes.frameReceipts) {
    for (const fr of txRes.frameReceipts) {
      console.log(`  Frame [${fr.mode}]: ${fr.status ? "OK" : "REVERTED"} (${fr.gasUsed} gas)`);
    }
  }
  if (txRes.error) console.log("Error:", txRes.error);
  return txRes;
}

async function testDeployExecute() {
  console.log("\n=== DEPLOY + EXECUTE ===");
  // Simple counter contract:
  // constructor: stores 0
  // increment(): SSTORE slot0 = SLOAD slot0 + 1
  // Minimal bytecode: PUSH1 0 SSTORE ... (we'll use a trivial contract)
  //
  // Runtime: PUSH1 1 PUSH1 0 SLOAD ADD PUSH1 0 SSTORE STOP
  // = 6001 6000 54 01 6000 55 00
  // Init code: PUSH7 <runtime> PUSH1 0 MSTORE PUSH1 7 PUSH1 25 RETURN
  // Actually let's use the simplest possible: a contract that just returns 42
  // Runtime: PUSH1 0x2a PUSH1 0 MSTORE PUSH1 32 PUSH1 0 RETURN
  // = 602a 6000 52 6020 6000 f3
  // Init: PUSH6 <runtime> PUSH1 0 MSTORE PUSH1 6 PUSH1 26 RETURN
  // Actually simpler — just use raw hex:
  const runtimeCode = "602a60005260206000f3"; // returns 42
  const runtimeLen = runtimeCode.length / 2; // 10 bytes
  // PUSH<len> <runtime> PUSH1 0 MSTORE PUSH1 <len> PUSH1 <32-len> RETURN
  const offset = 32 - runtimeLen; // 22
  const initCode = `69${runtimeCode}600052600a60${offset.toString(16).padStart(2, "0")}f3`;

  // For execute frame: just call the deployed contract with empty calldata
  // We don't know the address yet, so use a dummy execute calldata
  // Actually, the deploy-execute route needs calldata for the SENDER frame
  // Let's just call with empty data — the contract returns 42 on any call
  const executeCalldata = "0x";

  const sigRes = await jsonPost(`${API}/sig-hash`, {
    demoType: "deploy-execute",
    params: {
      from: ACCOUNT,
      bytecode: "0x" + initCode,
      calldata: executeCalldata,
    },
  });
  if (sigRes.error) throw new Error("sig-hash: " + sigRes.error);
  console.log("sigHash:", sigRes.sigHash);

  const auth = signWebAuthn(sigRes.sigHash);

  const txRes = await jsonPost(`${API}/deploy-execute`, {
    address: ACCOUNT,
    bytecode: "0x" + initCode,
    calldata: executeCalldata,
    ...auth,
  });

  console.log("Result:", txRes.success ? "SUCCESS" : "FAILED");
  console.log("TxHash:", txRes.txHash);
  if (txRes.frameReceipts) {
    for (const fr of txRes.frameReceipts) {
      console.log(`  Frame [${fr.mode}]: ${fr.status ? "OK" : "REVERTED"} (${fr.gasUsed} gas)`);
    }
  }
  if (txRes.error) console.log("Error:", txRes.error);
  return txRes;
}

// ── Main ────────────────────────────────────────────────────────────
async function main() {
  console.log("EIP-8141 Demo End-to-End Test");
  console.log("=============================");
  console.log(`Account: ${ACCOUNT}`);
  console.log(`PubKey X: ${pubKeyX.slice(0, 18)}...`);
  console.log(`PubKey Y: ${pubKeyY.slice(0, 18)}...`);

  // Check services
  const block = await rpc("eth_blockNumber");
  console.log(`\nBlock number: ${parseInt(block, 16)}`);

  const results: Record<string, any> = {};

  // Register
  await register();

  // Test all 4 demos
  results["simple-send"] = await testSimpleSend();
  results["sponsored-send"] = await testSponsoredSend();
  results["batch-ops"] = await testBatchOps();
  results["deploy-execute"] = await testDeployExecute();

  // Summary
  console.log("\n\n=== SUMMARY ===");
  const txHashes: string[] = [];
  for (const [name, res] of Object.entries(results)) {
    const status = res.success ? "PASS" : "FAIL";
    const frames = res.frameReceipts?.length ?? 0;
    console.log(`  ${status}  ${name} (${frames} frames) tx=${res.txHash ?? "none"}`);
    if (res.txHash) txHashes.push(res.txHash);
  }

  const allPassed = Object.values(results).every((r: any) => r.success);
  console.log(`\nOverall: ${allPassed ? "ALL PASSED" : "SOME FAILED"}`);

  if (txHashes.length > 0) {
    console.log("\nTransaction hashes for Blockscout verification:");
    txHashes.forEach((h) => console.log(`  ${h}`));
  }
}

main().catch((err) => {
  console.error("\nFATAL:", err.message);
  process.exit(1);
});
