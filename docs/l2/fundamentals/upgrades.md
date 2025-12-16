# Upgrades

## Sequencer and prover versions

Each committed batch stores the git commit hash of the sequencer build that produced it. The OnChainProposer uses that commit hash to look up the verifier key in its `verificationKeys` mapping. When the sequencer is upgraded, all batches committed before the upgrade must be proved with the prover matching the old version, and all batches committed after the upgrade must be proved with a prover built from the new version.

## Registering a new verification key

To allow proofs from a new sequencer/prover build, register its verification key against the commit hash:

1. Compute the commit hash as the Keccak-256 of the (reduced) git commit. For example, the commit `9219410` produces `b9105485bc4ba523201eaaf76478a47b259fa7399bbed795cf19294861b7fc57`.
2. From the OnChainProposer owner account, send the upgrade transaction. Example (replace addresses and keys with your values):
   ```
   rex send <ON_CHAIN_PROPOSER_ADDRESS> \
     "upgradeSP1VerificationKey(bytes32,bytes32)" \
     <KECCAK_GIT_COMMIT> \
     <VERIFICATION_KEY> \
     --private-key <ON_CHAIN_PROPOSER_OWNER_PK>
   ```
3. (Optional) Verify the mapping entry:
   ```
   rex call <ON_CHAIN_PROPOSER_ADDRESS> \
     "verificationKeys(bytes32,uint8)(bytes32)" \
     <KECCAK_GIT_COMMIT> \
     <PROVER_ID>
   ```
   `1` is the SP1 verifier ID, `2` is RISC0.

### Verification key artifacts

The verification key that goes on-chain is obtained when you build the prover.

For SP1 it is stored at:

  - `crates/l2/prover/src/guest_program/src/sp1/out/riscv32im-succinct-zkvm-vk-bn254`.

  - If proving with Aligned, use the `u32` form generated alongside it at `crates/l2/prover/src/guest_program/src/sp1/out/riscv32im-succinct-zkvm-vk-u32`.

For RISC0 it is stored at:
  - `crates/l2/prover/src/guest_program/src/risc0/out/riscv32im-risc0-vk`
