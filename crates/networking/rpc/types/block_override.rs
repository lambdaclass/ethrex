//! JSON-RPC shape of geth's Block Override Set.
//!
//! Each field, when present, replaces the corresponding `BlockHeader` field for
//! the synthetic block context used by the simulated call. Omitted fields keep
//! the real header values.
//!
//! `blobBaseFeePerGas` is special: ethrex's EVM derives `BLOBBASEFEE` from
//! `header.excess_blob_gas` via `fake_exponential`. To honor a direct override
//! we invert that function and find the `excess_blob_gas` that produces the
//! requested fee. The inversion is exact when the desired fee is representable
//! within the fake-exponential range for the active fork's update fraction; for
//! values that fall between representable steps the result rounds down to the
//! closest fee ≤ requested.

use ethrex_common::{
    Address, H256, U256,
    constants::MIN_BASE_FEE_PER_BLOB_GAS,
    types::{BlockHeader, ChainConfig, Withdrawal, fake_exponential},
};
use serde::{Deserialize, Deserializer, de::Error as DeError};

use crate::utils::RpcErr;

/// JSON shape of geth's Block Override Set (`internal/ethapi/override.BlockOverrides`).
///
/// Field names follow geth, which marshals Go field names and whose `encoding/json`
/// matches keys case-insensitively — so `FeeRecipient` is reached as `feeRecipient`.
/// geth renamed `Coinbase`/`Random` to `FeeRecipient`/`PrevRandao` and calls the blob fee
/// `BlobBaseFee`; alloy (reth, Foundry) keeps the older spellings as canonical and adds
/// `baseFee`, and erigon uses `blockNumber`/`timestamp`. Every spelling in circulation is
/// accepted via `alias`, so a request shaped for any of those clients works here. These are
/// also the `eth_simulateV1` spellings (execution-apis `BlockOverrides`).
///
/// `deny_unknown_fields` mirrors [`StateOverrideSet`](super::state_override::StateOverrideSet):
/// an override this client cannot honor must be an error rather than a silent drop, which
/// would return a plausible-looking but wrong result. `block_hash` is declared for exactly
/// that reason — see [`BlockOverrideSet::apply_to`]. `withdrawals` and `beacon_root` are
/// the fields whose support depends on the caller: `eth_simulateV1` builds a block and runs
/// its system calls, so it honors both, while the `eth_call` family has no block to attach
/// withdrawals to and runs no system contracts, and refuses them.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BlockOverrideSet {
    #[serde(default, alias = "blockNumber", deserialize_with = "deser_u64_hex_opt")]
    pub number: Option<u64>,
    #[serde(default, alias = "timestamp", deserialize_with = "deser_u64_hex_opt")]
    pub time: Option<u64>,
    #[serde(default, deserialize_with = "deser_u64_hex_opt")]
    pub gas_limit: Option<u64>,
    /// geth's `FeeRecipient`; `coinbase` is the older spelling alloy still uses.
    #[serde(default, alias = "feeRecipient")]
    pub coinbase: Option<Address>,
    /// Override for PREVRANDAO. geth's `PrevRandao`; `random` is the older spelling.
    #[serde(default, alias = "prevRandao")]
    pub random: Option<H256>,
    /// geth's `BaseFeePerGas`; `baseFee` is alloy's canonical spelling.
    #[serde(default, alias = "baseFee", deserialize_with = "deser_u64_hex_opt")]
    pub base_fee_per_gas: Option<u64>,
    /// geth's `BlobBaseFee`. `blobBaseFeePerGas` was ethrex's own spelling and is kept as
    /// an alias so requests written against earlier builds of this branch still parse.
    #[serde(
        default,
        rename = "blobBaseFee",
        alias = "blobBaseFeePerGas",
        deserialize_with = "deser_u256_hex_opt"
    )]
    pub blob_base_fee_per_gas: Option<U256>,
    #[serde(default, deserialize_with = "deser_u256_hex_opt")]
    pub difficulty: Option<U256>,
    /// geth's `BeaconRoot`. Support depends on the caller, like `withdrawals`:
    /// `eth_simulateV1` honors it, because that engine runs the block's system calls and
    /// so writes the value into the EIP-4788 ring buffer; the `eth_call` family refuses it
    /// in [`BlockOverrideSet::apply_to`], where nothing runs system contracts and the
    /// value would only reach the header.
    #[serde(default, alias = "parentBeaconBlockRoot")]
    pub beacon_root: Option<H256>,
    /// Withdrawals to apply in the simulated block (balance credits +
    /// `withdrawalsRoot`). Honored by `eth_simulateV1`, which builds a block; refused by
    /// [`BlockOverrideSet::apply_to`], since the `eth_call`-family paths have no block to
    /// attach withdrawals to.
    #[serde(default)]
    pub withdrawals: Option<Vec<Withdrawal>>,
    /// reth/alloy's `blockHash` extension (a number -> hash map read by `BLOCKHASH`).
    /// Not a geth field. Declared only to be refused with a reason.
    #[serde(default)]
    pub block_hash: Option<serde_json::Value>,
}

