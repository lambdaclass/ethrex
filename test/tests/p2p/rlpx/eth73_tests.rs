//! eth/73, provisional: eth/72 plus the wrapper exchange of EIP-8504.
//!
//! A dependency transaction travels only inside its wrapper, which a node
//! announces by hash and sends on request. The mempool does not verify witnesses,
//! admission does, so these tests fill the pool directly with placeholder ones.

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    mempool::{PooledDependencyWitnesses, ProofWrapperAnnouncement},
};
use ethrex_common::{
    Address, H256, U256,
    types::{
        Frame, FrameTransaction, LEANSPHINCS_SCHEME, LEANSPHINCS_SIGNATURE_SIZE,
        LEANSPHINCS_VERIFICATION_GAS, MempoolTransaction, PROOF_FRAME_MODE, ProofWrapper,
        Transaction,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_p2p::rlpx::{
    Message,
    eth::{
        cells::GetCells,
        eth73::proof_wrappers::{
            GetProofWrappers, MAX_PROOF_WRAPPER_ANNOUNCEMENTS, MAX_PROOF_WRAPPER_REQUEST_HASHES,
            MAX_REQUESTED_FROM_PEER, NewProofWrapperHashes, PROOF_WRAPPERS_RESPONSE_SOFT_LIMIT,
            ProofWrappers, RequestedFromPeer,
        },
    },
    message::{EthCapVersion, RLPxMessage},
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{EngineType, Store};

fn encode<M: RLPxMessage>(msg: &M) -> Vec<u8> {
    let mut buf = vec![];
    msg.encode(&mut buf).expect("encode");
    buf
}

fn decode(code: u8, data: &[u8], version: EthCapVersion) -> Option<Message> {
    Message::decode(version.eth_capability_offset() + code, data, version, None).ok()
}

fn test_blockchain() -> Blockchain {
    let store = Store::new("", EngineType::InMemory).expect("in-memory store");
    Blockchain::default_with_store(store)
}

/// A wrapper of a transaction from `sender` whose proof frame declares
/// `dependencies` triples, with a placeholder witness for each.
fn wrapper(sender: u64, dependencies: usize) -> ProofWrapper {
    let mut data = Vec::new();
    for key in 0..dependencies {
        data.extend_from_slice(&[0u8; 31]);
        data.push(LEANSPHINCS_SCHEME);
        data.extend_from_slice(&[0u8; 32]);
        data.extend_from_slice(H256::from_low_u64_be(key as u64 + 1).as_bytes());
    }
    let tx = FrameTransaction {
        sender: Address::from_low_u64_be(sender),
        frames: vec![Frame {
            mode: PROOF_FRAME_MODE,
            flags: 0,
            target: None,
            gas_limit: dependencies as u64 * LEANSPHINCS_VERIFICATION_GAS,
            state_gas_limit: 0,
            value: U256::zero(),
            data: Bytes::from(data),
        }],
        ..Default::default()
    };
    let witnesses = vec![Bytes::from(vec![0x5a; LEANSPHINCS_SIGNATURE_SIZE]); dependencies];
    ProofWrapper::new(tx, None, witnesses)
}

fn announcement(wrapper: &ProofWrapper) -> ProofWrapperAnnouncement {
    ProofWrapperAnnouncement {
        wrapper_hash: wrapper.hash(&NativeCrypto),
        tx_hash: wrapper.tx_hash(&NativeCrypto),
        wrapper_len: wrapper.length(),
    }
}

/// Puts the wrapper's transaction in the pool with its witnesses, as admission
/// does once it has verified them.
fn pool(
    blockchain: &Blockchain,
    wrapper: &ProofWrapper,
    broadcast: bool,
) -> ProofWrapperAnnouncement {
    let announced = announcement(wrapper);
    let sender = wrapper.tx.sender;
    blockchain
        .mempool
        .add_dependency_transaction(
            announced.tx_hash,
            sender,
            MempoolTransaction::new(Transaction::FrameTransaction(wrapper.tx.clone()), sender),
            None,
            None,
            PooledDependencyWitnesses {
                wrapper_hash: announced.wrapper_hash,
                wrapper_len: announced.wrapper_len,
                witnesses: wrapper.witnesses.clone(),
            },
            broadcast,
        )
        .expect("add the dependency transaction");
    announced
}

// ── The three messages ───────────────────────────────────────────────────────

#[test]
fn wrapper_messages_round_trip_on_eth73() {
    let first = wrapper(1, 1);
    let second = wrapper(2, 2);

    let announce = NewProofWrapperHashes::new(vec![announcement(&first), announcement(&second)]);
    let Some(Message::NewProofWrapperHashes(decoded)) = decode(
        NewProofWrapperHashes::CODE,
        &encode(&announce),
        EthCapVersion::V73,
    ) else {
        panic!("expected NewProofWrapperHashes");
    };
    assert_eq!(decoded, announce);

    let request = GetProofWrappers::new(7, vec![announce.wrappers[0].wrapper_hash]);
    let Some(Message::GetProofWrappers(decoded)) = decode(
        GetProofWrappers::CODE,
        &encode(&request),
        EthCapVersion::V73,
    ) else {
        panic!("expected GetProofWrappers");
    };
    assert_eq!(decoded, request);

    let response = ProofWrappers::new(7, vec![first, second]);
    let Some(Message::ProofWrappers(decoded)) =
        decode(ProofWrappers::CODE, &encode(&response), EthCapVersion::V73)
    else {
        panic!("expected ProofWrappers");
    };
    assert_eq!(decoded, response);
}

#[test]
fn wrapper_messages_take_the_three_codes_after_cells() {
    assert_eq!(NewProofWrapperHashes::CODE, 0x16);
    assert_eq!(GetProofWrappers::CODE, 0x17);
    assert_eq!(ProofWrappers::CODE, 0x18);
    // The capabilities after `eth` move up by the three new messages.
    assert_eq!(
        EthCapVersion::V73.snap_capability_offset(),
        EthCapVersion::V72.snap_capability_offset() + 3
    );
    assert_eq!(
        EthCapVersion::V73.based_capability_offset(),
        EthCapVersion::V72.based_capability_offset() + 3
    );
}

#[test]
fn eth72_session_has_no_wrapper_messages() {
    let wrapper = wrapper(1, 1);
    let announce = encode(&NewProofWrapperHashes::new(vec![announcement(&wrapper)]));
    let request = encode(&GetProofWrappers::new(7, vec![H256::zero()]));
    let response = encode(&ProofWrappers::new(7, vec![wrapper]));
    for version in [EthCapVersion::V71, EthCapVersion::V72] {
        assert!(decode(NewProofWrapperHashes::CODE, &announce, version).is_none());
        assert!(decode(GetProofWrappers::CODE, &request, version).is_none());
        assert!(decode(ProofWrappers::CODE, &response, version).is_none());
    }
}

#[test]
fn eth73_session_keeps_the_eth72_messages() {
    let get_cells = encode(&GetCells::new(1, vec![H256::zero()], 0b1));
    assert!(matches!(
        decode(GetCells::CODE, &get_cells, EthCapVersion::V73),
        Some(Message::GetCells(_))
    ));
}

#[test]
fn oversized_announcements_and_requests_are_rejected() {
    let entry = ProofWrapperAnnouncement {
        wrapper_hash: H256::repeat_byte(1),
        tx_hash: H256::repeat_byte(2),
        wrapper_len: 8_000,
    };
    let at_limit = NewProofWrapperHashes::new(vec![entry; MAX_PROOF_WRAPPER_ANNOUNCEMENTS]);
    assert!(NewProofWrapperHashes::decode(&encode(&at_limit)).is_ok());
    let over = NewProofWrapperHashes::new(vec![entry; MAX_PROOF_WRAPPER_ANNOUNCEMENTS + 1]);
    assert!(NewProofWrapperHashes::decode(&encode(&over)).is_err());

    let at_limit = GetProofWrappers::new(1, vec![H256::zero(); MAX_PROOF_WRAPPER_REQUEST_HASHES]);
    assert!(GetProofWrappers::decode(&encode(&at_limit)).is_ok());
    let over = GetProofWrappers::new(1, vec![H256::zero(); MAX_PROOF_WRAPPER_REQUEST_HASHES + 1]);
    assert!(GetProofWrappers::decode(&encode(&over)).is_err());
}

// ── Serving and announcing from the pool ─────────────────────────────────────

#[test]
fn get_proof_wrappers_serves_what_the_pool_holds() {
    let blockchain = test_blockchain();
    let public = wrapper(1, 1);
    let private = wrapper(2, 1);
    let public_hash = pool(&blockchain, &public, true).wrapper_hash;
    let private_hash = pool(&blockchain, &private, false).wrapper_hash;

    // The public wrapper is served once, byte for byte, however often it is
    // asked for. An unknown hash and a private transaction's wrapper are left out.
    let response = GetProofWrappers::new(
        9,
        vec![
            public_hash,
            H256::repeat_byte(0xEE),
            private_hash,
            public_hash,
        ],
    )
    .handle(&blockchain)
    .expect("handle");
    assert_eq!(response.id, 9);
    assert_eq!(response.wrappers.len(), 1);
    assert_eq!(response.wrappers[0].encode_to_vec(), public.encode_to_vec());
    assert_eq!(response.wrappers[0].hash(&NativeCrypto), public_hash);
}

#[test]
fn get_proof_wrappers_stops_at_the_response_soft_limit() {
    let blockchain = test_blockchain();
    // Sixteen dependencies make a wrapper of about 125 KiB: twenty of them do not
    // fit one response.
    let hashes: Vec<H256> = (1..=20)
        .map(|sender| pool(&blockchain, &wrapper(sender, 16), true).wrapper_hash)
        .collect();

    let response = GetProofWrappers::new(1, hashes.clone())
        .handle(&blockchain)
        .expect("handle");
    let served_bytes: usize = response.wrappers.iter().map(|w| w.length()).sum();
    assert!(!response.wrappers.is_empty());
    assert!(response.wrappers.len() < hashes.len());
    assert!(served_bytes <= PROOF_WRAPPERS_RESPONSE_SOFT_LIMIT);
    // What was served is a prefix of what was asked for.
    for (served, requested) in response.wrappers.iter().zip(&hashes) {
        assert_eq!(served.hash(&NativeCrypto), *requested);
    }
}

#[test]
fn dependency_transactions_are_announced_as_wrappers_only() {
    let blockchain = test_blockchain();
    let mempool = &blockchain.mempool;
    let public = pool(&blockchain, &wrapper(1, 1), true);
    let private = pool(&blockchain, &wrapper(2, 1), false);

    // Neither is offered as a transaction, queued or to a new peer.
    assert!(mempool.get_txs_for_broadcast().unwrap().is_empty());
    assert!(mempool.get_txs_for_new_peer_dump().unwrap().is_empty());
    assert!(
        blockchain
            .get_p2p_transaction_by_hash(&public.tx_hash)
            .is_err()
    );

    // The public one is queued for announcement; the private one never is.
    assert_eq!(
        mempool.get_proof_wrappers_for_broadcast().unwrap(),
        vec![public]
    );
    assert_eq!(
        mempool.get_proof_wrappers_for_new_peer_dump().unwrap(),
        vec![public]
    );
    assert!(
        mempool
            .get_proof_wrapper(private.wrapper_hash)
            .unwrap()
            .is_none()
    );

    // Once announced it leaves the queue, and a peer that connects later still
    // learns of it.
    mempool.remove_broadcasted_txs(&[public.tx_hash]).unwrap();
    assert!(
        mempool
            .get_proof_wrappers_for_broadcast()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        mempool.get_proof_wrappers_for_new_peer_dump().unwrap(),
        vec![public]
    );
}

// ── Requesting ───────────────────────────────────────────────────────────────

#[test]
fn a_wrapper_is_requested_from_one_peer_at_a_time() {
    let blockchain = test_blockchain();
    let mempool = &blockchain.mempool;
    let pooled = wrapper(1, 1);
    let pooled_announcement = pool(&blockchain, &pooled, true);
    let first = announcement(&wrapper(2, 1));
    let second = announcement(&wrapper(3, 1));
    // Another wrapper for the same transaction as `first`.
    let first_again = ProofWrapperAnnouncement {
        wrapper_hash: H256::repeat_byte(0xAB),
        ..first
    };

    // A pooled transaction's wrapper is not requested, and one transaction is
    // requested once even when two of its wrappers are announced together.
    let (to_request, in_flight_elsewhere) = mempool
        .reserve_proof_wrappers(vec![pooled_announcement, first, first_again, second])
        .unwrap();
    assert_eq!(to_request, vec![first, second]);
    assert!(in_flight_elsewhere.is_empty());

    // While that request is out, a second peer announcing the same transactions
    // requests nothing and keeps them to try later.
    let (to_request, in_flight_elsewhere) = mempool
        .reserve_proof_wrappers(vec![first_again, second])
        .unwrap();
    assert!(to_request.is_empty());
    assert_eq!(in_flight_elsewhere, vec![first_again, second]);

    // The first peer answered with `second` only: both marks are released, and
    // the second peer can now fetch what is still missing.
    mempool
        .clear_in_flight_proof_wrappers(&[first.tx_hash, second.tx_hash])
        .unwrap();
    let (to_request, in_flight_elsewhere) = mempool
        .reserve_proof_wrappers(vec![first_again, second])
        .unwrap();
    assert_eq!(to_request, vec![first_again, second]);
    assert!(in_flight_elsewhere.is_empty());
}

#[test]
fn a_transaction_is_requested_from_a_peer_at_most_once() {
    let first = announcement(&wrapper(1, 1));
    let second = announcement(&wrapper(2, 1));
    let mut requested = RequestedFromPeer::default();

    // Nothing was requested yet: both announcements stand.
    let mut announced = vec![first, second];
    requested.retain_unrequested(&mut announced);
    assert_eq!(announced, vec![first, second]);

    // Once `first` was requested, the peer announcing it again, under the same
    // wrapper or another one for the same transaction, changes nothing: whether
    // it answered, withheld it or sent a wrapper that was not admitted.
    requested.record(&[first]);
    let first_again = ProofWrapperAnnouncement {
        wrapper_hash: H256::repeat_byte(0xAB),
        ..first
    };
    let mut announced = vec![first, first_again, second];
    requested.retain_unrequested(&mut announced);
    assert_eq!(announced, vec![second]);

    // The memory is bounded: past the limit it starts over.
    let flood: Vec<ProofWrapperAnnouncement> = (0..MAX_REQUESTED_FROM_PEER as u64)
        .map(|n| ProofWrapperAnnouncement {
            wrapper_hash: H256::from_low_u64_be(n),
            tx_hash: H256::from_low_u64_be(n),
            wrapper_len: 8_000,
        })
        .collect();
    requested.record(&flood);
    let mut announced = vec![first];
    requested.retain_unrequested(&mut announced);
    assert_eq!(announced, vec![first]);
    let mut announced = vec![flood[0]];
    requested.retain_unrequested(&mut announced);
    assert!(announced.is_empty());
}
