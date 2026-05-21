//! POST /engine/v{1,2}/payloads/bodies/by-hash and by-range.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use ethrex_common::H256;
use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_common::types::{Block, BlockBody, BlockHeader};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::Store;
use ethrex_storage::error::StoreError;
use libssz_types::SszList;

use crate::engine_rest::conversions::vec_withdrawals_to_ssz;
use crate::engine_rest::error::{EngineError, EngineRestError};
use crate::engine_rest::extractors::Ssz;
use crate::engine_rest::responses::SszBody;
use crate::engine_rest::types::bodies::{
    ExecutionPayloadBodyV1, ExecutionPayloadBodyV2, GetPayloadBodiesByHashV1Request,
    GetPayloadBodiesByHashV2Request, GetPayloadBodiesByRangeV1Request,
    GetPayloadBodiesByRangeV2Request, PayloadBodiesV1Response, PayloadBodiesV2Response,
};
use crate::engine_rest::types::common::{
    MAX_BYTES_PER_TRANSACTION, MAX_PAYLOAD_BODIES_REQUEST, ssz_none, ssz_some,
};
use crate::engine_rest::types::execution_payload::{Transactions, Withdrawals};
use crate::rpc::RpcApiContext;

fn encode_txs(txs: &[ethrex_common::types::Transaction]) -> Result<Transactions, EngineRestError> {
    let inner: Result<Vec<SszList<u8, MAX_BYTES_PER_TRANSACTION>>, _> = txs
        .iter()
        .map(|tx| tx.encode_canonical_to_vec().try_into())
        .collect();
    inner
        .map_err(|_| EngineRestError::internal("transaction exceeds MAX_BYTES_PER_TRANSACTION"))?
        .try_into()
        .map_err(|_| EngineRestError::internal("tx count exceeds MAX_TRANSACTIONS_PER_PAYLOAD"))
}

fn encode_withdrawals(
    ws: &Option<Vec<ethrex_common::types::Withdrawal>>,
) -> Result<Withdrawals, EngineRestError> {
    vec_withdrawals_to_ssz(ws.as_deref().unwrap_or(&[]))
}

fn body_to_v1(body: &BlockBody) -> Result<ExecutionPayloadBodyV1, EngineRestError> {
    Ok(ExecutionPayloadBodyV1 {
        transactions: encode_txs(&body.transactions)?,
        withdrawals: encode_withdrawals(&body.withdrawals)?,
    })
}

fn body_to_v2(
    body: &BlockBody,
    bal: Option<&BlockAccessList>,
) -> Result<ExecutionPayloadBodyV2, EngineRestError> {
    let block_access_list = match bal {
        Some(b) => {
            let mut buf = Vec::new();
            b.encode(&mut buf);
            let inner: SszList<u8, MAX_BYTES_PER_TRANSACTION> = buf
                .try_into()
                .map_err(|_| EngineRestError::internal("BAL exceeds MAX_BYTES_PER_TRANSACTION"))?;
            ssz_some(inner)
        }
        None => ssz_none(),
    };
    Ok(ExecutionPayloadBodyV2 {
        transactions: encode_txs(&body.transactions)?,
        withdrawals: encode_withdrawals(&body.withdrawals)?,
        block_access_list,
    })
}

fn check_count(n: usize) -> Result<(), EngineRestError> {
    if n > MAX_PAYLOAD_BODIES_REQUEST {
        return Err(EngineRestError::payload_too_large(format!(
            "request exceeds MAX_PAYLOAD_BODIES_REQUEST ({MAX_PAYLOAD_BODIES_REQUEST})"
        )));
    }
    Ok(())
}

pub async fn bodies_by_hash_v1(
    State(ctx): State<RpcApiContext>,
    Ssz(req): Ssz<GetPayloadBodiesByHashV1Request>,
) -> Response {
    if let Err(e) = check_count(req.block_hashes.len()) {
        return e.into();
    }
    let lookups = req
        .block_hashes
        .iter()
        .map(|h| ctx.storage.get_block_body_by_hash(H256::from(*h)));
    let bodies = match futures::future::try_join_all(lookups).await {
        Ok(b) => b,
        Err(e) => return EngineError::internal(&format!("storage: {e}")),
    };
    bodies_v1_response(bodies)
}

pub async fn bodies_by_range_v1(
    State(ctx): State<RpcApiContext>,
    Ssz(req): Ssz<GetPayloadBodiesByRangeV1Request>,
) -> Response {
    if req.start == 0 {
        return EngineError::bad_request("start must be ≥ 1");
    }
    if req.count == 0 {
        return EngineError::bad_request("count must be ≥ 1");
    }
    if req.count as usize > MAX_PAYLOAD_BODIES_REQUEST {
        return EngineError::payload_too_large(&format!(
            "count exceeds MAX_PAYLOAD_BODIES_REQUEST ({MAX_PAYLOAD_BODIES_REQUEST})"
        ));
    }
    let latest = match ctx.storage.get_latest_block_number().await {
        Ok(n) => n,
        Err(e) => return EngineError::internal(&format!("storage: {e}")),
    };
    if req.start > latest {
        return bodies_v1_response(Vec::new());
    }
    let last = req.start.saturating_add(req.count - 1).min(latest);
    let bodies = match ctx.storage.get_block_bodies(req.start, last).await {
        Ok(b) => b,
        Err(e) => return EngineError::internal(&format!("storage: {e}")),
    };
    bodies_v1_response(bodies)
}

