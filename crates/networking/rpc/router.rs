use crate::context::RpcApiContext;
use crate::errors::RpcErr;
use crate::rpc_types::{RpcNamespace, RpcRequest};
use serde_json::Value;

pub trait RpcHandler: Sized {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, crate::errors::RpcErr>;

    fn call(req: &RpcRequest, context: RpcApiContext) -> Result<Value, crate::errors::RpcErr> {
        let request = Self::parse(&req.params)?;
        request.handle(context)
    }

    /// Relay the request to the gateway client, if the request fails, fallback to the local node
    /// The default implementation of this method is to call `RpcHandler::call` method because
    /// not all requests need to be relayed to the gateway client, and the only ones that have to
    /// must override this method.
    #[cfg(feature = "based")]
    async fn relay_to_gateway_or_fallback(
        req: &RpcRequest,
        context: RpcApiContext,
    ) -> Result<Value, crate::errors::RpcErr> {
        Self::call(req, context)
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, crate::errors::RpcErr>;
}

/// Handle requests that can come from either clients or other users
pub async fn map_http_requests(req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    match req.namespace() {
        Ok(RpcNamespace::Eth) => crate::eth::map_eth_requests(req, context).await,
        Ok(RpcNamespace::Admin) => crate::admin::map_admin_requests(req, context),
        Ok(RpcNamespace::Debug) => crate::eth::map_debug_requests(req, context).await,
        Ok(RpcNamespace::Web3) => crate::web3::map_web3_requests(req, context),
        Ok(RpcNamespace::Net) => crate::net::map_net_requests(req, context),
        #[cfg(feature = "l2")]
        Ok(RpcNamespace::EthrexL2) => crate::l2::map_l2_requests(req, context),
        _ => Err(RpcErr::MethodNotFound(req.method.clone())),
    }
}

/// Handle requests from consensus client
pub async fn map_authrpc_requests(
    req: &RpcRequest,
    context: RpcApiContext,
) -> Result<Value, RpcErr> {
    match req.namespace() {
        Ok(RpcNamespace::Engine) => crate::engine::map_engine_requests(req, context).await,
        Ok(RpcNamespace::Eth) => crate::eth::map_eth_requests(req, context).await,
        _ => Err(RpcErr::MethodNotFound(req.method.clone())),
    }
}
