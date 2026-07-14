//! XEN-specialized analytic read-set predictor (opt-in, mainnet-only, pre-BAL).
//!
//! XEN "Torrent" batch-mint trains are ethrex's recurring worst mainnet blocks:
//! a single sender sends several `bulkClaimMintReward` txs, each fanning out to
//! hundreds of CREATE2 minimal-proxy mint claims whose cold long-tail storage
//! reads dominate block-exec time. A general per-sender/per-tx prewarmer cannot
//! parallelize the single-sender train, but the proxy addresses are
//! ANALYTICALLY derivable from calldata alone:
//!   salt  = keccak256(abi.encodePacked(uint256 i, uint256 tokenId))
//!   proxy = keccak256(0xff ++ XEN_TORRENT ++ salt ++ PROXY_INIT_CODE_HASH)[12:]
//! so we can synthesize the read-set up front and prefetch it in parallel (the
//! proven queue-depth lever) with NO EVM execution — the one specialized win a
//! general prewarmer cannot replicate.
//!
//! Ground-truth validated against Xatu (2026-06-30): on XEN-Torrent-dominated
//! blocks this read-set covers ~99.6% of the bulkClaimMintReward-attributable
//! reads, 0 false predictions. `userMints` is a 6-slot struct, so 7 storage
//! slots per proxy (1 balance + 6 userMints) plus the proxy account.
//!
//! Quarantined here so it is trivial to retire once EIP-7928 (Amsterdam) ships
//! the read-set via the protocol BAL. Default-off; enable with
//! `ETHREX_XEN_PREFETCH=1`. Only fires pre-BAL on mainnet.

use ethrex_common::types::{Block, TxKind};
use ethrex_common::utils::keccak;
use ethrex_common::{Address, BigEndianHash, H256, U256};
use ethrex_levm::db::Database;

/// XEN Torrent (XENFT) factory/router — the CREATE2 deployer of the mint proxies.
const XEN_TORRENT: [u8; 20] = [
    0x0a, 0x25, 0x26, 0x63, 0xdb, 0xcc, 0x0b, 0x07, 0x30, 0x63, 0xd6, 0x42, 0x0a, 0x40, 0x31, 0x9e,
    0x43, 0x8c, 0xfa, 0x59,
];
/// XEN Crypto ERC-20 token — holds the cold per-proxy mint/balance storage.
const XEN_TOKEN: [u8; 20] = [
    0x06, 0x45, 0x0d, 0xee, 0x7f, 0xd2, 0xfb, 0x8e, 0x39, 0x06, 0x14, 0x34, 0xba, 0xbc, 0xfc, 0x05,
    0x59, 0x9a, 0x6f, 0xb8,
];
/// keccak256 of the EIP-1167 minimal-proxy init code that XEN Torrent CREATE2-deploys.
const PROXY_INIT_CODE_HASH: [u8; 32] = [
    0x0d, 0x44, 0x25, 0x7b, 0xf9, 0x09, 0x94, 0x8d, 0x7a, 0xfc, 0xb7, 0x4f, 0x09, 0x6c, 0x08, 0xe9,
    0xf4, 0x4d, 0xc7, 0x77, 0x0b, 0xaa, 0xce, 0xe0, 0xd4, 0x8c, 0x6e, 0x9c, 0x8f, 0x61, 0x01, 0x79,
];
/// `bulkClaimMintReward(uint256 tokenId, address to)` selector.
const SEL_BULK_CLAIM_MINT_REWARD: [u8; 4] = [0xf5, 0x87, 0x8b, 0x9b];

// XENFT / XEN storage layout (verified against on-chain reads):
const SLOT_VMU_COUNT: u64 = 11; // XENFT: mapping(tokenId => uint256) vmuCount
const SLOT_USER_MINTS: u64 = 9; // XEN:   mapping(address => MintInfo) userMints (6-slot struct)
const SLOT_BALANCES: u64 = 0; // XEN:   mapping(address => uint256) _balances
const USER_MINTS_STRUCT_SLOTS: u64 = 6; // MintInfo occupies 6 contiguous slots

/// Defensive cap on per-token proxy fan-out (real XENFT max VMUs per token is small).
const MAX_VMUS_PER_TOKEN: u64 = 10_000;

const MAINNET_CHAIN_ID: u64 = 1;

/// Read once: enable the XEN-specialized prefetch via `ETHREX_XEN_PREFETCH=1`.
fn enabled() -> bool {
    static EN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *EN.get_or_init(|| {
        matches!(
            std::env::var("ETHREX_XEN_PREFETCH").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE")
        )
    })
}

