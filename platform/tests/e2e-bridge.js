#!/usr/bin/env node
/**
 * E2E test: Full bridge cycle on a running zk-dex deployment.
 * Tests: deposit → withdraw → batch commit → proof verify → claim
 *
 * Usage:
 *   node platform/tests/e2e-bridge.js [--l1-port 8547] [--l2-port 1731] [--bridge <addr>]
 *
 * If no flags are given, it reads the latest running deployment from the DB.
 */

const { ethers } = require("ethers");
const { execSync } = require("child_process");
const path = require("path");

// ── Config ────────────────────────────────────────────────────────

const ROOT = path.resolve(__dirname, "../..");
const DB_PATH = path.join(ROOT, "platform/server/db/platform.sqlite");

const PRIVATE_KEY = "0xab63b23eb7941c1251757e24b3d2350d2bc05c3c388d06f8fe6feafefb1e8c70";

// Parse CLI args
const args = process.argv.slice(2);
function getArg(name) {
  const idx = args.indexOf(name);
  return idx >= 0 && args[idx + 1] ? args[idx + 1] : null;
}

let L1_PORT = getArg("--l1-port");
let L2_PORT = getArg("--l2-port");
let BRIDGE_ADDRESS = getArg("--bridge");
let PROPOSER_ADDRESS = getArg("--proposer");

// Auto-detect from DB if not provided
if (!L1_PORT || !L2_PORT || !BRIDGE_ADDRESS) {
  try {
    const row = execSync(
      `sqlite3 "${DB_PATH}" "SELECT l1_port, l2_port, bridge_address, proposer_address FROM deployments WHERE phase='running' ORDER BY rowid DESC LIMIT 1;"`,
      { encoding: "utf-8" }
    ).trim();
    if (row) {
      const [p1, p2, ba, pa] = row.split("|");
      L1_PORT = L1_PORT || p1;
      L2_PORT = L2_PORT || p2;
      BRIDGE_ADDRESS = BRIDGE_ADDRESS || ba;
      PROPOSER_ADDRESS = PROPOSER_ADDRESS || pa;
    }
  } catch {
    // ignore
  }
}

if (!L1_PORT || !L2_PORT || !BRIDGE_ADDRESS) {
  console.error("ERROR: Could not determine deployment. Pass --l1-port, --l2-port, --bridge or ensure a running deployment exists.");
  process.exit(1);
}

const L1_URL = `http://127.0.0.1:${L1_PORT}`;
const L2_URL = `http://127.0.0.1:${L2_PORT}`;

// ── Contract ABIs ────────────────────────────────────────────────

const L1_BRIDGE_ABI = [
  "function deposit(address l2Recipient) payable",
  "function claimWithdrawal(uint256 claimedAmount, uint256 withdrawalBatchNumber, uint256 withdrawalMessageId, bytes32[] calldata withdrawalProof)",
];

const PROPOSER_ABI = [
  "function lastCommittedBatch() view returns (uint256)",
  "function lastVerifiedBatch() view returns (uint256)",
];

// CommonBridgeL2 at 0x000000000000000000000000000000000000ffff
const L2_BRIDGE_ADDRESS = "0x000000000000000000000000000000000000ffff";
const L2_BRIDGE_ABI = [
  "function withdraw(address _receiverOnL1) payable",
];

// ── Test runner ──────────────────────────────────────────────────

let passed = 0;
let failed = 0;

