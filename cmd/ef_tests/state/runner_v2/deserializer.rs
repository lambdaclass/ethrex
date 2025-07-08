use std::collections::HashMap;

use ethrex_common::types::Fork;
use serde::Deserialize;

use crate::runner_v2::types::TestPostValue;

pub fn deserialize_post<'de, D>(
    deserializer: D,
) -> Result<HashMap<Fork, Vec<TestPostValue>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let post_deserialized = HashMap::<String, Vec<TestPostValue>>::deserialize(deserializer)?;
    let mut post_parsed = HashMap::new();
    for (fork_str, values) in post_deserialized {
        let fork = match fork_str.as_str() {
            "Frontier" => Fork::Frontier,
            "Homestead" => Fork::Homestead,
            "Constantinople" => Fork::Constantinople,
            "ConstantinopleFix" | "Petersburg" => Fork::Petersburg,
            "Istanbul" => Fork::Istanbul,
            "Berlin" => Fork::Berlin,
            "London" => Fork::London,
            "Paris" | "Merge" => Fork::Paris,
            "Shanghai" => Fork::Shanghai,
            "Cancun" => Fork::Cancun,
            "Prague" => Fork::Prague,
            "Byzantium" => Fork::Byzantium,
            "EIP158" => Fork::SpuriousDragon,
            "EIP150" => Fork::Tangerine,
            other => {
                return Err(serde::de::Error::custom(format!(
                    "Unknown fork name: {other}",
                )));
            }
        };
        post_parsed.insert(fork, values);
    }

    Ok(post_parsed)
}
