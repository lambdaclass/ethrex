use std::collections::HashMap;

use ethereum_types::Address;
use ethrex_trie::Trie;
use keccak_hash::H256;

pub fn rebuild_tries(keys: Vec<Vec<u8>>, state: Vec<Vec<u8>>) -> (Trie, HashMap<Address, Trie>) {
    // So keys can either be account addresses or storage slots
    // addresses are 20 u8 long
    let addresses: Vec<Address> = keys.iter()
        .filter(|k| k.len() == 20)
        .map(|k| Address::from_slice(k))
        .collect();

    // Storage slots are 32 u8 long
    let storage_slots: Vec<H256> = keys.iter()
        .filter(|k: &&Vec<u8>| k.len() == 32)
        .map(|k| H256::from_slice(k))
        .collect();

    

    todo!()
}

#[cfg(test)]
mod test {
    use super::*;


    fn generate_input() -> (Vec<Vec<u8>>, Vec<Vec<u8>>){
        let keys = vec![
            "0000000000000000000000000000000000000001",
            "000f3df6d732807ef1319fb7b8bb8522d0beac02",
            "0000000000000000000000000000000000000000000000000000000000002ffd",
            "0000000000000000000000000000000000000000000000000000000000000ffe"
        ].into_iter()
        .map(|k| hex::decode(k).unwrap())
        .collect();

        (keys, vec![])
    }

    #[test]
    pub fn feda_test(){
        let (keys, state) = generate_input();
        rebuild_tries(keys, state);
    }
}
