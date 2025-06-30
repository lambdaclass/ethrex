use std::collections::HashMap;

use ethereum_types::Address;
use ethrex_rlp::decode::RLPDecode;
use ethrex_trie::{EMPTY_TRIE_HASH, Node, Trie};
use keccak_hash::H256;

use crate::types::{block_execution_witness::ExecutionWitnessError, AccountState};

pub fn rebuild_trie(initial_state: H256, state: Vec<Vec<u8>>) -> Result<Trie, ExecutionWitnessError>{
    let mut initial_node = None;
    for node in state.iter() {
        let x = Node::decode_raw(node).map_err(|_| {
            ExecutionWitnessError::RebuildTrie("Invalid state trie node in witness".to_string())
        })?;
        let hash = x.compute_hash().finalize();
        if hash == initial_state {
            initial_node = Some(node.clone());
            break;
        }
    }

    Trie::from_nodes(initial_node.as_ref(), &state).map_err(|e| {
            ExecutionWitnessError::RebuildTrie(format!("Failed to build state trie {e}"))
        })
}

// This funciton is an option because we expect it to fail sometimes, and we just want to filter it
pub fn rebuild_storage_trie(address: &Vec<u8>, trie: &Trie, state: Vec<Vec<u8>>) -> Option<Trie>{
        let account_state_rlp = trie.get(address)
            .ok()??;
        let account_state = AccountState::decode(&account_state_rlp)
            .ok()?;
        if account_state.storage_root == *EMPTY_TRIE_HASH {
            return None;
        }
        rebuild_trie(account_state.storage_root, state.clone()).ok()
}

// Test version of the function that we are going to use for ExecutionWitnessResult
pub fn rebuild_tries(state_root: H256, keys: Vec<Vec<u8>>, state: Vec<Vec<u8>>) -> Result<(Trie, HashMap<Address, Trie>), ExecutionWitnessError> {
    let state_trie = rebuild_trie(state_root, state.clone())?;
    
    // So keys can either be account addresses or storage slots
    // addresses are 20 u8 long
    let addresses: Vec<&Vec<u8>> = keys.iter()
        .filter(|k| k.len() == 20)
        .collect();

    // Storage slots are 32 u8 long
    // TODO consider removing this
    let _: Vec<H256> = keys.iter()
        .filter(|k: &&Vec<u8>| k.len() == 32)
        .map(|k| H256::from_slice(k))
        .collect();

    let storage_tries: HashMap<Address, Trie> = HashMap::from_iter(
        addresses.iter()
            .filter_map(|addr| Some((
                Address::from_slice(addr),
                rebuild_storage_trie(addr, &state_trie, state.clone())?
            )))
            .collect::<Vec<(Address, Trie)>>()
    );

    Ok((state_trie, storage_tries))
}

#[cfg(test)]
mod test {
    use super::*;


    fn generate_input() -> (H256, Vec<Vec<u8>>, Vec<Vec<u8>>){
        let keys = vec![
            "0000000000000000000000000000000000000001",
            "000f3df6d732807ef1319fb7b8bb8522d0beac02",
            "0000000000000000000000000000000000000000000000000000000000002ffd",
            "0000000000000000000000000000000000000000000000000000000000000ffe"
        ].into_iter()
        .map(|k| hex::decode(k).unwrap())
        .collect();

        let state_root = H256::from_slice(&hex::decode("da87d7f5f91c51508791bbcbd4aa5baf04917830b86985eeb9ad3d5bfb657576")
            .unwrap());

        (state_root, keys, vec![])
    }

    #[test]
    pub fn feda_test(){
        let (state_root, keys, state) = generate_input();
        rebuild_tries(state_root, keys, state);
    }
}
