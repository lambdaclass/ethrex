use crate::rlpx::connection::server::{broadcast_message, send};
use crate::rlpx::l2::messages::{BatchSealed, L2Message, NewBlock};
use crate::rlpx::utils::{get_pub_key, log_peer_error};
use crate::rlpx::{connection::server::Established, error::RLPxError, message::Message};
use ethereum_types::Address;
use ethrex_blockchain::error::ChainError;
use ethrex_blockchain::fork_choice::apply_fork_choice;
use ethrex_common::types::Block;
use ethrex_storage_rollup::StoreRollup;
use secp256k1::{Message as SecpMessage, SecretKey};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::time::Instant;
use tracing::{debug, info, warn};

use super::messages::batch_hash;
use super::{PERIODIC_BATCH_BROADCAST_INTERVAL, PERIODIC_BLOCK_BROADCAST_INTERVAL};

#[derive(Debug, Clone)]
pub struct L2ConnectedState {
    pub latest_block_sent: u64,
    pub latest_block_added: u64,
    pub latest_batch_sent: u64,
    pub blocks_on_queue: BTreeMap<u64, Arc<Block>>,
    pub store_rollup: StoreRollup,
    pub committer_key: Arc<SecretKey>,
    pub next_block_broadcast: Instant,
    pub next_batch_broadcast: Instant,
}

#[derive(Debug, Clone)]
pub struct P2PBasedContext {
    pub store_rollup: StoreRollup,
    pub committer_key: Arc<SecretKey>,
}

#[derive(Debug, Clone)]
pub enum L2ConnState {
    Unsupported,
    Disconnected(P2PBasedContext),
    Connected(L2ConnectedState),
}

#[derive(Debug, Clone)]
pub enum L2Cast {
    BlockBroadcast,
    BatchBroadcast,
}

impl L2ConnState {
    pub(crate) fn connection_state_mut(&mut self) -> Result<&mut L2ConnectedState, RLPxError> {
        match self {
            Self::Unsupported => Err(RLPxError::IncompatibleProtocol),
            Self::Disconnected(_) => Err(RLPxError::L2CapabilityNotNegotiated),
            Self::Connected(conn_state) => Ok(conn_state),
        }
    }
    pub(crate) fn connection_state(&self) -> Result<&L2ConnectedState, RLPxError> {
        match self {
            Self::Unsupported => Err(RLPxError::IncompatibleProtocol),
            Self::Disconnected(_) => Err(RLPxError::L2CapabilityNotNegotiated),
            Self::Connected(conn_state) => Ok(conn_state),
        }
    }

    pub(crate) fn set_established(&mut self) -> Result<(), RLPxError> {
        match self {
            Self::Unsupported => Err(RLPxError::IncompatibleProtocol),
            Self::Disconnected(ctxt) => {
                let state = L2ConnectedState {
                    latest_block_sent: 0,
                    latest_block_added: 0,
                    blocks_on_queue: BTreeMap::new(),
                    latest_batch_sent: 0,
                    store_rollup: ctxt.store_rollup.clone(),
                    committer_key: ctxt.committer_key.clone(),
                    next_block_broadcast: Instant::now() + PERIODIC_BLOCK_BROADCAST_INTERVAL,
                    next_batch_broadcast: Instant::now() + PERIODIC_BATCH_BROADCAST_INTERVAL,
                };
                *self = L2ConnState::Connected(state);
                Ok(())
            }
            Self::Connected(_) => Ok(()),
        }
    }
}

fn validate_signature(_recovered_lead_sequencer: Address) -> bool {
    // Until the RPC module can be included in the P2P crate, we skip the validation
    true
}

pub(crate) async fn handle_based_capability_message(
    established: &mut Established,
    msg: L2Message,
) -> Result<(), RLPxError> {
    established.l2_state.connection_state()?;
    match msg {
        L2Message::BatchSealed(ref batch_sealed_msg) => {
            if should_process_batch_sealed(established, batch_sealed_msg).await? {
                process_batch_sealed(established, batch_sealed_msg).await?;
                broadcast_message(established, msg.into())?;
            }
        }
        L2Message::NewBlock(ref new_block_msg) => {
            if should_process_new_block(established, new_block_msg).await? {
                process_new_block(established, new_block_msg).await?;
                broadcast_message(established, msg.into())?;
            }
        }
    }
    Ok(())
}

