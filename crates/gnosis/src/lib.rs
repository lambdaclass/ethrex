//! Gnosis Chain support for ethrex.
//!
//! Gnosis Chain (formerly xDai, chain ID 100) and its Chiado testnet
//! (chain ID 10200) are EVM-compatible chains that use Ethereum's standard
//! post-Merge PoS architecture via the Engine API. They diverge from Ethereum
//! mainnet in three ways the execution client must handle:
//!
//! 1. **Post-block system calls** — every block calls the block-rewards
//!    contract (mints xDAI rewards) and the withdrawal contract (pays out GNO
//!    instead of natively crediting validator withdrawals).
//! 2. **Fee redirection** — EIP-1559 base fee and EIP-4844 blob base fee are
//!    sent to a fee-collector contract instead of being burned. Required to
//!    preserve the xDAI/Dai bridge invariant.
//! 3. **Smaller blob limits** — target=1, max=2 blobs per block, with a
//!    1 gwei minimum blob gas price.
//!
//! Consensus, P2P wire protocol, precompiles, and Engine API are all
//! identical to Ethereum mainnet.

pub mod genesis;
pub mod system_calls;
