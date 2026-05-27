use ethrex_common::H256;
use ethrex_storage::Store;
use ethrex_storage::error::StoreError;
use serde_json::Value;

use crate::{RpcApiContext, RpcErr, RpcHandler, types::block_identifier::BlockIdentifier};

/// `debug_getModifiedAccountsByNumber` / `debug_getModifiedAccountsByHash` —
/// diff two state tries and return the set of accounts whose value differs.
///
/// **Divergence from geth**: geth returns `Vec<Address>` by looking original
/// addresses up through its preimage store. ethrex has no preimage store, so
/// the response is `Vec<H256>` of *hashed* addresses (`keccak256(address)`).
/// Callers that know the address they care about can hash it themselves to
/// check membership; the existing `debug_dumpBlock` endpoint uses the same
/// hashed-key convention.
///
/// Both handlers traverse the state tries in lockstep (the iterator yields
/// entries in hashed-key order), so memory is O(1) regardless of state size.
/// The traversal is wrapped in `tokio::task::spawn_blocking` because the
/// underlying `iter_accounts_from` opens long-lived locked DB transactions and
/// performs synchronous trie I/O.
pub struct GetModifiedAccountsByNumberRequest {
    start_block: BlockIdentifier,
    end_block: BlockIdentifier,
}

pub struct GetModifiedAccountsByHashRequest {
    start_hash: H256,
    end_hash: H256,
}

impl RpcHandler for GetModifiedAccountsByNumberRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams(format!(
                "Expected 2 params, got {}",
                params.len()
            )));
        }
        Ok(GetModifiedAccountsByNumberRequest {
            start_block: BlockIdentifier::parse(params[0].clone(), 0)?,
            end_block: BlockIdentifier::parse(params[1].clone(), 1)?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let start_number = self
            .start_block
            .resolve_block_number(&context.storage)
            .await?
            .ok_or(RpcErr::WrongParam("Start block not found".to_string()))?;
        let end_number = self
            .end_block
            .resolve_block_number(&context.storage)
            .await?
            .ok_or(RpcErr::WrongParam("End block not found".to_string()))?;

        if start_number > end_number {
            return Err(RpcErr::BadParams(format!(
                "start block ({start_number}) must be older than end block ({end_number})"
            )));
        }

        let start_root = context
            .storage
            .get_block_header(start_number)?
            .ok_or(RpcErr::WrongParam("Start block header not found".to_string()))?
            .state_root;
        let end_root = context
            .storage
            .get_block_header(end_number)?
            .ok_or(RpcErr::WrongParam("End block header not found".to_string()))?
            .state_root;

        diff_state_roots_async(context.storage.clone(), start_root, end_root).await
    }
}

impl RpcHandler for GetModifiedAccountsByHashRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams(format!(
                "Expected 2 params, got {}",
                params.len()
            )));
        }
        Ok(GetModifiedAccountsByHashRequest {
            start_hash: serde_json::from_value(params[0].clone())?,
            end_hash: serde_json::from_value(params[1].clone())?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let start_header = context
            .storage
            .get_block_header_by_hash(self.start_hash)?
            .ok_or(RpcErr::WrongParam("Start block not found".to_string()))?;
        let end_header = context
            .storage
            .get_block_header_by_hash(self.end_hash)?
            .ok_or(RpcErr::WrongParam("End block not found".to_string()))?;

        if start_header.number > end_header.number {
            return Err(RpcErr::BadParams(format!(
                "start block ({}) must be older than end block ({})",
                start_header.number, end_header.number
            )));
        }

        diff_state_roots_async(
            context.storage.clone(),
            start_header.state_root,
            end_header.state_root,
        )
        .await
    }
}

async fn diff_state_roots_async(
    storage: Store,
    start_root: H256,
    end_root: H256,
) -> Result<Value, RpcErr> {
    let hashes =
        tokio::task::spawn_blocking(move || diff_state_roots(&storage, start_root, end_root))
            .await
            .map_err(|e| RpcErr::Internal(format!("modified-accounts task failed: {e}")))??;
    Ok(serde_json::to_value(hashes)?)
}

/// Streams both state tries in lockstep (entries arrive in hashed-key order)
/// and emits the set of hashed addresses where the two states differ. O(1)
/// extra memory regardless of state size.
fn diff_state_roots(
    storage: &Store,
    start_root: H256,
    end_root: H256,
) -> Result<Vec<H256>, StoreError> {
    if start_root == end_root {
        return Ok(vec![]);
    }

    let mut start_iter = storage.iter_accounts(start_root)?.peekable();
    let mut end_iter = storage.iter_accounts(end_root)?.peekable();
    let mut modified = Vec::new();

    loop {
        // Copy peeked keys/values upfront so borrows on the Peekable
        // wrappers are released before the mutable next() calls below.
        let s_entry = start_iter.peek().map(|(h, s)| (*h, s.clone()));
        let e_entry = end_iter.peek().map(|(h, s)| (*h, s.clone()));

        match (s_entry, e_entry) {
            (None, None) => break,
            (Some((s_hash, _)), None) => {
                modified.push(s_hash);
                start_iter.next();
            }
            (None, Some((e_hash, _))) => {
                modified.push(e_hash);
                end_iter.next();
            }
            (Some((s_hash, _)), Some((e_hash, _))) if s_hash < e_hash => {
                // Account in start but not yet (and never, since iters are
                // monotonic) seen in end at this position — deleted.
                modified.push(s_hash);
                start_iter.next();
            }
            (Some((s_hash, _)), Some((e_hash, _))) if s_hash > e_hash => {
                // New account in end.
                modified.push(e_hash);
                end_iter.next();
            }
            (Some((s_hash, s_state)), Some((_e_hash, e_state))) => {
                // Same hashed address on both sides — emit if anything changed.
                if s_state != e_state {
                    modified.push(s_hash);
                }
                start_iter.next();
                end_iter.next();
            }
        }
    }

    Ok(modified)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RpcHandler;
    use serde_json::json;

    // --- GetModifiedAccountsByNumberRequest parse tests ---

    #[test]
    fn parse_by_number_valid() {
        let params = Some(vec![json!("0x1"), json!("0xa")]);
        let req = GetModifiedAccountsByNumberRequest::parse(&params).unwrap();
        assert!(matches!(req.start_block, BlockIdentifier::Number(1)));
        assert!(matches!(req.end_block, BlockIdentifier::Number(10)));
    }

    #[test]
    fn parse_by_number_no_params() {
        assert!(GetModifiedAccountsByNumberRequest::parse(&None).is_err());
    }

    #[test]
    fn parse_by_number_wrong_count() {
        let params = Some(vec![json!("0x1")]);
        assert!(GetModifiedAccountsByNumberRequest::parse(&params).is_err());
    }

    // --- GetModifiedAccountsByHashRequest parse tests ---

    #[test]
    fn parse_by_hash_valid() {
        let params = Some(vec![
            json!("0x0000000000000000000000000000000000000000000000000000000000000001"),
            json!("0x0000000000000000000000000000000000000000000000000000000000000002"),
        ]);
        let req = GetModifiedAccountsByHashRequest::parse(&params).unwrap();
        assert_eq!(req.start_hash, H256::from_low_u64_be(1));
        assert_eq!(req.end_hash, H256::from_low_u64_be(2));
    }

    #[test]
    fn parse_by_hash_no_params() {
        assert!(GetModifiedAccountsByHashRequest::parse(&None).is_err());
    }

    #[test]
    fn parse_by_hash_wrong_count() {
        let params = Some(vec![json!("0x01")]);
        assert!(GetModifiedAccountsByHashRequest::parse(&params).is_err());
    }
}
