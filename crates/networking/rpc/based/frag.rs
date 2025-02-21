use crate::RpcHandler;

pub struct FragV0;

impl RpcHandler for FragV0 {
    fn parse(_params: &Option<Vec<serde_json::Value>>) -> Result<Self, crate::utils::RpcErr> {
        tracing::info!("parsing frag");
        Ok(Self)
    }

    fn handle(
        &self,
        _context: crate::RpcApiContext,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
        tracing::info!("handling frag");
        Ok(serde_json::Value::Null)
    }
}
