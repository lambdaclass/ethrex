use custom_runner::benchmark::Benchmark;

fn main() {
    let json = r#"
    {
        "fork": "forkName",
        "env": {
            "chain_id": "0x1",
            "gas_limit": "0x100000",
            "timestamp": "0x5f5e100"
        },
        "pre": {
            "0xabc": {
                "balance": "0x1",
                "code": "0x6000",
                "nonce": "0x0",
                "storage": {
                    "0x00": "0x01"
                }
            }
        },
        "transaction": {
            "from": "0xabc",
            "to": "0xdef",
            "value": "0x0",
            "data": "0x",
            "gas": "0x5208",
            "gas_price": "0x3b9aca00"
        },
        "initial_memory": "0x23901903190",
        "initial_stack": ["0x00000012312312", "0x123120319230190"]
    }
    "#;

    let benchmark: Benchmark = serde_json::from_str(json).unwrap();
    println!("{:#?}", benchmark);
}
