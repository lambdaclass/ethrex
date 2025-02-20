use crate::RpcHandler;

pub struct SealV0;

impl RpcHandler for SealV0 {
    fn parse(_params: &Option<Vec<serde_json::Value>>) -> Result<Self, crate::utils::RpcErr> {
        tracing::info!("parsing seal");
        Ok(Self)
    }

    fn handle(
        &self,
        _context: crate::RpcApiContext,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
        tracing::info!("handling seal");
        Ok(serde_json::Value::Null)
    }
}
