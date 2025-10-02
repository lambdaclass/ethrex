# Roadmap
This project is under active development. Over the next **two months**, our **primary objective is to finalize and audit the first version of the stack**.
This means every component — from L1 syncing to L2 bridging and prover integration — must meet stability, performance, and security standards.

The roadmap below outlines the remaining work required to achieve this milestone, organized into three major areas: **L2**, **DevOps & Performance**, and **L1**.

---

## L2 Roadmap

| Feature                     | Description                                                                                                                                                                                                                          | Status       |
|----------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|--------------|
| Native Bridge              | Secure and trust-minimized ERC-20 bridge between Ethereum L1 and L2 using canonical messaging and smart contracts.                                                                                                                   | In Progress  |
| Based Rollup               | Launch the rollup as a based permissionless rollup. Leverages Ethereum for sequencing and DA. For more information check [ethrex roadmap for becoming based](https://hackmd.io/TCa-bQisToW46enF58_3Vw?view)                                                          | In Progress    |
| Aligned Integration        | Optimize integration with Aligned’s aggregation mode.                                                                                                                                                                              | In Progress |
| Risc0 Support              | Integrate RISC Zero as an alternative zkVM to SP1, enabling configurable proving backends.                                                                                                                                                    | In Progress     |
| Battle-Test the Prover     |                                                               Ensure the prover (e.g., SP1, Risc0) is robust, correct, and performant under production-level  conditions.                                                                                 | In Progress    |
| One-Click L2 Deployment    | Deploy a fully operational rollup with a single command. Includes TDX, Prover, integrated Grafana metrics, alerting system, block explorer, bridge hub, backups and default configuration for rapid developer spin-up.                        | In Progress |
| Shared Bridge              | Direct bridging between multiple L2s to improve UX and avoid L1 costs.                                                                                                                                                           | Planned      |
| Custom Native Token        | Define a native token (non-ETH) for gas, staking, incentives, and governance. Fully integrated into fee mechanics and bridging.                                                                                                      | Planned  |
| Validiums & DACs           | Enhance Validium mode with Data Availability Committees.                                                                                                                                       | Planned      |
| Gas & Fees   | Set up a custom fee model to price deposits or any forced-included transaction, including data availability costs.                                                                                                                                                | Planned  |

---

## DevOps & Performance

| Initiative                   | Description                                                                                                                                                                                    | Status       |
|-----------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|--------------|
| Performance Benchmarking    | Continuous `ggas/s` measurement, client comparison, and reproducible load tests.                                                                                                               | In Progress|
| DB Optimizations            | Snapshots, background trie commits, parallel Merkle root calculation, and exploratory DB design.                                                                                                | In Progress |
| EVM Profiling               | Identify and optimize execution bottlenecks in the VM.                                                                                                                                          | In Progress  |
| Deployment & Dev Experience | One-command L2 launch, localnet spam testing, and L1 syncing on any network.                                                                                                                    | In Progress |

---

## L1 Roadmap

| Feature                  | Description                                                                                                                                                    | Status       |
|--------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------|--------------|
| P2P Improvements         | Use [spawned](https://github.com/lambdaclass/spawned) to improve peer discovery, sync reliability, and connection handling.                                   | In Progress  |
| Chain Syncing      | Verify the execution of all blocks across all chains. For Proof-of-Stake (PoS) chains (Holesky, Hoodi), verify all blocks since genesis. For chains with a pre-Merge genesis (Sepolia, Mainnet), verify all blocks after the Merge.  | In Progress |
| Snap Sync   | Improve Snap Sync implementation to make it more reliable and efficient. | Planned  |
| Client Stability | Increase client resilience to adverse scenarios and network disruptions. Improve observability and logging. | Planned |