fn bodies_v1_response(bodies: Vec<Option<BlockBody>>) -> Response {
    let mut payload_bodies: Vec<SszList<ExecutionPayloadBodyV1, 1>> =
        Vec::with_capacity(bodies.len());
    for body in bodies {
        let slot = match body {
            Some(b) => match body_to_v1(&b) {
                Ok(v) => ssz_some(v),
                Err(e) => return e.into(),
            },
            None => ssz_none(),
        };
        payload_bodies.push(slot);
    }
    let resp = match payload_bodies.try_into() {
        Ok(s) => s,
        Err(_) => return EngineError::internal("payload_bodies overflow"),
    };
    SszBody(PayloadBodiesV1Response {
        payload_bodies: resp,
    })
    .into_response()
}

// Pair each body with its (sync) header lookup and EVM-bound BAL generation
// inside a single `spawn_blocking` so the runtime isn't blocked.
// `generate_bal_for_block` re-executes the block in the EVM.
async fn assemble_blocks_with_bal<K>(
    ctx: &RpcApiContext,
    bodies: Vec<Option<BlockBody>>,
    keys: Vec<K>,
    fetch_header: fn(&Store, &K) -> Result<Option<BlockHeader>, StoreError>,
) -> Result<Vec<Option<(Block, Option<BlockAccessList>)>>, Response>
where
    K: Send + 'static,
{
    let storage = ctx.storage.clone();
    let blockchain = ctx.blockchain.clone();
    let result = tokio::task::spawn_blocking(move || {
        bodies
            .into_iter()
            .zip(keys)
            .map(|(body_opt, key)| {
                let Some(body) = body_opt else {
                    return Ok(None);
                };
                let Some(header) = fetch_header(&storage, &key)? else {
                    return Ok(None);
                };
                let block = Block { header, body };
                let bal = blockchain
                    .generate_bal_for_block(&block)
                    .map_err(|e| StoreError::Custom(e.to_string()))?;
                Ok(Some((block, bal)))
            })
            .collect::<Result<Vec<_>, StoreError>>()
    })
    .await;
    match result {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(EngineError::internal(&format!("storage: {e}"))),
        Err(e) => Err(EngineError::internal(&format!("BAL fetch panicked: {e}"))),
    }
}

pub async fn bodies_by_hash_v2(
    State(ctx): State<RpcApiContext>,
    Ssz(req): Ssz<GetPayloadBodiesByHashV2Request>,
) -> Response {
    if let Err(e) = check_count(req.block_hashes.len()) {
        return e.into();
    }
    let hashes: Vec<H256> = req.block_hashes.iter().map(|h| H256::from(*h)).collect();
    let lookups = hashes
        .iter()
        .map(|h| ctx.storage.get_block_body_by_hash(*h));
    let bodies = match futures::future::try_join_all(lookups).await {
        Ok(b) => b,
        Err(e) => return EngineError::internal(&format!("storage: {e}")),
    };
    let blocks_with_bal =
        match assemble_blocks_with_bal(&ctx, bodies, hashes, |s, h| s.get_block_header_by_hash(*h))
            .await
        {
            Ok(v) => v,
            Err(resp) => return resp,
        };
    bodies_v2_response(blocks_with_bal)
}

pub async fn bodies_by_range_v2(
    State(ctx): State<RpcApiContext>,
    Ssz(req): Ssz<GetPayloadBodiesByRangeV2Request>,
) -> Response {
    if req.start == 0 {
        return EngineError::bad_request("start must be ≥ 1");
    }
    if req.count == 0 {
        return EngineError::bad_request("count must be ≥ 1");
    }
    if req.count as usize > MAX_PAYLOAD_BODIES_REQUEST {
        return EngineError::payload_too_large(&format!(
            "count exceeds MAX_PAYLOAD_BODIES_REQUEST ({MAX_PAYLOAD_BODIES_REQUEST})"
        ));
    }
    let latest = match ctx.storage.get_latest_block_number().await {
        Ok(n) => n,
        Err(e) => return EngineError::internal(&format!("storage: {e}")),
    };
    if req.start > latest {
        return bodies_v2_response(Vec::new());
    }
    let last = req.start.saturating_add(req.count - 1).min(latest);
    let block_bodies = match ctx.storage.get_block_bodies(req.start, last).await {
        Ok(b) => b,
        Err(e) => return EngineError::internal(&format!("storage: {e}")),
    };
    let block_numbers: Vec<u64> = (req.start..=last).collect();
    let blocks_with_bal =
        match assemble_blocks_with_bal(&ctx, block_bodies, block_numbers, |s, n| {
            s.get_block_header(*n)
        })
        .await
        {
            Ok(v) => v,
            Err(resp) => return resp,
        };
    bodies_v2_response(blocks_with_bal)
}

fn bodies_v2_response(blocks_with_bal: Vec<Option<(Block, Option<BlockAccessList>)>>) -> Response {
    let mut payload_bodies: Vec<SszList<ExecutionPayloadBodyV2, 1>> =
        Vec::with_capacity(blocks_with_bal.len());
    for entry in blocks_with_bal {
        let slot = match entry {
            Some((block, bal)) => match body_to_v2(&block.body, bal.as_ref()) {
                Ok(v) => ssz_some(v),
                Err(e) => return e.into(),
            },
            None => ssz_none(),
        };
        payload_bodies.push(slot);
    }
    let resp = match payload_bodies.try_into() {
        Ok(s) => s,
        Err(_) => return EngineError::internal("payload_bodies overflow"),
    };
    SszBody(PayloadBodiesV2Response {
        payload_bodies: resp,
    })
    .into_response()
}
