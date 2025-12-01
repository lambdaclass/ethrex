use crate::types::Genesis;
use serde_json::{Map, Value};
use std::path::Path;

fn sort_config(genesis_map: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let config_keys_order = [
        "chainId",
        "homesteadBlock",
        "daoForkBlock",
        "daoForkSupport",
        "eip150Block",
        "eip150Hash",
        "eip155Block",
        "eip158Block",
        "byzantiumBlock",
        "constantinopleBlock",
        "petersburgBlock",
        "istanbulBlock",
        "muirGlacierBlock",
        "berlinBlock",
        "londonBlock",
        "arrowGlacierBlock",
        "grayGlacierBlock",
        "terminalTotalDifficulty",
        "terminalTotalDifficultyPassed",
        "shanghaiTime",
        "cancunTime",
        "pragueTime",
        "osakaTime",
        "verkleTime",
        "ethash",
        "depositContractAddress",
        "blobSchedule",
        "mergeNetsplitBlock",
        "enableVerkleAtGenesis",
    ];
    let Value::Object(config) = genesis_map
        .get("config")
        .ok_or_else(|| "Genesis file is missing config".to_owned())?
    else {
        return Err("Genesis file config is not a json object".to_owned());
    };
    let mut ordered_config: Map<String, Value> = Map::new();
    for key in config_keys_order {
        // If a key is not present in the config, this means
        // we're reading a genesis file that simply does not support
        // a certain configuration from the genesis block,
        // so we simply ignore it.
        if let Some(value) = config.get(key)
            && *value != Value::Null
        {
            ordered_config.insert(key.to_owned(), value.clone());
        };
    }
    // Check we're not missing any keys before returning
    for key in config.keys() {
        if ordered_config.get(key).is_none() && config.get(key) != Some(&Value::Null) {
            return Err(format!("Missing key in sorted config: {key}"));
        }
    }
    Ok(ordered_config)
}

pub fn write_genesis_as_json(genesis: Genesis, path: &Path) -> Result<(), String> {
    let genesis_json = serde_json::to_string(&genesis)
        .map_err(|e| format!("Could not convert genesis to string: {e}"))?;
    let genesis_as_map: Map<String, Value> = serde_json::from_str(&genesis_json)
        .map_err(|e| format!("Failed to de-serialize genesis file: {e}"))?;
    // Keys sorting based off this ethpandaops example:
    // https://github.com/ethpandaops/ethereum-genesis-generator/blob/master/apps/el-gen/mainnet/genesis.json
    // We actually want 'config' as the first key, but we sort that
    // separately.
    let keys = [
        "nonce",
        "timestamp",
        "extraData",
        "gasLimit",
        "difficulty",
        "mixHash",
        "coinbase",
        "alloc",
    ];
    let ordered_config = sort_config(&genesis_as_map)?;
    // Some keys that are in our genesis file,
    // but are not in the example above or
    // viceversa.
    let optional_keys = [
        "number",
        "gasUsed",
        "parentHash",
        "baseFeePerGas",
        "excessBlobGas",
        "requestsHash",
        "blobGasUsed",
    ];
    // This map will preserve insertion order because this crate uses the 'preserve_order'
    // feature from serde_json.
    let mut ordered_map: Map<String, Value> = serde_json::Map::new();
    ordered_map.insert(
        "config".to_owned(),
        serde_json::Value::Object(ordered_config),
    );
    for k in keys {
        let Some(v) = genesis_as_map.get(k) else {
            return Err(format!("Missing key in read genesis file: {k}"));
        };
        if *v != Value::Null {
            ordered_map.insert(k.to_owned(), v.clone());
        }
    }
    for k in optional_keys {
        if let Some(v) = genesis_as_map.get(k)
            && *v != Value::Null
        {
            ordered_map.insert(k.to_owned(), v.clone().take());
        }
    }
    // Check 1: check we're not missing any keys.
    for k in genesis_as_map.keys() {
        let expected = genesis_as_map.get(k);
        if expected != Some(&Value::Null)
            && ordered_map.contains_key(k) != genesis_as_map.contains_key(k)
        {
            return Err(format!("Genesis serialization is missing a key: {k}"));
        }
    }
    // Check 2: the new ordered map can be turned into a genesis struct.
    let _: Genesis = serde_json::from_value(ordered_map.clone().into())
        .map_err(|e| format!("Error turning into genesis: {e}"))?;
    let to_write = serde_json::to_string_pretty(&ordered_map)
        .map_err(|e| format!("Could not turn map into json: {e}"))?;
    std::fs::write(path, &to_write).map_err(|e| {
        format!(
            "Could not write genesis json to path: {}, error: {e}",
            path.display()
        )
    })
}
