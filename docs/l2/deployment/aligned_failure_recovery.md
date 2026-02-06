# Aligned Layer Failure Recovery Guide

This guide provides operators with procedures for handling various Aligned Layer failure scenarios when running ethrex L2 in Aligned mode.

> **SDK Version**: This documentation is based on Aligned Aggregation Mode SDK revision `54ca2471624700536561b6bd369ed9f4d327991e`.

> **WARNING**: This document is intended to be iterated and improved with use and with ethrex and Aligned upgrades. If you encounter a scenario not covered here or find that a procedure needs adjustment, please contribute improvements.

## Table of Contents

1. [Understanding the Aligned Integration](#understanding-the-aligned-integration)
2. [Scenario 1: Aligned Stops Then Recovers](#scenario-1-aligned-stops-then-recovers)
3. [Scenario 2: Aligned Loses a Proof Before Verification](#scenario-2-aligned-loses-a-proof-before-verification)
4. [Scenario 3: Aligned Permanent Shutdown](#scenario-3-aligned-permanent-shutdown)
5. [Scenario 4: Insufficient Quota Balance](#scenario-4-insufficient-quota-balance)
6. [Scenario 5: Proof Marked as Invalid by Aligned](#scenario-5-proof-marked-as-invalid-by-aligned)
7. [Monitoring and Detection](#monitoring-and-detection)

---

## Understanding the Aligned Integration

Before handling failures, understand the proof lifecycle in Aligned mode:

```
1. Prover generates compressed SP1 proof
2. L1ProofSender submits proof to Aligned Gateway (HTTP)
3. Aligned Gateway queues proof for aggregation
4. Aligned Aggregator aggregates multiple proofs (typically every 24 hours)
5. Aggregated proof is posted to L1 (AlignedProofAggregationService)
6. L1ProofVerifier polls for aggregation status
7. Once aggregated, L1ProofVerifier calls verifyBatchesAligned() on OnChainProposer
```

**Key Components**:
- **L1ProofSender**: Submits SP1 proofs to Aligned Gateway via HTTP
- **L1ProofVerifier**: Polls Aligned for aggregation status and triggers on-chain verification
- **Aligned Gateway**: Receives and queues proofs via HTTP REST API
- **Aligned Aggregator**: Aggregates proofs and posts to L1

> **Note**: Aligned mode only supports SP1 proofs.

---

## Scenario 1: Aligned Stops Then Recovers

### Symptoms
- L1ProofSender logs show connection errors to Aligned Gateway
- Proofs are generated but not being sent
- `AlignedGetNonceError` in logs (failed to get nonce from gateway)
- `AlignedSubmitProofError` in logs (HTTP request to gateway failed)

> **Note**: `AlignedFeeEstimateError` indicates your Ethereum RPC endpoints are failing, not Aligned. Fee estimation uses your configured `--eth.rpc-url` to query L1 gas prices.

### Impact
- Proofs queue locally in the rollup store
- Batch verification on L1 stalls
- No data loss - proofs remain in local storage

### Recovery Steps

**No manual intervention required.** The system handles this automatically:

1. **L1ProofSender** continuously retries sending proofs at the configured interval (`--proof-coordinator.send-interval`, default 30000ms)
2. Once Aligned recovers, proofs will be submitted in order
3. **L1ProofVerifier** will resume polling and verification

### What to Monitor

#### Check L1ProofSender status
```bash
curl -X GET http://localhost:5555/health | jq '.proof_sender'
```
#### Watch logs for recovery
```
# Look for: "Submitting proof to Aligned" followed by "Submitted proof to Aligned"
```

### Configuration Tips

- Consider increasing `--proof-coordinator.send-interval` during known Aligned maintenance windows to reduce log noise

---

## Scenario 2: Aligned Loses a Proof Before Verification

### Symptoms
- Proof was successfully submitted (logs show "Submitted proof to Aligned")
- L1ProofVerifier shows "Proof has not been aggregated by Aligned" for extended periods
- No aggregation event for the proof on Aligned's side

### Impact
- Chain verification is blocked at the lost proof's batch number
- Proofs for subsequent batches continue to generate and queue

### Recovery Steps

#### Step 1: Confirm the Proof is Lost

**Check what batch the system thinks it sent:**

```sql
-- Check the latest sent batch pointer (SQL storage)
SELECT batch FROM latest_sent WHERE _id = 0;
```

**Check if the proof exists locally:**

```sql
-- Check if proof exists in the rollup store (SQL storage)
-- prover_type: 1 = RISC0, 2 = SP1
SELECT batch, prover_type, length(proof) as proof_size
FROM batch_proofs
WHERE batch = <BATCH_NUMBER>;
```

**Find the nonce used for the batch:**

When the L1ProofSender submits a proof to Aligned, it logs the batch number and the nonce used:

```
INFO ethrex_l2::sequencer::l1_proof_sender: Submitted proof to Aligned batch_number=5 nonce=42 task_id=...
```

**Important**: Pay attention to these logs and note down the `batch_number` → `nonce` mapping. The nonce is needed to verify if the gateway received the proof.

**Check if Aligned has aggregated the proof on-chain:**

Use the Aligned CLI's `verify-on-chain` command:

```bash
cd aligned_layer/aggregation_mode/cli

cargo run --release -- verify-on-chain \
  --network <NETWORK> \
  --rpc-url <RPC_URL> \
  --beacon-url <BEACON_URL> \
  --proving-system sp1 \
  --vk-hash <VK_HASH_FILE> \
  --public-inputs <PUBLIC_INPUTS_FILE>
```

The L1ProofVerifier also continuously checks this - if it keeps logging "has not yet been aggregated" for an extended period, the proof likely wasn't received by Aligned.

**Check if the gateway received the proof:**

The SDK provides `get_receipts_for(address, nonce)` to check if a proof is in the gateway's database:

```rust
use aligned_sdk::gateway::AggregationModeGatewayProvider;

let gateway = AggregationModeGatewayProvider::new(network);
let receipts = gateway.get_receipts_for(proof_sender_address, Some(nonce)).await?;

for receipt in receipts {
    println!("Nonce: {}, Status: {}", receipt.nonce, receipt.status);
}
```

If the proof exists locally, `latest_sent` shows the batch was sent, but neither the gateway has a receipt nor Aligned has aggregated it after an extended period, the proof was likely lost.

#### Step 2: Reset the Latest Sent Batch Pointer

The proof still exists in the database - the system just thinks it was already sent. Reset the `latest_sent` value to make the L1ProofSender resend it:

```sql
-- Reset to the batch before the lost one (SQL storage)
-- This will cause L1ProofSender to resend batch N on the next iteration
UPDATE latest_sent SET batch = <BATCH_NUMBER - 1> WHERE _id = 0;
```

For example, if batch 5 was lost:
```sql
UPDATE latest_sent SET batch = 4 WHERE _id = 0;
```

> **Note**: It's safe to resend a proof even if Aligned didn't actually lose it. Aligned treats each submission with a different nonce as a separate entry, so the SDK will return `Ok` and queue the proof again. If the original proof was already aggregated, the L1ProofVerifier will find it when checking the commitment (which is deterministic based on vk + public inputs). The only downside is paying an extra aggregation fee for the duplicate submission.

#### Step 3: Wait for Automatic Resend

Once the pointer is reset:

1. **L1ProofSender** will detect that batch N needs to be sent
2. The existing proof will be retrieved from the database
3. The proof will be resubmitted to Aligned

No proof regeneration is needed since the proof data is still stored locally.

### Prevention

- Monitor proof submission success rates
- Set up alerts for proofs stuck in "not aggregated" state for >N minutes
- Keep the quota balance funded (see Scenario 4)

---

## Scenario 3: Aligned Permanent Shutdown

### Symptoms
- Sustained inability to connect to Aligned Gateway
- Aligned team confirms permanent shutdown or migration

### Impact
- **Critical**: Batch verification on L1 is completely blocked
- Users cannot withdraw funds (withdrawals require batch verification)
- L2 can continue producing blocks but they won't be finalized

### Recovery Steps

This requires **switching from Aligned mode to Standard mode**.

#### Step 1: Stop the L2 Node

- Gracefully stop the sequencer
- This prevents new proofs from being generated in the wrong format

#### Step 2: Upgrade OnChainProposer Contract

The OnChainProposer contract needs to be reconfigured through a timelock upgrade. See the [Upgrades documentation](./upgrades.md) for the upgrade procedure.

**Configuration changes:**
- Set `ALIGNED_MODE = false`
- Enable the direct verifiers (`REQUIRE_SP1_PROOF`, `REQUIRE_RISC0_PROOF`)
- Set verifier contract addresses (`SP1_VERIFIER_ADDRESS`, `RISC0_VERIFIER_ADDRESS`)

#### Step 3: Clear Incompatible Proofs

Proofs generated in Compressed format (for Aligned) are incompatible with Standard mode (Groth16). Delete all unverified proofs:

```sql
-- Get the last verified batch from L1 (check OnChainProposer.lastVerifiedBatch())
-- Then delete all proofs for batches after that
DELETE FROM batch_proofs WHERE batch > <LAST_VERIFIED_BATCH>;
```

#### Step 4: Restart Node in Standard Mode

Update your node configuration to disable Aligned mode:

```bash
# Remove Aligned-specific flags
ethrex l2 \
  # ... other flags ...
  # DO NOT include: --aligned
  # DO NOT include: --aligned-network
  # DO NOT include: --aligned.beacon-url
```

#### Step 5: Regenerate Proofs in Groth16 Format

1. Restart the prover(s) - they will automatically generate Groth16 proofs (since `--aligned` is not set)
2. ProofCoordinator will request proofs starting from `lastVerifiedBatch + 1`
3. L1ProofSender will submit directly to OnChainProposer.verifyBatch()


---

## Scenario 4: Insufficient Quota Balance

### Symptoms
- Proof submission fails with insufficient balance/quota errors
- L1ProofSender logs show: `AlignedSubmitProofError` with insufficient quota message

The error from the Aligned SDK looks like:
```
Submit error: Insufficient balance, address: 0x<YOUR_PROOF_SENDER_ADDRESS>
```

### Impact
- New proofs cannot be submitted to Aligned
- Verification stalls for new batches

### Recovery Steps

#### Step 1: Deposit More Funds

Using the Aligned CLI from the `aligned_layer` repository:

```bash
cd aligned_layer/aggregation_mode/cli

cargo run --release -- deposit \
  --private-key <PROOF_SENDER_PRIVATE_KEY> \
  --network <NETWORK> \
  --rpc-url <RPC_URL>
```

Where `<NETWORK>` is one of: `devnet`, `hoodi`, or `mainnet`.

### Prevention

- Monitor the `AggregationModePaymentService` contract for your address's quota balance
- Track proof submission frequency to estimate quota consumption
- Consider depositing a larger buffer to reduce maintenance frequency

---

## Scenario 5: Proof Marked as Invalid by Aligned

### Symptoms
- Logs show: "Proof is invalid, will be deleted"
- Aligned returns `InvalidProof` error during submission

### Impact
- Invalid proof is automatically deleted from local storage
- Proof regeneration is triggered automatically

### Recovery Steps

**Automatic recovery** - the system handles this:

1. L1ProofSender detects `InvalidProof` error
2. Proof is deleted from rollup store
3. ProofCoordinator detects missing proof
4. New proof is requested from prover
5. Fresh proof is submitted

### Investigation

If proofs are repeatedly marked invalid:

1. **Check prover version compatibility**: Ensure prover ELF/VK matches the deployed contract
2. **Verify public inputs**: Mismatched batch data can cause invalid proofs
3. **Check Aligned network**: Ensure you're using the correct network (devnet/testnet/mainnet)

---


## Monitoring and Detection

### Key Log Messages

| Message | Component | Meaning |
|---------|-----------|---------|
| `Sending batch proof(s) to Aligned Layer` | L1ProofSender | Proof submission starting |
| `Submitted proof to Aligned` | L1ProofSender | Proof sent successfully |
| `Proof is invalid, will be deleted` | L1ProofSender | Aligned rejected the proof |
| `Failed to create gateway` | L1ProofSender | Gateway connection issue |
| `Proof aggregated by Aligned` | L1ProofVerifier | Aggregation confirmed |
| `has not yet been aggregated` | L1ProofVerifier | Waiting for aggregation |
| `Batches verified in OnChainProposer` | L1ProofVerifier | On-chain verification complete |

### Health Check Endpoint

```bash
curl -X GET http://localhost:5555/health | jq
```

The response includes:
- `proof_sender`: L1ProofSender status and configuration
- `network`: Aligned network being used
- `fee_estimate`: Fee estimation type (instant/default)

### Contract Error Codes

| Code | Meaning | Action |
|------|---------|--------|
| `00h` | Use verifyBatch instead | Contract not in Aligned mode |
| `00m` | Invalid Aligned proof | Proof will be deleted and regenerated |
| `00y` | AlignedProofAggregator call failed | Check aggregator contract address |
| `00z` | Aligned proof verification failed | Merkle proof invalid |

---

## Summary

| Scenario | Automatic Recovery | Manual Intervention |
|----------|-------------------|---------------------|
| Aligned temporary outage | Yes | None needed |
| Proof lost before verification | No | Reset `latest_sent` pointer to trigger resend |
| Aligned permanent shutdown | No | Switch to Standard mode |
| Insufficient quota balance | No | Deposit funds |
| Proof marked invalid | Yes | None needed |

---

## References

- [Aligned Layer Integration](../fundamentals/ethrex_l2_aligned_integration.md)
- [Running Ethrex in Aligned Mode](./aligned.md)
- [Aligned Layer Documentation](https://docs.alignedlayer.com/)
- [Upgrades Guide](./upgrades.md)