pub(crate) async fn send_new_block(established: &mut Established) -> Result<(), RLPxError> {
    let latest_block_number = established.storage.get_latest_block_number().await?;
    let latest_block_sent = established
        .l2_state
        .connection_state_mut()?
        .latest_block_sent;
    for block_number in latest_block_sent + 1..=latest_block_number {
        let new_block_msg = {
            let l2_state = established.l2_state.connection_state_mut()?;
            debug!(
                "Broadcasting new block, current: {}, last broadcasted: {}",
                block_number, l2_state.latest_block_sent
            );

            let new_block_body = established
                .storage
                .get_block_body(block_number)
                .await?
                .ok_or(RLPxError::InternalError(
                    "Block body not found after querying for the block number".to_owned(),
                ))?;
            let new_block_header = established.storage.get_block_header(block_number)?.ok_or(
                RLPxError::InternalError(
                    "Block header not found after querying for the block number".to_owned(),
                ),
            )?;
            let new_block = Block {
                header: new_block_header,
                body: new_block_body,
            };
            let (signature, recovery_id) = if let Some(recovered_sig) = l2_state
                .store_rollup
                .get_signature_by_block(new_block.hash())
                .await?
            {
                let mut signature = [0u8; 64];
                let mut recovery_id = [0u8; 4];
                signature.copy_from_slice(&recovered_sig[..64]);
                recovery_id.copy_from_slice(&recovered_sig[64..68]);
                (signature, recovery_id)
            } else {
                let (recovery_id, signature) = secp256k1::SECP256K1
                    .sign_ecdsa_recoverable(
                        &SecpMessage::from_digest(new_block.hash().to_fixed_bytes()),
                        &l2_state.committer_key,
                    )
                    .serialize_compact();
                let recovery_id: [u8; 4] = recovery_id.to_i32().to_be_bytes();
                (signature, recovery_id)
            };
            NewBlock {
                block: new_block.into(),
                signature,
                recovery_id,
            }
        };

        send(established, new_block_msg.into()).await?;
        established
            .l2_state
            .connection_state_mut()?
            .latest_block_sent = block_number;
    }

    Ok(())
}

async fn should_process_new_block(
    established: &mut Established,
    msg: &NewBlock,
) -> Result<bool, RLPxError> {
    let l2_state = established.l2_state.connection_state_mut()?;
    if !established.blockchain.is_synced() {
        debug!("Not processing new block, blockchain is not synced");
        return Ok(false);
    }
    if l2_state.latest_block_added >= msg.block.header.number
        || l2_state
            .blocks_on_queue
            .contains_key(&msg.block.header.number)
    {
        debug!(
            "Block {} received by peer already stored, ignoring it",
            msg.block.header.number
        );
        return Ok(false);
    }

    let block_hash = msg.block.hash();

    let recovered_lead_sequencer = get_pub_key(
        msg.recovery_id,
        &msg.signature,
        *block_hash.as_fixed_bytes(),
    )
    .map_err(|e| {
        log_peer_error(
            &established.node,
            &format!("Failed to recover lead sequencer: {e}"),
        );
        RLPxError::CryptographyError(e.to_string())
    })?;

    if !validate_signature(recovered_lead_sequencer) {
        return Ok(false);
    }
    let mut signature = [0u8; 68];
    signature[..64].copy_from_slice(&msg.signature[..]);
    signature[64..].copy_from_slice(&msg.recovery_id[..]);
    l2_state
        .store_rollup
        .store_signature_by_block(block_hash, signature)
        .await?;
    Ok(true)
}

