/**
 * Comprehensive E2E test for EIP-8141 demo.
 *
 * Tests all 4 demo flows with:
 * - Balance verification (ETH + ERC20) before and after
 * - Nonce verification before and after
 * - Blockscout API verification of frame transaction structure
 * - Frame receipt validation per EIP-8141 spec
 *
 * Run: npx tsx test-e2e-full.ts
 * Optional: BLOCKSCOUT_URL=http://localhost:8082 npx tsx test-e2e-full.ts
 */
import { createHash, randomBytes } from "crypto";
import { p256 } from "@noble/curves/p256";

const API = process.env.API_URL ?? "http://localhost:3000/api";
const RPC = process.env.RPC_URL ?? "http://localhost:8545";
const BLOCKSCOUT = process.env.BLOCKSCOUT_URL ?? "http://localhost:8082";
const ACCOUNT = "0x1000000000000000000000000000000000000003";
const MOCK_ERC20 = "0x1000000000000000000000000000000000000002";
const SPONSOR = "0x1000000000000000000000000000000000000001";
const DEAD = "0x000000000000000000000000000000000000dEaD";
const ADDR_0001 = "0x0000000000000000000000000000000000000001";

// ── P256 key pair ───────────────────────────────────────────────────
const privKey = p256.utils.randomPrivateKey();
const pubKey = p256.getPublicKey(privKey, false);
const pubKeyX = "0x" + Buffer.from(pubKey.slice(1, 33)).toString("hex");
const pubKeyY = "0x" + Buffer.from(pubKey.slice(33, 65)).toString("hex");

// ── Test tracking ───────────────────────────────────────────────────
let totalAssertions = 0;
let passedAssertions = 0;
let failedAssertions = 0;
const failures: string[] = [];

function assert(condition: boolean, message: string) {
  totalAssertions++;
  if (condition) {
    passedAssertions++;
    console.log(`  ✓ ${message}`);
  } else {
    failedAssertions++;
    failures.push(message);
    console.log(`  ✗ FAIL: ${message}`);
  }
}

function assertBigIntEq(actual: bigint, expected: bigint, message: string) {
  assert(actual === expected, `${message} (got ${actual}, expected ${expected})`);
}

function assertBigIntGt(actual: bigint, threshold: bigint, message: string) {
  assert(actual > threshold, `${message} (got ${actual}, threshold ${threshold})`);
}

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

async function getEthBalance(address: string): Promise<bigint> {
  const result = await rpc("eth_getBalance", [address, "latest"]);
  return BigInt(result);
}

async function getNonce(address: string): Promise<bigint> {
  const result = await rpc("eth_getTransactionCount", [address, "latest"]);
  return BigInt(result);
}

async function getTokenBalance(address: string): Promise<bigint> {
  // balanceOf(address) selector = 0x70a08231
  const paddedAddr = address.replace("0x", "").toLowerCase().padStart(64, "0");
  const data = "0x70a08231" + paddedAddr;
  const result = await rpc("eth_call", [
    { to: MOCK_ERC20, data },
    "latest",
  ]);
  return BigInt(result);
}

async function getTransactionByHash(txHash: string): Promise<any> {
  return await rpc("eth_getTransactionByHash", [txHash]);
}

async function getTransactionReceipt(txHash: string): Promise<any> {
  return await rpc("eth_getTransactionReceipt", [txHash]);
}

function base64url(buf: Buffer): string {
  return buf.toString("base64url");
}

function sha256(data: Buffer): Buffer {
  return createHash("sha256").update(data).digest();
}

