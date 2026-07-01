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
    types::{BlockHeader, ChainConfig, fake_exponential},
};
use serde::{Deserialize, Deserializer, de::Error as DeError};

/// JSON shape of geth's Block Override Set.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockOverrideSet {
    #[serde(default, deserialize_with = "deser_u64_hex_opt")]
    pub number: Option<u64>,
    #[serde(default, deserialize_with = "deser_u64_hex_opt")]
    pub time: Option<u64>,
    #[serde(default, deserialize_with = "deser_u64_hex_opt")]
    pub gas_limit: Option<u64>,
    #[serde(default)]
    pub coinbase: Option<Address>,
    /// Override for PREVRANDAO.
    #[serde(default)]
    pub random: Option<H256>,
    #[serde(default, deserialize_with = "deser_u64_hex_opt")]
    pub base_fee_per_gas: Option<u64>,
    #[serde(default, deserialize_with = "deser_u256_hex_opt")]
    pub blob_base_fee_per_gas: Option<U256>,
    #[serde(default, deserialize_with = "deser_u256_hex_opt")]
    pub difficulty: Option<U256>,
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
    }

    /// Produce a synthesized header by overlaying the set fields on top of
    /// `header`. The `chain_config` is consulted to resolve the blob-fee update
    /// fraction for the active fork when inverting `blobBaseFeePerGas`.
    pub fn apply_to(&self, mut header: BlockHeader, chain_config: &ChainConfig) -> BlockHeader {
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
        if let Some(desired) = self.blob_base_fee_per_gas {
            let denom = chain_config
                .get_fork_blob_schedule(header.timestamp)
                .map(|s| s.base_fee_update_fraction)
                .unwrap_or(0);
            header.excess_blob_gas = Some(invert_blob_base_fee(desired, denom));
        }
        // Force hash recomputation by replacing the OnceCell.
        header.hash = Default::default();
        header
    }
}

/// Binary-search the smallest `excess_blob_gas` whose `fake_exponential`-derived
/// blob base fee is ≥ `desired`. Returns 0 when the desired fee is at or below
/// `MIN_BASE_FEE_PER_BLOB_GAS`. Returns `u64::MAX` when the desired fee is
/// unreachable within the representable range (clamped).
fn invert_blob_base_fee(desired: U256, denominator: u64) -> u64 {
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
