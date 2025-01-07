use crate::eth::fee_calculator::estimate_gas_tip;

use crate::utils::RpcErr;
use crate::{RpcApiContext, RpcHandler};
use serde_json::Value;

// TODO: This does not need a struct,
// but I'm leaving it like this for consistency
// with the other RPC endpoints.
// The handle function could simply be
// a function called 'estimate'.
#[derive(Debug, Clone)]
pub struct MaxPriorityFee;

impl RpcHandler for MaxPriorityFee {
    fn parse(_: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(MaxPriorityFee {})
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let estimated_gas_tip = estimate_gas_tip(&context.storage)?;

        let gas_tip = match estimated_gas_tip {
            Some(gas_tip) => gas_tip,
            None => return Ok(serde_json::Value::Null),
        };

        let gas_as_hex = format!("0x{:x}", gas_tip);
        Ok(serde_json::Value::String(gas_as_hex))
    }
}

#[cfg(test)]
mod tests {}
