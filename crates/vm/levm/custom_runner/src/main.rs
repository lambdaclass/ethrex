use custom_runner::benchmark::ExecutionInput;

fn main() {
    let json = r#"
    {
        env: {
            "gas_limit": "100",
            "origin": "0x0000000000000000000000000000000000000001",
            "gas_price": "0x1"
        },
        initial_memory: "0x239019031905",
        transaction: {
            "nonce": "0",
            "gas_limit": "21000",
            "value": "0x10000000000000000"
        },
        initial_stack: [
            "0x1",
            "0x2",
            "0x3"
        ],
    }
    "#;

    //json5 because it is more flexible than normal json: trailing commas allowed, comments, unquoted keys, etc.
    let benchmark: ExecutionInput = json5::from_str(json).unwrap();
    println!("{:#?}", benchmark);
}