impl super::LEVM {
    /// Analytically predict the cold read-set of XEN Torrent `bulkClaimMintReward`
    /// txs in `block`, for up-front parallel prefetch. Returns `(proxy_accounts,
    /// storage_slots)` in the `prefetch_accounts`/`prefetch_storage` shape.
    ///
    /// Returns empty (a cheap no-op) when disabled, off mainnet, or when the block
    /// has no XEN Torrent mint-claim txs — i.e. on the overwhelming majority of
    /// blocks. Reads exactly one storage slot per distinct tokenId (`vmuCount`),
    /// which also warms the shared cache so execution never re-pays it.
    pub fn xen_predicted_read_set(
        block: &Block,
        store: &dyn Database,
    ) -> (Vec<Address>, Vec<(Address, H256)>) {
        let empty = (Vec::new(), Vec::new());
        if !enabled() {
            return empty;
        }
        // Mainnet-only: the hardcoded addresses exist only on mainnet. Cheap guard
        // that also avoids reading vmuCount on the wrong chain.
        if store.get_chain_config().map(|c| c.chain_id) != Ok(MAINNET_CHAIN_ID) {
            return empty;
        }

        let torrent = Address::from(XEN_TORRENT);
        let token = Address::from(XEN_TOKEN);
        let init_code_hash = H256(PROXY_INIT_CODE_HASH);

        // Distinct tokenIds claimed in this block (a tokenId may recur across txs).
        let mut token_ids: Vec<U256> = Vec::new();
        for tx in block.body.transactions.iter() {
            if !matches!(tx.to(), TxKind::Call(to) if to == torrent) {
                continue;
            }
            let data = tx.data();
            if data.len() < 36 || data.get(0..4) != Some(&SEL_BULK_CLAIM_MINT_REWARD[..]) {
                continue;
            }
            let token_id = U256::from_big_endian(&data[4..36]);
            if !token_ids.contains(&token_id) {
                token_ids.push(token_id);
            }
        }
        if token_ids.is_empty() {
            return empty;
        }

        let mut accounts: Vec<Address> = Vec::new();
        let mut slots: Vec<(Address, H256)> = Vec::new();
        for token_id in token_ids {
            // The one pre-block read: vmuCount[tokenId] gives the proxy fan-out bound.
            let vmu_slot = uint_mapping_slot(token_id, SLOT_VMU_COUNT);
            let n = match store.get_storage_value(torrent, vmu_slot) {
                Ok(v) => v.min(U256::from(MAX_VMUS_PER_TOKEN)).as_u64(),
                Err(_) => continue,
            };
            for i in 1..=n {
                let proxy = create2(torrent, proxy_salt(i, token_id), init_code_hash);
                accounts.push(proxy);
                // 1 balance slot + the 6-slot userMints struct, all on the XEN token.
                slots.push((token, address_mapping_slot(proxy, SLOT_BALANCES)));
                let user_mints_base =
                    U256::from_big_endian(address_mapping_slot(proxy, SLOT_USER_MINTS).as_bytes());
                for off in 0..USER_MINTS_STRUCT_SLOTS {
                    slots.push((token, H256::from_uint(&(user_mints_base + off))));
                }
            }
        }
        (accounts, slots)
    }
}

/// CREATE2 address from a precomputed init-code hash:
/// `keccak256(0xff ++ deployer ++ salt ++ init_code_hash)[12:]`.
fn create2(deployer: Address, salt: H256, init_code_hash: H256) -> Address {
    let mut buf = [0u8; 1 + 20 + 32 + 32];
    buf[0] = 0xff;
    buf[1..21].copy_from_slice(deployer.as_bytes());
    buf[21..53].copy_from_slice(salt.as_bytes());
    buf[53..85].copy_from_slice(init_code_hash.as_bytes());
    Address::from_slice(&keccak(buf).as_bytes()[12..])
}

/// XEN Torrent proxy salt = `keccak256(abi.encodePacked(uint256 i, uint256 tokenId))`.
fn proxy_salt(i: u64, token_id: U256) -> H256 {
    let mut buf = [0u8; 64];
    buf[0..32].copy_from_slice(&U256::from(i).to_big_endian());
    buf[32..64].copy_from_slice(&token_id.to_big_endian());
    keccak(buf)
}

/// Storage slot of `mapping(address => _) at base` = `keccak256(pad32(addr) ++ pad32(base))`.
fn address_mapping_slot(key: Address, base: u64) -> H256 {
    let mut buf = [0u8; 64];
    buf[12..32].copy_from_slice(key.as_bytes());
    buf[56..64].copy_from_slice(&base.to_be_bytes());
    keccak(buf)
}

/// Storage slot of `mapping(uint256 => _) at base` = `keccak256(pad32(key) ++ pad32(base))`.
fn uint_mapping_slot(key: U256, base: u64) -> H256 {
    let mut buf = [0u8; 64];
    buf[0..32].copy_from_slice(&key.to_big_endian());
    buf[56..64].copy_from_slice(&base.to_be_bytes());
    keccak(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Known-good XEN Torrent proxies for tokenId 76253 (verified on-chain).
    #[test]
    fn create2_matches_known_proxies() {
        let torrent = Address::from(XEN_TORRENT);
        let ich = H256(PROXY_INIT_CODE_HASH);
        let token_id = U256::from(76253u64);
        let expected = [
            "0xe47b0dadd68db4472600444c161950878c6a1e22",
            "0xda5e652b9f1449f8e43e166e037575a5f7d5ce1a",
            "0xc292bf8998ee560af27549ecd0db7fb83e87c14c",
        ];
        for (idx, exp) in expected.iter().enumerate() {
            let i = (idx + 1) as u64;
            let got = create2(torrent, proxy_salt(i, token_id), ich);
            assert_eq!(format!("{got:#x}"), *exp, "proxy i={i}");
        }
    }
}