async fn should_process_batch_sealed(
    established: &mut Established,
    msg: &BatchSealed,
) -> Result<bool, RLPxError> {
    let l2_state = established.l2_state.connection_state_mut()?;
    if !established.blockchain.is_synced() {
        debug!("Not processing new block, blockchain is not synced");
        return Ok(false);
    }
    if l2_state
        .store_rollup
        .contains_batch(&msg.batch.number)
        .await?
    {
        debug!("Batch {} already sealed, ignoring it", msg.batch.number);
        return Ok(false);
    }
    if msg.batch.first_block == msg.batch.last_block {
        // is empty batch
        return Ok(false);
    }
    if l2_state.latest_block_added < msg.batch.last_block {
        debug!(
            "Not processing batch {} because the last block {} is not added yet",
            msg.batch.number, msg.batch.last_block
        );
        return Ok(false);
    }

    let hash = batch_hash(&msg.batch);

    let recovered_lead_sequencer =
        get_pub_key(msg.recovery_id, &msg.signature, hash.0).map_err(|e| {
            log_peer_error(
                &established.node,
                &format!("Failed to recover lead sequencer: {e}"),
            );
            RLPxError::CryptographyError(e.to_string())
        })?;

    if !validate_signature(recovered_lead_sequencer) {
        return Ok(false);
    }

    let mut signature = [0u8; 68];
    signature[..64].copy_from_slice(&msg.signature[..]);
    signature[64..].copy_from_slice(&msg.recovery_id[..]);
    l2_state
        .store_rollup
        .store_signature_by_batch(msg.batch.number, signature)
        .await?;
    Ok(true)
}

async fn process_new_block(established: &mut Established, msg: &NewBlock) -> Result<(), RLPxError> {
    let l2_state = established.l2_state.connection_state_mut()?;
    l2_state
        .blocks_on_queue
        .entry(msg.block.header.number)
        .or_insert_with(|| msg.block.clone());

    let mut next_block_to_add = l2_state.latest_block_added + 1;
    while let Some(block) = l2_state.blocks_on_queue.remove(&next_block_to_add) {
        // This check is necessary if a connection to another peer already applied the block but this connection
        // did not register that update.
        if let Ok(Some(_)) = established.storage.get_block_body(next_block_to_add).await {
            l2_state.latest_block_added = next_block_to_add;
            next_block_to_add += 1;
            continue;
        }
        established
            .blockchain
            .add_block(&block)
            .await
            .inspect_err(|e| {
                log_peer_error(
                    &established.node,
                    &format!(
                        "Error adding new block {} with hash {:?}, error: {e}",
                        block.header.number,
                        block.hash()
                    ),
                );
            })?;
        let block_hash = block.hash();

        apply_fork_choice(&established.storage, block_hash, block_hash, block_hash)
            .await
            .map_err(|e| {
                RLPxError::BlockchainError(ChainError::Custom(format!(
                    "Error adding new block {} with hash {:?}, error: {e}",
                    block.header.number,
                    block.hash()
                )))
            })?;
        info!(
            "Added new block {} with hash {:?}",
            next_block_to_add, block_hash
        );
        l2_state.latest_block_added = next_block_to_add;
        next_block_to_add += 1;
    }
    Ok(())
}

pub(crate) async fn send_sealed_batch(established: &mut Established) -> Result<(), RLPxError> {
    let batch_sealed_msg = {
        let l2_state = established.l2_state.connection_state_mut()?;
        let next_batch_to_send = l2_state.latest_batch_sent + 1;
        if !l2_state
            .store_rollup
            .contains_batch(&next_batch_to_send)
            .await?
        {
            return Ok(());
        }
        let Some(batch) = l2_state.store_rollup.get_batch(next_batch_to_send).await? else {
            return Ok(());
        };
        match l2_state
            .store_rollup
            .get_signature_by_batch(next_batch_to_send)
            .await
        {
            Ok(Some(recovered_sig)) => {
                let (signature, recovery_id) = {
                    let mut signature = [0u8; 64];
                    let mut recovery_id = [0u8; 4];
                    signature.copy_from_slice(&recovered_sig[..64]);
                    recovery_id.copy_from_slice(&recovered_sig[64..68]);
                    (signature, recovery_id)
                };
                BatchSealed::new(batch, signature, recovery_id)
            }
            Ok(None) => {
                BatchSealed::from_batch_and_key(batch, l2_state.committer_key.clone().as_ref())
            }
            Err(err) => {
                warn!(
                    "Fetching signature from store returned an error, defaulting to signing with commiter key: {err}"
                );
                BatchSealed::from_batch_and_key(batch, l2_state.committer_key.clone().as_ref())
            }
        }
    };
    let batch_sealed_msg: Message = batch_sealed_msg.into();
    send(established, batch_sealed_msg).await?;
    established
        .l2_state
        .connection_state_mut()?
        .latest_batch_sent += 1;
    Ok(())
}