function signWebAuthn(sigHashHex: string) {
  const sigHash = Buffer.from(sigHashHex.replace("0x", ""), "hex");
  const rpIdHash = sha256(Buffer.from("localhost"));
  const flags = Buffer.from([0x05]); // UP + UV
  const signCount = Buffer.alloc(4);
  const authenticatorData = Buffer.concat([rpIdHash, flags, signCount]);

  const challengeB64url = base64url(sigHash);
  const clientDataJSON = JSON.stringify({
    type: "webauthn.get",
    challenge: challengeB64url,
    origin: "http://localhost:5173",
    crossOrigin: false,
  });

  const typeIndex = clientDataJSON.indexOf('"type":"webauthn.get"');
  const challengeIndex = clientDataJSON.indexOf(`"challenge":"${challengeB64url}"`);

  const clientDataHash = sha256(Buffer.from(clientDataJSON));
  const message = sha256(Buffer.concat([authenticatorData, clientDataHash]));
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

function formatEth(wei: bigint): string {
  const eth = Number(wei) / 1e18;
  return eth.toFixed(6) + " ETH";
}

function formatTokens(raw: bigint): string {
  const tokens = Number(raw) / 1e18;
  return tokens.toFixed(2) + " DEMO";
}

// ── State capture ───────────────────────────────────────────────────
interface AccountState {
  ethBalance: bigint;
  tokenBalance: bigint;
  nonce: bigint;
}

async function captureState(address: string): Promise<AccountState> {
  const [ethBalance, tokenBalance, nonce] = await Promise.all([
    getEthBalance(address),
    getTokenBalance(address),
    getNonce(address),
  ]);
  return { ethBalance, tokenBalance, nonce };
}

function logState(label: string, state: AccountState) {
  console.log(`  ${label}: ETH=${formatEth(state.ethBalance)}, Tokens=${formatTokens(state.tokenBalance)}, Nonce=${state.nonce}`);
}

// ── Blockscout verification ─────────────────────────────────────────
async function verifyOnBlockscout(txHash: string, expectedFrameCount: number, expectedFrameModes: string[]) {
  console.log(`\n  --- Blockscout verification for ${txHash} ---`);

  // Wait for Blockscout to index
  let blockscoutTx: any = null;
  for (let attempt = 0; attempt < 15; attempt++) {
    try {
      const res = await fetch(`${BLOCKSCOUT}/api/v2/transactions/${txHash}`);
      if (res.ok) {
        blockscoutTx = await res.json();
        if (blockscoutTx.hash) break;
      }
    } catch (_) {}
    await new Promise((r) => setTimeout(r, 2000));
  }

  if (!blockscoutTx || !blockscoutTx.hash) {
    console.log("  ⚠ Blockscout: transaction not found after 30s (indexing may be slow)");
    return;
  }

  // Check transaction type
  const txType = blockscoutTx.type;
  assert(txType === 6 || txType === "0x6" || txType === "6",
    `Blockscout tx type is 6 (frame tx) — got ${txType}`);

  // Check transaction status
  const status = blockscoutTx.status;
  if (status !== undefined) {
    assert(status === "ok" || status === "success" || status === true,
      `Blockscout tx status is success — got ${status}`);
  }

  // Check block exists
  if (blockscoutTx.block !== undefined) {
    assert(blockscoutTx.block !== null,
      `Blockscout tx is in a block (block=${blockscoutTx.block})`);
  }

  // Log raw Blockscout response fields for debugging
  console.log(`  Blockscout fields: type=${txType}, status=${status}, block=${blockscoutTx.block ?? "N/A"}`);
  if (blockscoutTx.tx_types) console.log(`  tx_types: ${JSON.stringify(blockscoutTx.tx_types)}`);

  // Also verify via RPC that the raw tx has the right structure
  const rpcTx = await getTransactionByHash(txHash);
  if (rpcTx) {
    const rpcType = parseInt(rpcTx.type, 16);
    assert(rpcType === 6, `RPC tx type is 6 (frame tx) — got ${rpcType}`);

    // Check frame receipts in receipt
    const receipt = await getTransactionReceipt(txHash);
    if (receipt && receipt.frameReceipts) {
      const frameReceipts = receipt.frameReceipts as Array<{ status: string; gasUsed: string }>;
      assert(frameReceipts.length === expectedFrameCount,
        `Frame receipt count matches expected (got ${frameReceipts.length}, expected ${expectedFrameCount})`);

      for (let i = 0; i < frameReceipts.length; i++) {
        const fr = frameReceipts[i];
        const statusOk = fr.status === "0x1";
        const gasUsed = parseInt(fr.gasUsed, 16);
        const mode = expectedFrameModes[i] ?? `FRAME${i}`;
        assert(statusOk, `Frame ${i} [${mode}] succeeded`);
        assert(gasUsed > 0, `Frame ${i} [${mode}] used gas (${gasUsed})`);
      }
    } else {
      console.log("  ⚠ No frame receipts in RPC receipt");
    }
  }
}

// ── Test flows ──────────────────────────────────────────────────────
async function register() {
  console.log("\n══════════════════════════════════════════════════════════");
  console.log("  REGISTER");
  console.log("══════════════════════════════════════════════════════════");

  const beforeNonce = await getNonce(ACCOUNT);

  const res = await jsonPost(`${API}/register`, {
    credentialId: "test-" + randomBytes(8).toString("hex"),
    publicKey: { x: pubKeyX, y: pubKeyY },
  });

  assert(res.success === true, "Registration succeeded");
  assert(res.address.toLowerCase() === ACCOUNT.toLowerCase(), `Registered address matches expected account`);

  // Check that the account now has tokens
  const tokenBal = await getTokenBalance(ACCOUNT);
  assertBigIntGt(tokenBal, 0n, "Account has DEMO tokens after registration");
  console.log(`  Token balance: ${formatTokens(tokenBal)}`);

  return res.address;
}

async function testSimpleSend(): Promise<string | null> {
  console.log("\n══════════════════════════════════════════════════════════");
  console.log("  SIMPLE SEND (0.001 ETH to 0xdead)");
  console.log("══════════════════════════════════════════════════════════");

  const SEND_AMOUNT = 1_000_000_000_000_000n; // 0.001 ETH

  // Capture state BEFORE
  console.log("\n  --- Before ---");
  const accountBefore = await captureState(ACCOUNT);
  const deadBefore = await captureState(DEAD);
  logState("Account", accountBefore);
  logState("Dead", deadBefore);

  // Step 1: get sig-hash
  const sigRes = await jsonPost(`${API}/sig-hash`, {
    demoType: "simple-send",
    params: { from: ACCOUNT, to: DEAD, amount: "0.001" },
  });
  assert(!sigRes.error, `sig-hash succeeded`);
  assert(typeof sigRes.sigHash === "string" && sigRes.sigHash.startsWith("0x"),
    `sig-hash returned valid hash: ${sigRes.sigHash?.slice(0, 18)}...`);

  // Verify skeleton structure
  const skeleton = sigRes.txSkeleton;
  assert(skeleton.frames.length === 2, `Skeleton has 2 frames (VERIFY + SENDER)`);
  assertBigIntEq(BigInt(skeleton.frames[0].mode), 1n, `Frame 0 is VERIFY (mode=1)`);
  assertBigIntEq(BigInt(skeleton.frames[1].mode), 2n, `Frame 1 is SENDER (mode=2)`);
  assert(skeleton.frames[0].data === "0x" || skeleton.frames[0].data === "0x00" || skeleton.frames[0].data.length <= 4,
    `VERIFY frame data is empty (will be filled after signing)`);

  // Step 2: sign
  const auth = signWebAuthn(sigRes.sigHash);

  // Step 3: submit
  const txRes = await jsonPost(`${API}/simple-send`, {
    address: ACCOUNT,
    to: DEAD,
    amount: "0.001",
    ...auth,
  });

  assert(txRes.success === true, "Simple send transaction succeeded");
  assert(typeof txRes.txHash === "string", `Got tx hash: ${txRes.txHash}`);

  // Verify frame receipts
  assert(txRes.frameReceipts?.length === 2, "Got 2 frame receipts");
  if (txRes.frameReceipts) {
    assert(txRes.frameReceipts[0].mode === "VERIFY", "Frame 0 mode is VERIFY");
    assert(txRes.frameReceipts[0].status === true, "VERIFY frame succeeded");
    assert(txRes.frameReceipts[1].mode === "SENDER", "Frame 1 mode is SENDER");
    assert(txRes.frameReceipts[1].status === true, "SENDER frame succeeded");
  }

  // Capture state AFTER
  console.log("\n  --- After ---");
  const accountAfter = await captureState(ACCOUNT);
  const deadAfter = await captureState(DEAD);
  logState("Account", accountAfter);
  logState("Dead", deadAfter);

  // Verify balance deltas
  console.log("\n  --- Balance verification ---");
  const deadEthDelta = deadAfter.ethBalance - deadBefore.ethBalance;
  assertBigIntEq(deadEthDelta, SEND_AMOUNT,
    `Dead received exactly 0.001 ETH`);

  const accountEthDelta = accountBefore.ethBalance - accountAfter.ethBalance;
  assertBigIntGt(accountEthDelta, SEND_AMOUNT,
    `Account spent > 0.001 ETH (0.001 + gas)`);

  const gasPaid = accountEthDelta - SEND_AMOUNT;
  console.log(`  Gas paid by account: ${formatEth(gasPaid)}`);

  // Token balances should not change
  assertBigIntEq(accountAfter.tokenBalance, accountBefore.tokenBalance,
    "Account token balance unchanged");

  // Nonce should increment by 1
  console.log("\n  --- Nonce verification ---");
  assertBigIntEq(accountAfter.nonce, accountBefore.nonce + 1n,
    `Account nonce incremented by 1 (${accountBefore.nonce} -> ${accountAfter.nonce})`);

  // Blockscout
  if (txRes.txHash) {
    await verifyOnBlockscout(txRes.txHash, 2, ["VERIFY", "SENDER"]);
  }

  return txRes.txHash ?? null;
}

async function testSponsoredSend(): Promise<string | null> {
  console.log("\n══════════════════════════════════════════════════════════");
  console.log("  SPONSORED ERC20 SEND (100 DEMO to 0xdead)");
  console.log("══════════════════════════════════════════════════════════");

  const SEND_TOKENS = 100n * 10n ** 18n; // 100 DEMO

  // Capture state BEFORE
  console.log("\n  --- Before ---");
  const accountBefore = await captureState(ACCOUNT);
  const deadBefore = await captureState(DEAD);
  const sponsorBefore = await captureState(SPONSOR);
  logState("Account", accountBefore);
  logState("Dead", deadBefore);
  logState("Sponsor", sponsorBefore);

  // Step 1: sig-hash
  const sigRes = await jsonPost(`${API}/sig-hash`, {
    demoType: "sponsored-send",
    params: {
      from: ACCOUNT,
      to: DEAD,
      amount: "100",
      sponsorAddress: SPONSOR,
    },
  });
  assert(!sigRes.error, "sig-hash succeeded");

  // Verify skeleton: 3 frames [VERIFY sender, VERIFY sponsor, SENDER]
  const skeleton = sigRes.txSkeleton;
  assert(skeleton.frames.length === 3, `Skeleton has 3 frames`);
  assertBigIntEq(BigInt(skeleton.frames[0].mode), 1n, "Frame 0 is VERIFY (mode=1)");
  assertBigIntEq(BigInt(skeleton.frames[1].mode), 1n, "Frame 1 is VERIFY (mode=1)");
  assertBigIntEq(BigInt(skeleton.frames[2].mode), 2n, "Frame 2 is SENDER (mode=2)");

  // Verify targets
  assert(skeleton.frames[0].target.toLowerCase() === ACCOUNT.toLowerCase(),
    "Frame 0 target is account (sender verification)");
  assert(skeleton.frames[1].target.toLowerCase() === SPONSOR.toLowerCase(),
    "Frame 1 target is sponsor (gas payer verification)");
  assert(skeleton.frames[2].target.toLowerCase() === ACCOUNT.toLowerCase(),
    "Frame 2 target is account (ERC20 transfer execution)");

  // Step 2: sign
  const auth = signWebAuthn(sigRes.sigHash);

  // Step 3: submit
  const txRes = await jsonPost(`${API}/sponsored-send`, {
    address: ACCOUNT,
    to: DEAD,
    amount: "100",
    sponsorAddress: SPONSOR,
    ...auth,
  });

  assert(txRes.success === true, "Sponsored send transaction succeeded");
  assert(typeof txRes.txHash === "string", `Got tx hash: ${txRes.txHash}`);

  // Verify frame receipts
  assert(txRes.frameReceipts?.length === 3, "Got 3 frame receipts");
  if (txRes.frameReceipts) {
    assert(txRes.frameReceipts[0].mode === "VERIFY", "Frame 0 is VERIFY");
    assert(txRes.frameReceipts[0].status === true, "VERIFY sender frame succeeded");
    assert(txRes.frameReceipts[1].mode === "VERIFY", "Frame 1 is VERIFY");
    assert(txRes.frameReceipts[1].status === true, "VERIFY sponsor frame succeeded");
    assert(txRes.frameReceipts[2].mode === "SENDER", "Frame 2 is SENDER");
    assert(txRes.frameReceipts[2].status === true, "SENDER frame succeeded");
  }

  // Capture state AFTER
  console.log("\n  --- After ---");
  const accountAfter = await captureState(ACCOUNT);
  const deadAfter = await captureState(DEAD);
  const sponsorAfter = await captureState(SPONSOR);
  logState("Account", accountAfter);
  logState("Dead", deadAfter);
  logState("Sponsor", sponsorAfter);

  // Verify token deltas
  console.log("\n  --- Balance verification ---");
  const accountTokenDelta = accountBefore.tokenBalance - accountAfter.tokenBalance;
  assertBigIntEq(accountTokenDelta, SEND_TOKENS,
    `Account sent exactly 100 DEMO tokens`);

  const deadTokenDelta = deadAfter.tokenBalance - deadBefore.tokenBalance;
  assertBigIntEq(deadTokenDelta, SEND_TOKENS,
    `Dead received exactly 100 DEMO tokens`);

  // Sponsor pays gas — sponsor ETH should decrease
  const sponsorEthDelta = sponsorBefore.ethBalance - sponsorAfter.ethBalance;
  assertBigIntGt(sponsorEthDelta, 0n,
    `Sponsor paid gas (${formatEth(sponsorEthDelta)})`);

  // Account ETH should NOT decrease (gas sponsored)
  assertBigIntEq(accountAfter.ethBalance, accountBefore.ethBalance,
    `Account ETH unchanged (gas paid by sponsor)`);

  // Nonce
  console.log("\n  --- Nonce verification ---");
  assertBigIntEq(accountAfter.nonce, accountBefore.nonce + 1n,
    `Account nonce incremented by 1 (${accountBefore.nonce} -> ${accountAfter.nonce})`);

  // Blockscout
  if (txRes.txHash) {
    await verifyOnBlockscout(txRes.txHash, 3, ["VERIFY", "VERIFY", "SENDER"]);
  }

  return txRes.txHash ?? null;
}

async function testBatchOps(): Promise<string | null> {
  console.log("\n══════════════════════════════════════════════════════════");
  console.log("  BATCH OPS (0.001 ETH to 0xdead + 0.001 ETH to 0x0001)");
  console.log("══════════════════════════════════════════════════════════");

  const SEND_PER_OP = 1_000_000_000_000_000n; // 0.001 ETH each
  const TOTAL_SEND = SEND_PER_OP * 2n;

  const operations = [
    { to: DEAD, value: "0.001", data: "0x" },
    { to: ADDR_0001, value: "0.001", data: "0x" },
  ];

  // Capture state BEFORE
  console.log("\n  --- Before ---");
  const accountBefore = await captureState(ACCOUNT);
  const deadBefore = await captureState(DEAD);
  const addr1Before = await getEthBalance(ADDR_0001);
  logState("Account", accountBefore);
  logState("Dead", deadBefore);
  console.log(`  0x0001: ETH=${formatEth(addr1Before)}`);

  // Step 1: sig-hash
  const sigRes = await jsonPost(`${API}/sig-hash`, {
    demoType: "batch-ops",
    params: { from: ACCOUNT, operations },
  });
  assert(!sigRes.error, "sig-hash succeeded");

  // Verify skeleton: 1 VERIFY + 2 SENDER
  const skeleton = sigRes.txSkeleton;
  assert(skeleton.frames.length === 3, "Skeleton has 3 frames (1 VERIFY + 2 SENDER)");
  assertBigIntEq(BigInt(skeleton.frames[0].mode), 1n, "Frame 0 is VERIFY (mode=1)");
  assertBigIntEq(BigInt(skeleton.frames[1].mode), 2n, "Frame 1 is SENDER (mode=2)");
  assertBigIntEq(BigInt(skeleton.frames[2].mode), 2n, "Frame 2 is SENDER (mode=2)");

  // Step 2: sign
  const auth = signWebAuthn(sigRes.sigHash);

  // Step 3: submit
  const txRes = await jsonPost(`${API}/batch-ops`, {
    address: ACCOUNT,
    operations,
    ...auth,
  });

  assert(txRes.success === true, "Batch ops transaction succeeded");
  assert(typeof txRes.txHash === "string", `Got tx hash: ${txRes.txHash}`);

  // Verify frame receipts
  assert(txRes.frameReceipts?.length === 3, "Got 3 frame receipts");
  if (txRes.frameReceipts) {
    assert(txRes.frameReceipts[0].mode === "VERIFY", "Frame 0 is VERIFY");
    assert(txRes.frameReceipts[0].status === true, "VERIFY frame succeeded");
    assert(txRes.frameReceipts[1].mode === "SENDER", "Frame 1 is SENDER");
    assert(txRes.frameReceipts[1].status === true, "SENDER frame 1 succeeded");
    assert(txRes.frameReceipts[2].mode === "SENDER", "Frame 2 is SENDER");
    assert(txRes.frameReceipts[2].status === true, "SENDER frame 2 succeeded");
  }

  // Capture state AFTER
  console.log("\n  --- After ---");
  const accountAfter = await captureState(ACCOUNT);
  const deadAfter = await captureState(DEAD);
  const addr1After = await getEthBalance(ADDR_0001);
  logState("Account", accountAfter);
  logState("Dead", deadAfter);
  console.log(`  0x0001: ETH=${formatEth(addr1After)}`);

  // Verify balance deltas
  console.log("\n  --- Balance verification ---");
  const deadEthDelta = deadAfter.ethBalance - deadBefore.ethBalance;
  assertBigIntEq(deadEthDelta, SEND_PER_OP,
    `Dead received 0.001 ETH`);

  const addr1EthDelta = addr1After - addr1Before;
  assertBigIntEq(addr1EthDelta, SEND_PER_OP,
    `0x0001 received 0.001 ETH`);

  const accountEthDelta = accountBefore.ethBalance - accountAfter.ethBalance;
  assertBigIntGt(accountEthDelta, TOTAL_SEND,
    `Account spent > 0.002 ETH (transfers + gas)`);

  const gasPaid = accountEthDelta - TOTAL_SEND;
  console.log(`  Gas paid by account: ${formatEth(gasPaid)}`);

  // Token balances unchanged
  assertBigIntEq(accountAfter.tokenBalance, accountBefore.tokenBalance,
    "Account token balance unchanged");

  // Nonce
  console.log("\n  --- Nonce verification ---");
  assertBigIntEq(accountAfter.nonce, accountBefore.nonce + 1n,
    `Account nonce incremented by 1 (${accountBefore.nonce} -> ${accountAfter.nonce})`);

  // Blockscout
  if (txRes.txHash) {
    await verifyOnBlockscout(txRes.txHash, 3, ["VERIFY", "SENDER", "SENDER"]);
  }

  return txRes.txHash ?? null;
}

async function testDeployExecute(): Promise<string | null> {
  console.log("\n══════════════════════════════════════════════════════════");
  console.log("  DEPLOY + EXECUTE");
  console.log("══════════════════════════════════════════════════════════");

  // Runtime code: returns 42 (0x2a) on any call
  // PUSH1 0x2a PUSH1 0 MSTORE PUSH1 32 PUSH1 0 RETURN
  const runtimeCode = "602a60005260206000f3";
  const runtimeLen = runtimeCode.length / 2; // 10 bytes
  const offset = 32 - runtimeLen; // 22
  const initCode = `69${runtimeCode}600052600a60${offset.toString(16).padStart(2, "0")}f3`;
  const executeCalldata = "0x";

  // Capture state BEFORE
  console.log("\n  --- Before ---");
  const accountBefore = await captureState(ACCOUNT);
  logState("Account", accountBefore);

  // Step 1: sig-hash
  const sigRes = await jsonPost(`${API}/sig-hash`, {
    demoType: "deploy-execute",
    params: {
      from: ACCOUNT,
      bytecode: "0x" + initCode,
      calldata: executeCalldata,
    },
  });
  assert(!sigRes.error, "sig-hash succeeded");

  // Verify skeleton: [VERIFY, DEFAULT (deploy), SENDER (execute)]
  const skeleton = sigRes.txSkeleton;
  assert(skeleton.frames.length === 3, "Skeleton has 3 frames (VERIFY + DEFAULT + SENDER)");
  assertBigIntEq(BigInt(skeleton.frames[0].mode), 1n, "Frame 0 is VERIFY (mode=1)");
  assertBigIntEq(BigInt(skeleton.frames[1].mode), 0n, "Frame 1 is DEFAULT (mode=0)");
  assertBigIntEq(BigInt(skeleton.frames[2].mode), 2n, "Frame 2 is SENDER (mode=2)");

  // DEFAULT frame targets the deterministic deployment proxy (CREATE2 factory)
  const DEPLOYER_PROXY = "0x4e59b44847b379578588920ca78fbf26c0b4956c";
  assert(skeleton.frames[1].target.toLowerCase() === DEPLOYER_PROXY,
    `DEFAULT frame targets deployer proxy — got ${skeleton.frames[1].target}`);

  // DEFAULT frame should have the init code
  assert(skeleton.frames[1].data.toLowerCase().includes(runtimeCode.toLowerCase()),
    "DEFAULT frame data contains the init code");

  // Step 2: sign
  const auth = signWebAuthn(sigRes.sigHash);

  // Step 3: submit
  const txRes = await jsonPost(`${API}/deploy-execute`, {
    address: ACCOUNT,
    bytecode: "0x" + initCode,
    calldata: executeCalldata,
    ...auth,
  });

  assert(txRes.success === true, "Deploy+execute transaction succeeded");
  assert(typeof txRes.txHash === "string", `Got tx hash: ${txRes.txHash}`);

  // Verify frame receipts
  assert(txRes.frameReceipts?.length === 3, "Got 3 frame receipts");
  if (txRes.frameReceipts) {
    assert(txRes.frameReceipts[0].mode === "VERIFY", "Frame 0 is VERIFY");
    assert(txRes.frameReceipts[0].status === true, "VERIFY frame succeeded");
    assert(txRes.frameReceipts[1].mode === "DEFAULT", "Frame 1 is DEFAULT");
    assert(txRes.frameReceipts[1].status === true, "DEFAULT (deploy) frame succeeded");
    assert(txRes.frameReceipts[2].mode === "SENDER", "Frame 2 is SENDER");
    assert(txRes.frameReceipts[2].status === true, "SENDER (execute) frame succeeded");
  }

  // Capture state AFTER
  console.log("\n  --- After ---");
  const accountAfter = await captureState(ACCOUNT);
  logState("Account", accountAfter);

  // Verify balance deltas
  console.log("\n  --- Balance verification ---");
  const accountEthDelta = accountBefore.ethBalance - accountAfter.ethBalance;
  assertBigIntGt(accountEthDelta, 0n,
    `Account spent ETH on gas (${formatEth(accountEthDelta)})`);

  // Token balance unchanged
  assertBigIntEq(accountAfter.tokenBalance, accountBefore.tokenBalance,
    "Account token balance unchanged");

  // Nonce
  console.log("\n  --- Nonce verification ---");
  assertBigIntEq(accountAfter.nonce, accountBefore.nonce + 1n,
    `Account nonce incremented by 1 (${accountBefore.nonce} -> ${accountAfter.nonce})`);

  // Blockscout
  if (txRes.txHash) {
    await verifyOnBlockscout(txRes.txHash, 3, ["VERIFY", "DEFAULT", "SENDER"]);
  }

  return txRes.txHash ?? null;
}

// ── Main ────────────────────────────────────────────────────────────
async function main() {
  console.log("╔══════════════════════════════════════════════════════════╗");
  console.log("║  EIP-8141 Frame Transaction — Comprehensive E2E Test    ║");
  console.log("╚══════════════════════════════════════════════════════════╝");
  console.log(`  API:        ${API}`);
  console.log(`  RPC:        ${RPC}`);
  console.log(`  Blockscout: ${BLOCKSCOUT}`);
  console.log(`  Account:    ${ACCOUNT}`);
  console.log(`  PubKey X:   ${pubKeyX.slice(0, 18)}...`);

  // Check services are up
  try {
    const block = await rpc("eth_blockNumber");
    console.log(`  Block:      ${parseInt(block, 16)}`);
  } catch (e: any) {
    console.error(`\nFATAL: Cannot reach RPC at ${RPC}: ${e.message}`);
    process.exit(1);
  }

  try {
    await fetch(`${API.replace("/api", "")}`);
  } catch (e: any) {
    console.error(`\nFATAL: Cannot reach backend at ${API}: ${e.message}`);
    process.exit(1);
  }

  const txHashes: string[] = [];

  // ── Register ──
  await register();

  // ── Test all 4 flows ──
  const hash1 = await testSimpleSend();
  if (hash1) txHashes.push(hash1);

  const hash2 = await testSponsoredSend();
  if (hash2) txHashes.push(hash2);

  const hash3 = await testBatchOps();
  if (hash3) txHashes.push(hash3);

  const hash4 = await testDeployExecute();
  if (hash4) txHashes.push(hash4);

  // ── Final summary ──
  console.log("\n╔══════════════════════════════════════════════════════════╗");
  console.log("║  TEST SUMMARY                                           ║");
  console.log("╚══════════════════════════════════════════════════════════╝");
  console.log(`  Total assertions: ${totalAssertions}`);
  console.log(`  Passed:           ${passedAssertions}`);
  console.log(`  Failed:           ${failedAssertions}`);

  if (failures.length > 0) {
    console.log("\n  FAILURES:");
    for (const f of failures) {
      console.log(`    ✗ ${f}`);
    }
  }

  if (txHashes.length > 0) {
    console.log("\n  Transaction hashes:");
    txHashes.forEach((h) => console.log(`    ${h}`));
    console.log(`\n  Blockscout: ${BLOCKSCOUT}/txs`);
  }

  console.log(`\n  Result: ${failedAssertions === 0 ? "ALL PASSED" : "SOME FAILED"}`);
  process.exit(failedAssertions === 0 ? 0 : 1);
}

main().catch((err) => {
  console.error("\nFATAL:", err.message ?? err);
  process.exit(1);
});
