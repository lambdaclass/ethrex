# Supported EIPs

This document tracks which eips we support for each hard fork.

## Glamsterdam

| Number | Title | Description | Status | devnet-bal | Supported |
|--------|-------|-------------|--------|------------|-----------|
| [7928](https://eips.ethereum.org/EIPS/eip-7928) | Block-Level Access Lists | Record all accessed accounts and storage slots during block execution | SFI | [x] | [ ] |
| [7708](https://eips.ethereum.org/EIPS/eip-7708) | ETH Transfers Emit a Log | All ETH transfers emit Transfer/Selfdestruct logs automatically | CFI | [x] | [ ] |
| [7778](https://eips.ethereum.org/EIPS/eip-7778) | Block Gas Accounting without Refunds | Gas refunds no longer reduce block gas accounting | CFI | [x] | [ ] |
| [7843](https://eips.ethereum.org/EIPS/eip-7843) | SLOTNUM Opcode | New opcode (0x4b) returning beacon chain slot number | CFI | [x] | [ ] |
| [8024](https://eips.ethereum.org/EIPS/eip-8024) | Backward Compatible SWAPN, DUPN, EXCHANGE | New opcodes for deeper stack access (0xe6, 0xe7, 0xe8) | CFI | [x] | [ ] |
| [2780](https://eips.ethereum.org/EIPS/eip-2780) | Reduce Intrinsic Transaction Gas | Lower base transaction cost from 21,000 to 4,500 gas | CFI | [ ] | [ ] |
| [7904](https://eips.ethereum.org/EIPS/eip-7904) | General Repricing | Gas cost repricing to reflect computational complexity | CFI | [ ] | [ ] |
| [7954](https://eips.ethereum.org/EIPS/eip-7954) | Increase Maximum Contract Size | Raise contract size limit from 24KiB to 32KiB | CFI | [ ] | [ ] |
| [7976](https://eips.ethereum.org/EIPS/eip-7976) | Increase Calldata Floor Cost | Raise floor cost to 15/60 gas per zero/non-zero byte | CFI | [ ] | [ ] |
| [7981](https://eips.ethereum.org/EIPS/eip-7981) | Increase Access List Cost | Additional data cost for access list entries | CFI | [ ] | [ ] |
| [7997](https://eips.ethereum.org/EIPS/eip-7997) | Deterministic Factory Predeploy | System contract for deterministic CREATE2 deployments | CFI | [ ] | [ ] |
| [8037](https://eips.ethereum.org/EIPS/eip-8037) | State Creation Gas Cost Increase | Higher gas for state-creating operations | CFI | [ ] | [ ] |
| [8038](https://eips.ethereum.org/EIPS/eip-8038) | State-Access Gas Cost Update | Updated gas costs for SSTORE, SLOAD, and account access | CFI | [ ] | [ ] |
| [8070](https://eips.ethereum.org/EIPS/eip-8070) | Sparse Blobpool | Custody-aligned sampling to reduce blob bandwidth | CFI | [ ] | [ ] |
| [7610](https://eips.ethereum.org/EIPS/eip-7610) | Revert Creation in Case of Non-empty Storage | Prevent contract creation at addresses with existing storage | PFI | [ ] | [ ] |
| [7872](https://eips.ethereum.org/EIPS/eip-7872) | Max Blob Flag for Local Builders | Configurable maximum blobs per block for builders | PFI | [ ] | [ ] |

### Status Legend
- **SFI**: Scheduled for Inclusion
- **CFI**: Considered for Inclusion
- **PFI**: Proposed for Inclusion