async fn process_batch_sealed(
    established: &mut Established,
    msg: &BatchSealed,
) -> Result<(), RLPxError> {
    let l2_state = established.l2_state.connection_state_mut()?;
    l2_state.store_rollup.seal_batch(*msg.batch.clone()).await?;
    info!(
        "Sealed batch {} with blocks from {} to {}",
        msg.batch.number, msg.batch.first_block, msg.batch.last_block
    );
    Ok(())
}

#[cfg(test)]
mod tests {

    // async fn test_store(path: &str) -> Store {
    //     // Get genesis
    //     let file = File::open("../../../test_data/genesis-execution-api.json")
    //         .expect("Failed to open genesis file");
    //     let reader = BufReader::new(file);
    //     let genesis = serde_json::from_reader(reader).expect("Failed to deserialize genesis file");

    //     // Build store with genesis
    //     let store = Store::new(path, EngineType::InMemory).expect("Failed to build DB for testing");

    //     store
    //         .add_initial_state(genesis)
    //         .await
    //         .expect("Failed to add genesis state");

    //     store
    // }

    // /// Creates a new block using the blockchain's payload building logic,
    // /// Copied behavior from smoke_test.rs
    // async fn new_block(store: &Store, parent: &BlockHeader) -> Block {
    //     let args = BuildPayloadArgs {
    //         parent: parent.hash(),
    //         timestamp: parent.timestamp + 1, // Increment timestamp to be valid
    //         fee_recipient: H160::random(),
    //         random: H256::random(),
    //         withdrawals: Some(Vec::new()),
    //         beacon_root: Some(H256::random()),
    //         version: 1,
    //         elasticity_multiplier: ELASTICITY_MULTIPLIER,
    //     };

    //     // Create a temporary blockchain instance to use its building logic
    //     let blockchain = Blockchain::default_with_store(store.clone());

    //     let block = create_payload(&args, store).unwrap();
    //     let result = blockchain.build_payload(block).await.unwrap();
    //     blockchain.add_block(&result.payload).await.unwrap();
    //     result.payload
    // }

    // /// A helper function to create an RLPxConnection for testing
    // async fn create_rlpx_connection(
    //     signer: SigningKey,
    //     stream: tokio::io::DuplexStream,
    //     codec: RLPxCodec,
    // ) -> RLPxConnection<tokio::io::DuplexStream> {
    //     let node = Node::new(
    //         "127.0.0.1".parse().unwrap(),
    //         30303,
    //         30303,
    //         public_key_from_signing_key(&signer),
    //     );
    //     let storage = test_store("store.db").await;
    //     let blockchain = Arc::new(Blockchain::default_with_store(storage.clone()));
    //     let (broadcast, _) = broadcast::channel(10);
    //     let committer_key = SigningKeySecp256k1::new(&mut rand::rngs::OsRng);

    //     let mut connection = RLPxConnection::new(
    //         signer,
    //         node,
    //         stream,
    //         codec,
    //         storage,
    //         blockchain,
    //         "test-client/0.1.0".to_string(),
    //         broadcast,
    //         Some(P2PBasedContext {
    //             store_rollup: StoreRollup::default(),
    //             committer_key: Arc::new(committer_key),
    //         }),
    //     );
    //     connection
    //         .capabilities
    //         .push(SUPPORTED_BASED_CAPABILITIES[0].clone());
    //     connection.negotiated_eth_capability = Some(SUPPORTED_ETH_CAPABILITIES[0].clone());
    //     connection.blockchain.set_synced();

