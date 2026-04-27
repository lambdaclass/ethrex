//! Reproducer for a defensive bug in `get_block_l1_messages` filter logic.
//!
//! The function in `crates/l2/common/src/messages.rs` filters logs as
//!
//! ```ignore
//! log.address == MESSENGER_ADDRESS
//!     && log.topics.contains(&L1MESSAGE_EVENT_SELECTOR)
//! ```
//!
//! `topics.contains(&SELECTOR)` returns true if the selector appears
//! anywhere in the topic list, not specifically at index 0. After the
//! filter passes, the parser unconditionally reads `topics[1]` as the
//! `from` address, `topics[2]` as `data_hash`, and `topics[3]` as
//! `message_id` — so any log with at least four topics that happens to
//! contain `L1MESSAGE_EVENT_SELECTOR` somewhere past index 0 is parsed
//! as a fake `L1Message`. The companion `get_block_l2_out_messages`
//! correctly uses `topics.first() == Some(&L2MESSAGE_EVENT_SELECTOR)`,
//! so this is an inconsistency.
//!
//! Today the only contract whose `address` matches `MESSENGER_ADDRESS`
//! is `Messenger.sol`, which only emits `L1Message` (selector at
//! `topics[0]`) and `L2Message` (only 2 topics, so the parser bails
//! out via `topics.get(2) == None`). So the bug isn't currently
//! reachable through the real EVM emission path. But the filter is
//! still wrong, and any future event added to the messenger with at
//! least three indexed parameters whose values can land on
//! `L1MESSAGE_EVENT_SELECTOR` would silently start producing fake
//! `L1Message`s — which would then become withdrawal-claim leaves
//! committed to L1 by the committer.
//!
//! This test pins the *current* (buggy) behaviour by feeding a hand-
//! crafted log into `get_block_l1_messages`. After the fix
//! (`topics.first() == Some(&L1MESSAGE_EVENT_SELECTOR)`), the filter
//! rejects the crafted log and the assertion has to be updated to
//! `assert_eq!(messages.len(), 0)`.
//!
//! Why a hand-crafted log is acceptable as a reproducer: the function
//! is a public API on `ethrex_l2_common::messages` and is consumed by
//! at least three callers (committer, RPC, state-reconstruct) that
//! are fed receipts from many sources (storage, peers, replay). The
//! filter must hold defensively against any well-formed `Log` value,
//! not only logs synthesised by today's Solidity. EIP-3541 doesn't
//! help here — the prefix rule is on contract bytecode, not on
//! arbitrary topic values.

use bytes::Bytes;
use ethrex_common::{
    H256,
    types::{Log, Receipt, TxType},
};
use ethrex_l2_common::messages::{
    L1MESSAGE_EVENT_SELECTOR, MESSENGER_ADDRESS, get_block_l1_messages,
};

#[test]
fn get_block_l1_messages_misparses_log_with_selector_off_topic_zero() {
    // A log emitted by an account at MESSENGER_ADDRESS, with the
    // L1Message selector sitting at topic[1] rather than topic[0].
    // Topic[0] is some unrelated 32-byte value (here all 0xAA).
    let unrelated_topic_0 = H256([0xAA; 32]);
    let fake_data_hash = H256([0xBB; 32]);
    let fake_message_id = {
        let mut bytes = [0u8; 32];
        // message_id = 1 in big-endian
        bytes[31] = 1;
        H256(bytes)
    };

    let crafted_log = Log {
        address: MESSENGER_ADDRESS,
        topics: vec![
            unrelated_topic_0,         // topic[0] — NOT L1MESSAGE_EVENT_SELECTOR
            *L1MESSAGE_EVENT_SELECTOR, // topic[1] — read by parser as `from`
            fake_data_hash,            // topic[2] — read by parser as `data_hash`
            fake_message_id,           // topic[3] — read by parser as `message_id`
        ],
        data: Bytes::new(),
    };

    let receipt = Receipt::new(TxType::EIP1559, true, 21_000, vec![crafted_log]);
    let messages = get_block_l1_messages(&[receipt]);

    // Current (buggy) behaviour: the `topics.contains(&SELECTOR)` filter
    // accepts this log because L1MESSAGE_EVENT_SELECTOR is at topics[1],
    // and the parser proceeds to fabricate an `L1Message` whose `from`
    // is the bottom 20 bytes of L1MESSAGE_EVENT_SELECTOR.
    assert_eq!(
        messages.len(),
        1,
        "Expected the buggy filter to (incorrectly) accept the crafted log. \
         If this assertion now fails because messages.len() == 0, the filter \
         was tightened to check `topics.first() == Some(&SELECTOR)` — update \
         this test to assert `0` once the fix lands.",
    );

    // The fabricated L1Message has from = bottom-20-bytes of the selector,
    // confirming the parser indexed off topic[1] thinking it was an address.
    let parsed = &messages[0];
    let mut expected_from = [0u8; 20];
    expected_from.copy_from_slice(&L1MESSAGE_EVENT_SELECTOR.0[12..32]);
    assert_eq!(
        parsed.from.0, expected_from,
        "Parser should have read topic[1] (the selector) as the `from` field"
    );
    assert_eq!(parsed.data_hash, fake_data_hash);
    assert_eq!(parsed.message_id.low_u64(), 1);
}