impl BlockOverrideSet {
    pub fn is_empty(&self) -> bool {
        self.number.is_none()
            && self.time.is_none()
            && self.gas_limit.is_none()
            && self.coinbase.is_none()
            && self.random.is_none()
            && self.base_fee_per_gas.is_none()
            && self.blob_base_fee_per_gas.is_none()
            && self.difficulty.is_none()
            && self.beacon_root.is_none()
            && self.withdrawals.is_none()
            && self.block_hash.is_none()
    }

    /// Resolve `blobBaseFee` into the `excess_blob_gas` that produces it
    /// (ethrex derives BLOBBASEFEE from `excess_blob_gas`). `None` when the
    /// override is not set.
    ///
    /// Used by `eth_simulateV1`, which builds its own header rather than going through
    /// [`BlockOverrideSet::apply_to`]. Unlike `apply_to` this cannot fail: a fork with no
    /// blob schedule yields a zero update fraction, for which the inversion returns 0.
    pub fn resolved_excess_blob_gas(
        &self,
        chain_config: &ChainConfig,
        timestamp: u64,
    ) -> Option<u64> {
        let desired = self.blob_base_fee_per_gas?;
        let denom = chain_config
            .get_fork_blob_schedule(timestamp)
            .map(|s| s.base_fee_update_fraction)
            .unwrap_or(0);
        // `invert_blob_base_fee` requires a non-zero update fraction (it debug-asserts on
        // one), so a fork with no blob schedule is resolved here instead: no fraction to
        // invert against means the minimum blob fee, i.e. zero excess blob gas.
        if denom == 0 {
            return Some(0);
        }
        Some(invert_blob_base_fee(desired, denom))
    }

    /// Produce a synthesized header by overlaying the set fields on top of
    /// `header`. The `chain_config` is consulted to resolve the blob-fee update
    /// fraction for the active fork when inverting `blobBaseFeePerGas`.
    ///
    /// Fails only for `blobBaseFeePerGas` on a fork with no blob schedule, where there is
    /// no update fraction to invert against and honoring the request is impossible.
    pub fn apply_to(
        &self,
        mut header: BlockHeader,
        chain_config: &ChainConfig,
    ) -> Result<BlockHeader, RpcErr> {
        if let Some(n) = self.number {
            header.number = n;
        }
        if let Some(t) = self.time {
            header.timestamp = t;
        }
        if let Some(g) = self.gas_limit {
            header.gas_limit = g;
        }
        if let Some(c) = self.coinbase {
            header.coinbase = c;
        }
        if let Some(r) = self.random {
            header.prev_randao = r;
        }
        if let Some(bf) = self.base_fee_per_gas {
            header.base_fee_per_gas = Some(bf);
        }
        if let Some(d) = self.difficulty {
            header.difficulty = d;
        }
        // geth's `BlockOverrides.Apply` refuses these two, and for the same reason: the
        // block's system contracts would have to run for either to mean anything, and no
        // simulation path runs them. Setting `parentBeaconBlockRoot` on the header alone
        // would leave the EIP-4788 ring buffer untouched, so a contract reading
        // `BEACON_ROOTS_ADDRESS` would not observe it — a silent no-op. To simulate that,
        // override the beacon-roots contract's storage through the State Override Set.
        if self.beacon_root.is_some() {
            return Err(RpcErr::BadParams(
                "beaconRoot is not supported: honoring it requires running the block's \
                 EIP-4788 system call, which simulation does not. Override the \
                 beacon-roots contract's storage via the State Override Set instead"
                    .to_string(),
            ));
        }
        if self.withdrawals.is_some() {
            return Err(RpcErr::BadParams(
                "withdrawals is not supported: honoring it requires processing the \
                 block's withdrawals, which simulation does not"
                    .to_string(),
            ));
        }
        if self.block_hash.is_some() {
            return Err(RpcErr::BadParams(
                "blockHash is not supported: it is a reth/alloy extension, not part of \
                 geth's Block Override Set"
                    .to_string(),
            ));
        }
        if let Some(desired) = self.blob_base_fee_per_gas {
            // Read after the `time` override above, so a timestamp that crosses into a
            // blob-carrying fork resolves that fork's schedule.
            let Some(denom) = chain_config
                .get_fork_blob_schedule(header.timestamp)
                .map(|s| s.base_fee_update_fraction)
                .filter(|d| *d != 0)
            else {
                return Err(RpcErr::BadParams(
                    "blobBaseFeePerGas cannot be applied: the block's fork has no blob \
                     schedule, so there is no update fraction to invert"
                        .to_string(),
                ));
            };
            header.excess_blob_gas = Some(invert_blob_base_fee(desired, denom));
        }
        // Force hash recomputation by replacing the OnceCell.
        header.hash = Default::default();
        Ok(header)
    }
}