    //     // Each connection has the same signing key, since for now the signature is not verified. In the future this will change
    //     let l2_state = L2ConnectedState {
    //         latest_block_sent: 0,
    //         latest_block_added: 0,
    //         blocks_on_queue: BTreeMap::new(),
    //         latest_batch_sent: 0,
    //         store_rollup: StoreRollup::default(),
    //         committer_key: SigningKeySecp256k1::from_slice(
    //             &hex::decode("385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924")
    //                 .unwrap(),
    //         )
    //         .unwrap()
    //         .into(),
    //         next_block_broadcast: Instant::now() + PERIODIC_BLOCK_BROADCAST_INTERVAL,
    //         next_batch_broadcast: Instant::now() + PERIODIC_BATCH_BROADCAST_INTERVAL,
    //     };
    //     connection.l2_state = L2ConnState::Connected(l2_state);

    //     connection
    // }

    // /// Helper function to create and send a NewBlock message for the test.
    // async fn send_block(_conn: &mut RLPxConnection<tokio::io::DuplexStream>, _block: &Block) {
    //     let secret_key = _conn
    //         .l2_state
    //         .connection_state()
    //         .unwrap()
    //         .committer_key
    //         .as_ref();
    //     let (recovery_id, signature) = secp256k1::SECP256K1
    //         .sign_ecdsa_recoverable(
    //             &SignedMessage::from_digest(_block.hash().to_fixed_bytes()),
    //             secret_key,
    //         )
    //         .serialize_compact();
    //     let recovery_id: [u8; 4] = recovery_id.to_i32().to_be_bytes();

    //     let message_to_send = NewBlock {
    //         block: _block.clone().into(),
    //         signature,
    //         recovery_id,
    //     };
    //     println!(
    //         "Sender (conn_a) sending block {} with hash {:?}.",
    //         _block.header.number,
    //         _block.hash()
    //     );
    //     _conn.send(message_to_send.into()).await.unwrap();
    // }

    // /// Helper function to create and send a BatchSealed message for the test.
    // async fn send_sealed_batch(
    //     conn: &mut RLPxConnection<tokio::io::DuplexStream>,
    //     batch_number: u64,
    //     first_block: u64,
    //     last_block: u64,
    // ) {
    //     let batch = Batch {
    //         number: batch_number,
    //         first_block,
    //         last_block,
    //         ..Default::default()
    //     };
    //     let secret_key = conn
    //         .l2_state
    //         .connection_state_mut()
    //         .unwrap()
    //         .committer_key
    //         .as_ref();
    //     let (recovery_id, signature) = secp256k1::SECP256K1
    //         .sign_ecdsa_recoverable(
    //             &SignedMessage::from_digest(get_hash_batch_sealed(&batch)),
    //             secret_key,
    //         )
    //         .serialize_compact();
    //     let recovery_id: [u8; 4] = recovery_id.to_i32().to_be_bytes();

    //     let message_to_send = BatchSealed {
    //         batch,
    //         signature,
    //         recovery_id,
    //     };
    //     println!(
    //         "Sender (conn_a) sending sealed batch {} for blocks {}-{}.",
    //         batch_number, first_block, last_block
    //     );
    //     conn.send(message_to_send.into()).await.unwrap();
    // }

    // async fn test_connections() -> (
    //     RLPxConnection<tokio::io::DuplexStream>,
    //     RLPxConnection<tokio::io::DuplexStream>,
    // ) {
    //     // Stream for testing
    //     let (stream_a, stream_b) = duplex(4096);

    //     let eph_sk_a = SecretKey::random(&mut rand::rngs::OsRng);
    //     let nonce_a = H256::random();
    //     let eph_sk_b = SecretKey::random(&mut rand::rngs::OsRng);
    //     let nonce_b = H256::random();
    //     let hashed_nonces = Keccak256::digest([nonce_b.0, nonce_a.0].concat()).into();

