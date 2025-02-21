use crate::RpcHandler;

pub struct EnvV0;

impl RpcHandler for EnvV0 {
    fn parse(_params: &Option<Vec<serde_json::Value>>) -> Result<Self, crate::utils::RpcErr> {
        tracing::info!("parsing env");
        Ok(Self)
    }

    fn handle(
        &self,
        _context: crate::RpcApiContext,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
        tracing::info!("handling env");
        Ok(serde_json::Value::Null)
    }
}
