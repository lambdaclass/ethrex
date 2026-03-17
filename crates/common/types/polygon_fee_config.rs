use ethereum_types::Address;

/// Fee configuration for Polygon PoS block execution.
///
/// Polygon distributes fees differently from Ethereum L1:
/// - Base fee revenue goes to a "burnt contract" address (not actually burned)
/// - Tips (priority fees) go to a BorConfig-specified coinbase (not header.coinbase)
/// - Both fee components are paid per-tx during execution (not deferred)
#[derive(Debug, Clone, Copy, Default)]
pub struct PolygonFeeConfig {
    /// Address to receive base fee revenue (base_fee * gas_used).
    /// Comes from BorConfig.burnt_contract, block-number-indexed.
    /// None if no burnt contract is configured for this block.
    pub burnt_contract: Option<Address>,
    /// Address to receive tip revenue (priority_fee * gas_used).
    /// Comes from BorConfig.coinbase, block-number-indexed.
    /// Pre-Rio: zero address (tips effectively burned).
    /// Post-Rio: governance address.
    pub coinbase: Address,
}