    //     let local_state_a = LocalState {
    //         nonce: nonce_a,
    //         ephemeral_key: eph_sk_a.clone(),
    //         init_message: vec![],
    //     };
    //     let remote_state_a = RemoteState {
    //         nonce: nonce_b,
    //         ephemeral_key: eph_sk_b.public_key(),
    //         init_message: vec![],
    //         public_key: H512::zero(),
    //     };
    //     let codec_a = RLPxCodec::new(&local_state_a, &remote_state_a, hashed_nonces).unwrap();

    //     let local_state_b = LocalState {
    //         nonce: nonce_b,
    //         ephemeral_key: eph_sk_b,
    //         init_message: vec![],
    //     };
    //     let remote_state_b = RemoteState {
    //         nonce: nonce_a,
    //         ephemeral_key: eph_sk_a.public_key(),
    //         init_message: vec![],
    //         public_key: H512::zero(),
    //     };
    //     let codec_b = RLPxCodec::new(&local_state_b, &remote_state_b, hashed_nonces).unwrap();

    //     // Create the two RLPxConnection instances
    //     let conn_a = create_rlpx_connection(
    //         SigningKey::random(&mut rand::rngs::OsRng),
    //         stream_a,
    //         codec_a,
    //     )
    //     .await;
    //     let conn_b = create_rlpx_connection(
    //         SigningKey::random(&mut rand::rngs::OsRng),
    //         stream_b,
    //         codec_b,
    //     )
    //     .await;

    //     (conn_a, conn_b)
    // }

    // #[tokio::test]
    // /// Tests to ensure that blocks are added in the correct order to the RLPxConnection when received out of order.
    // async fn add_block_in_correct_order() {
    //     let (mut conn_a, mut conn_b) = test_connections().await;

    //     let b_task = tokio::spawn(async move {
    //         println!("Receiver task (conn_b) started.");
    //         let mut blocks_received_count = 0;

    //         loop {
    //             let Some(Ok(message)) = conn_b.receive().await else {
    //                 println!("Receiver task (conn_b) stream ended or failed.");
    //                 break;
    //             };

    //             let Message::L2(L2Message::NewBlock(msg)) = message else {
    //                 continue;
    //             };

    //             blocks_received_count += 1;
    //             println!(
    //                 "Receiver task received block {}. Total received: {}",
    //                 msg.block.header.number, blocks_received_count
    //             );

    //             // Process the message
    //             let (dummy_tx, _) = mpsc::channel(1);
    //             match conn_b.handle_message(msg.into(), dummy_tx).await {
    //                 Ok(_) | Err(RLPxError::BroadcastError(_)) => {}
    //                 Err(e) => panic!("handle_message failed: {:?}", e),
    //             }

    //             // Perform assertions based on how many blocks have been received
    //             match blocks_received_count {
    //                 1 => {
    //                     // Received block 3. No checks yet.
    //                 }
    //                 2 => {
    //                     // Received block 2. Now check intermediate state.
    //                     println!("Receiver task: Checking intermediate state...");
    //                     assert_eq!(
    //                         conn_b
    //                             .l2_state
    //                             .connection_state()
    //                             .unwrap()
    //                             .blocks_on_queue
    //                             .len(),
    //                         2,
    //                         "Queue should contain blocks 2 and 3"
    //                     );
    //                     assert!(
    //                         conn_b
    //                             .l2_state
    //                             .connection_state()
    //                             .unwrap()
    //                             .blocks_on_queue
    //                             .contains_key(&2)
    //                     );
    //                     assert!(
    //                         conn_b
    //                             .l2_state
    //                             .connection_state()
    //                             .unwrap()
    //                             .blocks_on_queue
    //                             .contains_key(&3)
    //                     );
    //                     assert_eq!(
    //                         conn_b
    //                             .l2_state
    //                             .connection_state()
    //                             .unwrap()
    //                             .latest_block_added,
    //                         0,
    //                         "No blocks should be added to the chain yet"
    //                     );
    //                 }
    //                 3 => {
    //                     // Received block 1. Now check final state.
    //                     println!("Receiver task: Checking final state...");
    //                     assert!(
    //                         conn_b
    //                             .l2_state
    //                             .connection_state()
    //                             .unwrap()
    //                             .blocks_on_queue
    //                             .is_empty(),
    //                         "Queue should be empty after processing"
    //                     );
    //                     assert_eq!(
    //                         conn_b
    //                             .l2_state
    //                             .connection_state()
    //                             .unwrap()
    //                             .latest_block_added,
    //                         3,
    //                         "All blocks up to 3 should have been added"
    //                     );
    //                     break; // Test is complete, exit the loop
    //                 }
    //                 _ => panic!("Received more blocks than expected"),
    //             }
    //         }
    //     });

