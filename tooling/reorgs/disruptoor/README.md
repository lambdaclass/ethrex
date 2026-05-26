# Deep-reorg Kurtosis scenario

This directory contains the Kurtosis enclave config and assertoor task spec
for driving a deep reorg (>128 blocks) on a live EL+CL network and validating
that ethrex converges correctly.

## Files

| File | Purpose |
|---|---|
| `network-partition-deep-reorg.yaml` | Kurtosis enclave manifest (4 EL/CL pairs + disruptoor) |
| `assertions.yaml` | Assertoor task spec: partition, wait 200 slots, heal, assert |

## Installing Kurtosis

Follow the official guide: <https://docs.kurtosis.com/install>

Short version (Debian/Ubuntu):

```sh
echo "deb [trusted=yes] https://apt.fury.io/kurtosis-tech/ /" \
  | sudo tee /etc/apt/sources.list.d/kurtosis.list
sudo apt-get update && sudo apt-get install kurtosis-cli
kurtosis engine start
```

## Running the partition scenario locally

1. Build the ethrex Docker image from the current branch:

   ```sh
   docker build -t ethrex:local .
   ```

2. Launch the enclave:

   ```sh
   kurtosis run --enclave deep-reorg \
     github.com/ethpandaops/ethereum-package \
     --args-file tooling/reorgs/disruptoor/network-partition-deep-reorg.yaml
   ```

3. Tear down when done:

   ```sh
   kurtosis enclave rm deep-reorg --force
   ```

## CI

See `.github/workflows/deep_reorg.yaml` for the GitHub Actions version.
It is triggered only by `workflow_dispatch` (never on PRs).

## Tuning

The YAML configs are starting points. After the first run produces real data
you may need to adjust:

- `validator_count` per participant (affects finalization speed)
- `seconds_per_slot` (12 s is mainnet; lower for faster iteration locally)
- Partition duration (`minSlotDelta: 200` in `assertions.yaml`)
- Service names passed to disruptoor (inspect with `kurtosis service ls deep-reorg`)
