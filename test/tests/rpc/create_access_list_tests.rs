//! `eth_createAccessList` must return the list that *minimises* gas.
//!
//! Under the EIP-8038 repricing an access-list entry costs the cold charge minus
//! `WARM_ACCESS`, so prepaying is gas-neutral for a single access — while the
//! entry's floor tokens still raise the EIP-7623 calldata floor. The minimal
//! list for an access that touches an account but no storage slot is therefore
//! the empty one, and differential testing found four of six clients returning a
//! costlier list because their heuristic predates the repricing.
//!
//! ethrex gets this right, but incidentally: `Substate::make_access_list` derives
//! entries from `accessed_storage_slots`, so an address with no storage access
//! cannot appear, and there is no profitability heuristic to keep in step with
//! pricing. This test is what makes the behaviour durable.

use ethrex_rpc::test_utils::{call_http, default_context_with_storage, setup_store};

/// A genesis-funded EOA, so the call is not rejected for insufficient balance.
const RICH: &str = "0x3f1eae7d46d88f08fc2f8ed27fcb2ab183eb2d0e";

/// A plain value transfer touches the recipient account and no storage, so the
/// gas-minimal access list is empty.
#[tokio::test]
async fn create_access_list_omits_an_account_only_access() {
    let context = default_context_with_storage(setup_store().await).await;
    let body = format!(
        r#"{{"jsonrpc":"2.0","method":"eth_createAccessList","params":[{{"from":"{RICH}","to":"{RICH}","value":"0x1"}},"latest"],"id":1}}"#
    );
    let response = call_http(context, body).await;
    let result = &response["result"];
    assert!(
        result.get("error").is_none(),
        "eth_createAccessList should not report an execution error: {response}"
    );
    let access_list = result["accessList"]
        .as_array()
        .unwrap_or_else(|| panic!("accessList should be an array, got {response}"));
    assert!(
        access_list.is_empty(),
        "an account-only access must yield an empty access list, got {access_list:?}"
    );
}