    //     // Here we create a new store for simulating another node and create blocks to be sent
    //     let storage_2 = test_store("store_2.db").await;
    //     let genesis_header = storage_2.get_block_header(0).unwrap().unwrap();
    //     let block1 = new_block(&storage_2, &genesis_header).await;
    //     let block2 = new_block(&storage_2, &block1.header).await;
    //     let block3 = new_block(&storage_2, &block2.header).await;

    //     // Send blocks in reverse order
    //     send_block(&mut conn_a, &block3).await;
    //     send_block(&mut conn_a, &block2).await;

    //     // Send the final block that allows the queue to be processed
    //     send_block(&mut conn_a, &block1).await;

    //     // wait for the receiver task to finish
    //     match b_task.await {
    //         Ok(_) => println!("Receiver task completed successfully."),
    //         Err(e) => panic!("Receiver task failed: {:?}", e),
    //     }
    // }

    // #[tokio::test]
    // /// Tests that a batch can be sealed after all its blocks have been received.
    // async fn test_seal_batch_with_blocks() {
    //     let (mut conn_a, mut conn_b) = test_connections().await;

    //     let b_task = tokio::spawn(async move {
    //         println!("Receiver task (conn_b) started.");
    //         let mut blocks_received_count = 0;

    //         loop {
    //             let Some(Ok(message)) = conn_b.receive().await else {
    //                 println!("Receiver task (conn_b) stream ended or failed.");
    //                 break;
    //             };

    //             let (dummy_tx, _) = mpsc::channel(1);
    //             match message {
    //                 Message::L2(L2Message::NewBlock(msg)) => {
    //                     blocks_received_count += 1;
    //                     println!(
    //                         "Receiver task received block {}. Total received: {}",
    //                         msg.block.header.number, blocks_received_count
    //                     );

    //                     // Process the message
    //                     match conn_b.handle_message(msg.clone().into(), dummy_tx).await {
    //                         Ok(_) | Err(RLPxError::BroadcastError(_)) => {}
    //                         Err(e) => panic!("handle_message failed: {:?}", e),
    //                     }

    //                     if blocks_received_count == 3 {
    //                         println!("Receiver task: All blocks received, checking state...");
    //                         assert_eq!(
    //                             conn_b
    //                                 .l2_state
    //                                 .connection_state()
    //                                 .unwrap()
    //                                 .latest_block_added,
    //                             3,
    //                             "All blocks up to 3 should have been added"
    //                         );
    //                     }
    //                 }
    //                 Message::L2(L2Message::BatchSealed(msg)) => {
    //                     println!("Receiver task received sealed batch {}.", msg.batch.number);
    //                     // Process the message
    //                     match conn_b.handle_message(msg.into(), dummy_tx).await {
    //                         Ok(_) | Err(RLPxError::BroadcastError(_)) => {}
    //                         Err(e) => panic!("handle_message failed: {:?}", e),
    //                     }

    //                     println!("Receiver task: Checking for sealed batch...");
    //                     assert!(
    //                         conn_b
    //                             .l2_state
    //                             .connection_state()
    //                             .unwrap()
    //                             .store_rollup
    //                             .contains_batch(&1)
    //                             .await
    //                             .unwrap(),
    //                         "Batch 1 should be sealed in the store"
    //                     );
    //                     break; // Test complete
    //                 }
    //                 _ => panic!("Received unexpected message type in receiver task"),
    //             }
    //         }
    //     });