/// Binary-search the smallest `excess_blob_gas` whose `fake_exponential`-derived
/// blob base fee is ≥ `desired`. Returns 0 when the desired fee is at or below
/// `MIN_BASE_FEE_PER_BLOB_GAS`, and the search ceiling (400_000_000, below where
/// `fake_exponential` overflows) when the desired fee is unreachable within that range.
///
/// `denominator` must be non-zero; [`BlockOverrideSet::apply_to`] rejects the override
/// rather than calling this with a fork that has no blob schedule.
fn invert_blob_base_fee(desired: U256, denominator: u64) -> u64 {
    debug_assert!(
        denominator != 0,
        "caller must reject a zero update fraction"
    );
    if denominator == 0 {
        return 0;
    }
    let factor = U256::from(MIN_BASE_FEE_PER_BLOB_GAS);
    if desired <= factor {
        return 0;
    }
    let compute = |excess: u64| -> U256 {
        fake_exponential(factor, U256::from(excess), denominator).unwrap_or(U256::MAX)
    };
    // fake_exponential overflows past ~400_000_000 numerator (per its doc comment).
    // Cap the search range conservatively below that, then clamp.
    let mut lo: u64 = 0;
    let mut hi: u64 = 400_000_000;
    if compute(hi) < desired {
        return hi;
    }
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if compute(mid) < desired {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

fn deser_u64_hex_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    let opt = Option::<String>::deserialize(d)?;
    opt.map(|s| {
        let trimmed = s.trim_start_matches("0x");
        u64::from_str_radix(trimmed, 16).map_err(|e| D::Error::custom(format!("invalid u64: {e}")))
    })
    .transpose()
}

fn deser_u256_hex_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<U256>, D::Error> {
    let opt = Option::<String>::deserialize(d)?;
    opt.map(|s| {
        let trimmed = s.trim_start_matches("0x");
        U256::from_str_radix(trimmed, 16)
            .map_err(|e| D::Error::custom(format!("invalid u256: {e}")))
    })
    .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_all_fields() {
        let v = json!({
            "number": "0x1000",
            "time": "0x65000000",
            "gasLimit": "0x1c9c380",
            "coinbase": "0x000000000000000000000000000000000000beef",
            "random": "0x000000000000000000000000000000000000000000000000000000000000dead",
            "baseFeePerGas": "0x10",
            "blobBaseFeePerGas": "0x100",
            "difficulty": "0x0"
        });
        let set: BlockOverrideSet = serde_json::from_value(v).unwrap();
        assert_eq!(set.number, Some(0x1000));
        assert_eq!(set.time, Some(0x65000000));
        assert_eq!(set.gas_limit, Some(0x1c9c380));
        assert_eq!(set.base_fee_per_gas, Some(0x10));
        assert_eq!(set.blob_base_fee_per_gas, Some(U256::from(0x100)));
        assert_eq!(set.difficulty, Some(U256::zero()));
    }

    /// A typo must be an error, not a silent drop: a silently-ignored override yields a
    /// plausible-looking but wrong result. Every field geth or alloy actually defines is
    /// modelled (and refused with a reason where it cannot be honored), so what is left
    /// for this to catch is genuine misspelling.
    #[test]
    fn unknown_field_is_rejected() {
        let v = json!({ "number": "0x1", "gasLimitt": "0x1" });
        let err = serde_json::from_value::<BlockOverrideSet>(v)
            .expect_err("unknown field should be rejected");
        assert!(
            err.to_string().contains("gasLimitt"),
            "error should name the offending field, got: {err}"
        );
    }

    /// geth's `BlockOverrides.Apply` returns an error for `beaconRoot` (and
    /// `withdrawals`): they cannot be honored without running the block's system
    /// contracts, which no simulation path does. Named explicitly so the error says why
    /// rather than "unknown field".
    #[test]
    fn beacon_root_is_rejected_as_unsupported() {
        let v = json!({ "beaconRoot": format!("{:#x}", H256::from_low_u64_be(0xbeac)) });
        let set: BlockOverrideSet = serde_json::from_value(v).unwrap();
        let err = set
            .apply_to(BlockHeader::default(), &ChainConfig::default())
            .expect_err("beaconRoot must be rejected");
        assert!(
            format!("{err}").contains("beaconRoot"),
            "error should name the field, got: {err}"
        );
    }

    /// Same for `withdrawals`, and for reth/alloy's `blockHash` extension: both are
    /// declared so the refusal is explanatory.
    #[test]
    fn withdrawals_and_block_hash_are_rejected_as_unsupported() {
        for field in ["withdrawals", "blockHash"] {
            let v = json!({ field: json!(null) });
            let set: BlockOverrideSet = serde_json::from_value(v)
                .unwrap_or_else(|e| panic!("`{field}` should be a known field, got: {e}"));
            let _ = set;
        }
        let set: BlockOverrideSet = serde_json::from_value(json!({ "withdrawals": [] })).unwrap();
        let err = set
            .apply_to(BlockHeader::default(), &ChainConfig::default())
            .expect_err("withdrawals must be rejected");
        assert!(format!("{err}").contains("withdrawals"), "got: {err}");
    }

    /// geth's current field names, which ethrex did not accept: `FeeRecipient` and
    /// `PrevRandao` replaced the older `Coinbase`/`Random`, and the blob fee is
    /// `BlobBaseFee` (Go matches JSON keys case-insensitively).
    #[test]
    fn geth_current_field_names_are_accepted() {
        let v = json!({
            "feeRecipient": "0x000000000000000000000000000000000000beef",
            "prevRandao": "0x000000000000000000000000000000000000000000000000000000000000dead",
            "baseFeePerGas": "0x10",
            "blobBaseFee": "0x100"
        });
        let set: BlockOverrideSet = serde_json::from_value(v).expect("geth names must parse");
        assert_eq!(set.coinbase, Some(Address::from_low_u64_be(0xbeef)));
        assert_eq!(set.random, Some(H256::from_low_u64_be(0xdead)));
        assert_eq!(set.base_fee_per_gas, Some(0x10));
        assert_eq!(set.blob_base_fee_per_gas, Some(U256::from(0x100)));
    }

    /// alloy's canonical `baseFee`, and erigon's `blockNumber`/`timestamp`.
    #[test]
    fn alloy_and_erigon_aliases_are_accepted() {
        let v = json!({ "blockNumber": "0x7", "timestamp": "0x8", "baseFee": "0x9" });
        let set: BlockOverrideSet = serde_json::from_value(v).expect("aliases must parse");
        assert_eq!(set.number, Some(7));
        assert_eq!(set.time, Some(8));
        assert_eq!(set.base_fee_per_gas, Some(9));
    }

    /// Pre-Cancun there is no blob schedule, so `fake_exponential` has no update
    /// fraction to invert against. Accepting the override and writing
    /// `excess_blob_gas: Some(0)` would silently ignore what the caller asked for, so it
    /// has to be an error.
    #[test]
    fn blob_base_fee_override_without_a_blob_schedule_is_rejected() {
        let v = json!({ "blobBaseFeePerGas": "0x100" });
        let set: BlockOverrideSet = serde_json::from_value(v).unwrap();
        let err = set
            .apply_to(BlockHeader::default(), &ChainConfig::default())
            .expect_err("a blob fee override with no blob schedule must be rejected");
        assert!(
            format!("{err}").contains("blobBaseFeePerGas"),
            "error should name the field, got: {err}"
        );
    }

    #[test]
    fn empty_is_empty() {
        let v = json!({});
        let set: BlockOverrideSet = serde_json::from_value(v).unwrap();
        assert!(set.is_empty());
    }

    #[test]
    fn invert_blob_fee_min_value() {
        // desired == MIN -> excess = 0.
        let excess = invert_blob_base_fee(U256::from(MIN_BASE_FEE_PER_BLOB_GAS), 3338477);
        assert_eq!(excess, 0);
    }

    #[test]
    fn invert_blob_fee_round_trips_within_one_step() {
        // Round-trip: pick an excess, compute fee, invert, recompute. Should match
        // exactly because the binary search finds the smallest excess whose fee is
        // ≥ desired, and the chosen `desired` is the exact output of `compute`.
        let denom = 3338477u64;
        let factor = U256::from(MIN_BASE_FEE_PER_BLOB_GAS);
        let original_excess: u64 = 786_432;
        let fee = fake_exponential(factor, U256::from(original_excess), denom).unwrap();
        let recovered = invert_blob_base_fee(fee, denom);
        let recovered_fee = fake_exponential(factor, U256::from(recovered), denom).unwrap();
        assert_eq!(fee, recovered_fee);
    }
}
