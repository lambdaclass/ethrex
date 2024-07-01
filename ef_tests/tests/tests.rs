use ::ef_tests::types::TestUnit;
use ethrex_core::evm::{execute_tx, SpecId};

fn execute_test(test: TestUnit) {
    // TODO: Add support for multiple blocks and multiple transactions per block.
    let transaction = test
        .blocks
        .first()
        .unwrap()
        .transactions
        .as_ref()
        .unwrap()
        .first()
        .unwrap();
    let pre = test.pre.into_iter().map(|(k, v)| (k, v.into())).collect();
    assert!(execute_tx(
        &transaction.clone().into(),
        &test
            .blocks
            .first()
            .as_ref()
            .unwrap()
            .block_header
            .clone()
            .unwrap()
            .into(),
        &pre,
        SpecId::CANCUN,
    )
    .is_success());
}

#[cfg(test)]
mod ef_tests {
    use std::collections::HashMap;

    use ef_tests::types::TestUnit;

    use crate::execute_test;

    #[test]
    fn add11_test() {
        let s: String =
            std::fs::read_to_string("./vectors/add11.json").expect("Unable to read file");
        let tests: HashMap<String, TestUnit> =
            serde_json::from_str(&s).expect("Unable to parse JSON");

        for (_k, test) in tests {
            execute_test(test)
        }
    }

    #[test]
    fn solidity_example_test() {
        let s: String =
            std::fs::read_to_string("./vectors/solidityExample.json").expect("Unable to read file");
        let tests: HashMap<String, TestUnit> =
            serde_json::from_str(&s).expect("Unable to parse JSON");

        for (_k, test) in tests {
            execute_test(test)
        }
    }
}