    //     let storage = test_store("store_for_sending.db").await;
    //     let genesis_header = storage.get_block_header(0).unwrap().unwrap();
    //     let block1 = new_block(&storage, &genesis_header).await;
    //     let block2 = new_block(&storage, &block1.header).await;
    //     let block3 = new_block(&storage, &block2.header).await;

    //     // Send blocks in order
    //     send_block(&mut conn_a, &block1).await;
    //     send_block(&mut conn_a, &block2).await;
    //     send_block(&mut conn_a, &block3).await;

    //     // Now send the sealed batch message
    //     send_sealed_batch(&mut conn_a, 1, 1, 3).await;

    //     // Wait for the receiver task to finish
    //     match b_task.await {
    //         Ok(_) => println!("Receiver task completed successfully."),
    //         Err(e) => panic!("Receiver task failed: {:?}", e),
    //     }
    // }

    // #[tokio::test]
    // /// Tests that a batch cannot be sealed after all its blocks have been received.
    // async fn test_batch_not_seal_with_missing_blocks() {
    //     let (mut conn_a, mut conn_b) = test_connections().await;

    //     let b_task = tokio::spawn(async move {
    //         println!("Receiver task (conn_b) started.");
    //         let mut blocks_received_count = 0;

    //         loop {
    //             let Some(Ok(message)) = conn_b.receive().await else {
    //                 println!("Receiver task (conn_b) stream ended or failed.");
    //                 break;
    //             };

    //             let (dummy_tx, _) = mpsc::channel(1);
    //             match message {
    //                 Message::L2(L2Message::NewBlock(msg)) => {
    //                     blocks_received_count += 1;
    //                     println!(
    //                         "Receiver task received block {}. Total received: {}",
    //                         msg.block.header.number, blocks_received_count
    //                     );

    //                     // Process the message
    //                     match conn_b.handle_message(msg.into(), dummy_tx).await {
    //                         Ok(_) | Err(RLPxError::BroadcastError(_)) => {}
    //                         Err(e) => panic!("handle_message failed: {:?}", e),
    //                     }

    //                     if blocks_received_count == 3 {
    //                         println!("Receiver task: All blocks received, checking state...");
    //                         assert_eq!(
    //                             conn_b
    //                                 .l2_state
    //                                 .connection_state()
    //                                 .unwrap()
    //                                 .latest_block_added,
    //                             3,
    //                             "All blocks up to 3 should have been added"
    //                         );
    //                     }
    //                 }
    //                 Message::L2(L2Message::BatchSealed(msg)) => {
    //                     println!("Receiver task received sealed batch {}.", msg.batch.number);
    //                     // Process the message
    //                     match conn_b.handle_message(msg.into(), dummy_tx).await {
    //                         Ok(_) | Err(RLPxError::BroadcastError(_)) => {}
    //                         Err(e) => panic!("handle_message failed: {:?}", e),
    //                     }

    //                     println!("Receiver task: Checking for sealed batch...");
    //                     assert!(
    //                         !conn_b
    //                             .l2_state
    //                             .connection_state()
    //                             .unwrap()
    //                             .store_rollup
    //                             .contains_batch(&1)
    //                             .await
    //                             .unwrap(),
    //                         "Batch 1 should not be sealed in the store"
    //                     );
    //                     break; // Test complete
    //                 }
    //                 _ => panic!("Received unexpected message type in receiver task"),
    //             }
    //         }
    //     });

    //     let storage = test_store("store_for_sending.db").await;
    //     let genesis_header = storage.get_block_header(0).unwrap().unwrap();
    //     let block1 = new_block(&storage, &genesis_header).await;
    //     let block2 = new_block(&storage, &block1.header).await;
    //     // Skip the third block

    //     // Send blocks in order
    //     send_block(&mut conn_a, &block1).await;
    //     send_block(&mut conn_a, &block2).await;
    //     // Skip the third block

    //     // Now send the sealed batch message
    //     send_sealed_batch(&mut conn_a, 1, 1, 3).await;

    //     // Wait for the receiver task to finish
    //     match b_task.await {
    //         Ok(_) => println!("Receiver task completed successfully."),
    //         Err(e) => panic!("Receiver task failed: {:?}", e),
    //     }
    // }
}