function assert(condition, msg) {
  if (condition) {
    console.log(`  PASS: ${msg}`);
    passed++;
  } else {
    console.error(`  FAIL: ${msg}`);
    failed++;
  }
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

// Retry wrapper for flaky RPC calls (socket hang up, etc.)
async function retry(fn, retries = 3, delayMs = 2000) {
  for (let i = 0; i < retries; i++) {
    try {
      return await fn();
    } catch (e) {
      if (i === retries - 1) throw e;
      await sleep(delayMs);
    }
  }
}

async function main() {
  console.log("=== E2E Bridge Full Cycle Test ===");
  console.log(`L1: ${L1_URL}`);
  console.log(`L2: ${L2_URL}`);
  console.log(`Bridge: ${BRIDGE_ADDRESS}`);
  console.log(`Proposer: ${PROPOSER_ADDRESS}`);
  console.log();

  const l1Provider = new ethers.JsonRpcProvider(L1_URL, undefined, { staticNetwork: true });
  const l2Provider = new ethers.JsonRpcProvider(L2_URL, undefined, { staticNetwork: true });
  const l1Wallet = new ethers.Wallet(PRIVATE_KEY, l1Provider);
  const l2Wallet = new ethers.Wallet(PRIVATE_KEY, l2Provider);
  const address = l1Wallet.address;

  console.log(`Test account: ${address}`);
  console.log();

  // ── Test 1: Health check ───────────────────────────────────
  console.log("[1] Health check");
  const l1Block = await l1Provider.getBlockNumber();
  assert(l1Block >= 0, `L1 block number = ${l1Block}`);
  const l2Block = await l2Provider.getBlockNumber();
  assert(l2Block >= 0, `L2 block number = ${l2Block}`);
  const l1Network = await l1Provider.getNetwork();
  const l2Network = await l2Provider.getNetwork();
  console.log(`  L1 chainId: ${l1Network.chainId}, L2 chainId: ${l2Network.chainId}`);
  console.log();

  // ── Test 2: L1 balance ─────────────────────────────────────
  console.log("[2] L1 balance check");
  const l1Balance = await l1Provider.getBalance(address);
  console.log(`  L1 balance: ${ethers.formatEther(l1Balance)} ETH`);
  assert(l1Balance > 0n, "L1 account has funds");
  console.log();

  // ── Test 3: Deposit ETH (L1 → L2) ─────────────────────────
  console.log("[3] Deposit ETH from L1 to L2");
  const depositAmount = ethers.parseEther("1.0");
  const l2BalanceBefore = await l2Provider.getBalance(address);
  console.log(`  L2 balance before: ${ethers.formatEther(l2BalanceBefore)} ETH`);

  const bridge = new ethers.Contract(BRIDGE_ADDRESS, L1_BRIDGE_ABI, l1Wallet);
  const depositTx = await bridge.deposit(address, { value: depositAmount });
  console.log(`  Deposit tx: ${depositTx.hash}`);
  const depositReceipt = await depositTx.wait();
  assert(depositReceipt.status === 1, `Deposit tx succeeded (block ${depositReceipt.blockNumber})`);

  console.log("  Waiting for L2 to process deposit (up to 120s)...");
  let l2BalanceAfterDeposit = l2BalanceBefore;
  const depositDeadline = Date.now() + 120000;
  while (Date.now() < depositDeadline) {
    await sleep(5000);
    l2BalanceAfterDeposit = await l2Provider.getBalance(address);
    if (l2BalanceAfterDeposit > l2BalanceBefore) break;
    process.stdout.write(".");
  }
  console.log();
  console.log(`  L2 balance after: ${ethers.formatEther(l2BalanceAfterDeposit)} ETH`);
  assert(l2BalanceAfterDeposit >= l2BalanceBefore + depositAmount, `L2 balance increased by >= 1.0 ETH`);
  console.log();

  // ── Test 4: Withdraw ETH (L2 → L1) ────────────────────────
  console.log("[4] Withdraw ETH from L2 to L1");
  const withdrawAmount = ethers.parseEther("0.5");
  const l2BalanceBeforeWithdraw = await l2Provider.getBalance(address);
  console.log(`  L2 balance before withdraw: ${ethers.formatEther(l2BalanceBeforeWithdraw)} ETH`);

  const l2Bridge = new ethers.Contract(L2_BRIDGE_ADDRESS, L2_BRIDGE_ABI, l2Wallet);
  const withdrawTx = await l2Bridge.withdraw(address, { value: withdrawAmount });
  console.log(`  Withdraw tx: ${withdrawTx.hash}`);
  const withdrawReceipt = await withdrawTx.wait();
  assert(withdrawReceipt.status === 1, `Withdraw tx succeeded (block ${withdrawReceipt.blockNumber})`);

  const l2BalanceAfterWithdraw = await l2Provider.getBalance(address);
  console.log(`  L2 balance after withdraw: ${ethers.formatEther(l2BalanceAfterWithdraw)} ETH`);
  // Balance should decrease by at least the withdrawal amount (plus gas)
  assert(l2BalanceAfterWithdraw < l2BalanceBeforeWithdraw, "L2 balance decreased after withdrawal");
  console.log();

  // ── Test 5: Wait for batch commit ──────────────────────────
  console.log("[5] Waiting for batch commit (up to 5 min)...");
  const proposer = new ethers.Contract(PROPOSER_ADDRESS, PROPOSER_ABI, l1Provider);
  const committedBefore = await retry(() => proposer.lastCommittedBatch());
  console.log(`  lastCommittedBatch before: ${committedBefore}`);

  const commitDeadline = Date.now() + 300000; // 5 min
  let committedAfter = committedBefore;
  while (Date.now() < commitDeadline) {
    await sleep(10000);
    committedAfter = await retry(() => proposer.lastCommittedBatch());
    process.stdout.write(`  committed=${committedAfter}\n`);
    if (committedAfter > committedBefore) break;
  }
  assert(committedAfter > committedBefore, `Batch committed: ${committedBefore} -> ${committedAfter}`);
  console.log();

  // ── Test 6: Wait for proof verification (THE KEY TEST) ─────
  console.log("[6] Waiting for proof verification (up to 15 min)...");
  console.log("  This verifies the WITHDRAWAL_GAS fix — if 00e occurs, verification will fail.");
  const verifiedBefore = await retry(() => proposer.lastVerifiedBatch());
  console.log(`  lastVerifiedBatch before: ${verifiedBefore}`);

  const verifyDeadline = Date.now() + 900000; // 15 min
  let verifiedAfter = verifiedBefore;
  while (Date.now() < verifyDeadline) {
    await sleep(15000);
    verifiedAfter = await retry(() => proposer.lastVerifiedBatch());
    const committed = await retry(() => proposer.lastCommittedBatch());
    console.log(`  committed=${committed} verified=${verifiedAfter}`);
    if (verifiedAfter > verifiedBefore) break;
  }
  assert(verifiedAfter > verifiedBefore, `Batch verified: ${verifiedBefore} -> ${verifiedAfter} (NO 00e error!)`);
  console.log();

  // ── Test 7: Get withdrawal proof from L2 ───────────────────
  console.log("[7] Getting withdrawal proof from L2");
  let proofData = null;
  try {
    const proofResult = await l2Provider.send("ethrex_getL1MessageProof", [withdrawTx.hash]);
    assert(proofResult && proofResult.length > 0, "Got withdrawal proof from L2");
    proofData = proofResult[0];
    console.log(`  batch_number: ${proofData.batch_number}`);
    console.log(`  message_id: ${proofData.message_id}`);
    console.log(`  merkle_proof length: ${(proofData.merkle_proof || []).length}`);
  } catch (e) {
    assert(false, `Failed to get withdrawal proof: ${e.message}`);
  }
  console.log();

  // ── Test 8: Claim withdrawal on L1 ─────────────────────────
  console.log("[8] Claiming withdrawal on L1");
  if (proofData) {
    try {
      // Wait until the batch containing our withdrawal is verified
      const batchNumber = BigInt(proofData.batch_number);
      console.log(`  Waiting for batch ${batchNumber} to be verified...`);
      const claimDeadline = Date.now() + 900000; // 15 min
      while (Date.now() < claimDeadline) {
        const verified = await retry(() => proposer.lastVerifiedBatch());
        if (verified >= batchNumber) {
          console.log(`  Batch ${batchNumber} is verified (lastVerified=${verified})`);
          break;
        }
        console.log(`  lastVerified=${verified}, need=${batchNumber}`);
        await sleep(15000);
      }

      const l1BalanceBefore = await l1Provider.getBalance(address);
      const claimTx = await bridge.claimWithdrawal(
        withdrawAmount,
        batchNumber,
        BigInt(proofData.message_id),
        proofData.merkle_proof || []
      );
      console.log(`  Claim tx: ${claimTx.hash}`);
      const claimReceipt = await claimTx.wait();
      assert(claimReceipt.status === 1, `Claim tx succeeded (block ${claimReceipt.blockNumber})`);

      const l1BalanceAfter = await l1Provider.getBalance(address);
      const l1Increase = l1BalanceAfter - l1BalanceBefore;
      // L1 balance should increase (claim amount minus gas)
      console.log(`  L1 balance change: ${ethers.formatEther(l1Increase)} ETH`);
      assert(l1Increase > 0n, "L1 balance increased after claim");
    } catch (e) {
      assert(false, `Claim failed: ${e.message}`);
    }
  } else {
    assert(false, "Skipped claim — no proof data");
  }
  console.log();

  // ── Summary ────────────────────────────────────────────────
  console.log("=== Summary ===");
  console.log(`Passed: ${passed}, Failed: ${failed}`);
  process.exit(failed > 0 ? 1 : 0);
}

main().catch((e) => {
  console.error("Fatal:", e);
  process.exit(1);
});
