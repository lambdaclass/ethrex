use crate::{
    discovery::{
        contact_table::{ContactValidation, DiscoveryProtocol},
        ip_predictor::IpPredictor,
        lookup::{IterativeLookup, LOOKUP_ALPHA, LOOKUP_BUCKET_SIZE, LookupId},
        query::{
            FindNodesError, FindNodesRequest, MAX_ACTIVE_QUERIES, NodeQuery, QueryOutcome,
            QueryStep,
        },
    },
    discv5::{
        messages::{
            DISTANCES_PER_FIND_NODE_MSG, FindNodeMessage, Handshake, HandshakeAuthdata, Message,
            NodesMessage, Ordinary, Packet, PacketTrait as _, PingMessage, PongMessage,
            TalkResMessage, WhoAreYou, decrypt_message,
        },
        server::{Discv5Message, SessionSource},
        session::{
            build_challenge_data, create_id_signature, derive_session_keys, verify_id_signature,
        },
    },
    metrics::METRICS,
    types::{Node, NodeRecord},
    utils::{compress_pubkey, distance, node_id},
};
use bytes::{Bytes, BytesMut};
use ethrex_common::{H256, H512};
use rand::{Rng, rngs::OsRng};
use secp256k1::{PublicKey, SecretKey, ecdsa::Signature};
use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};
use tracing::{debug, trace, warn};

use super::server::{DiscoveryServer, DiscoveryServerError};

/// Maximum number of ENRs per NODES message (limited by UDP packet size).
const MAX_ENRS_PER_MESSAGE: usize = 3;
/// Nodes not validated within this interval are candidates for revalidation.
const REVALIDATION_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60); // 12 hours
/// Minimum interval between WHOAREYOU packets to the same IP address.
const WHOAREYOU_RATE_LIMIT: Duration = Duration::from_secs(1);
/// Maximum number of WHOAREYOU packets sent globally per second.
const GLOBAL_WHOAREYOU_RATE_LIMIT: u32 = 100;
/// Largest discv5 log-distance: the bit length of a 256-bit XOR.
const MAX_LOG_DISTANCE: u32 = 256;

impl DiscoveryServer {
    pub(crate) async fn discv5_handle_packet(
        &mut self,
        Discv5Message { packet, from }: Discv5Message,
    ) -> Result<(), DiscoveryServerError> {
        #[cfg(feature = "metrics")]
        {
            use ethrex_metrics::p2p::METRICS_P2P;
            match packet.header.flag {
                0x01 => METRICS_P2P.inc_discv5_incoming("WhoAreYou"),
                0x02 => METRICS_P2P.inc_discv5_incoming("Handshake"),
                _ => {}
            }
        }
        match packet.header.flag {
            0x00 => self.discv5_handle_ordinary(packet, from).await,
            0x01 => self.discv5_handle_who_are_you(packet, from).await,
            0x02 => self.discv5_handle_handshake(packet, from).await,
            f => {
                tracing::trace!(protocol = "discv5", "Unexpected flag {f}");
                Err(crate::discv5::messages::PacketCodecError::MalformedData.into())
            }
        }
    }

    async fn discv5_handle_ordinary(
        &mut self,
        packet: Packet,
        addr: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        // Length-checked: a peer can send an ordinary packet with authdata of any length, and
        // reading the src-id before the session lookup must not panic on a short/empty authdata
        // (an unauthenticated single-packet DoS of the discv5 actor).
        let src_id = Ordinary::src_id(&packet)?;

        let decrypt_key = self.contacts.session(&src_id).map(|s| s.inbound_key);

        let discv5 = self.discv5.as_mut().expect("discv5 state must exist");

        let ordinary = match decrypt_key {
            Some(key) => match Ordinary::decode(&packet, &key) {
                Ok(ordinary) => {
                    if let Some(SessionSource { ip: session_ip, .. }) =
                        discv5.session_ips.get(&src_id)
                        && addr.ip() != *session_ip
                    {
                        trace!(
                            protocol = "discv5",
                            from = %src_id,
                            %addr,
                            expected_ip = %session_ip,
                            "IP mismatch for existing session, sending WhoAreYou"
                        );
                        discv5.whoareyou_rate_limit.pop(&(addr.ip(), src_id));
                        return self
                            .discv5_send_who_are_you(packet.header.nonce, src_id, addr)
                            .await;
                    }
                    ordinary
                }
                Err(_) => {
                    trace!(protocol = "discv5", from = %src_id, %addr, "Decryption failed, sending WhoAreYou");
                    return self
                        .discv5_send_who_are_you(packet.header.nonce, src_id, addr)
                        .await;
                }
            },
            None => {
                trace!(protocol = "discv5", from = %src_id, %addr, "No session, sending WhoAreYou");
                return self
                    .discv5_send_who_are_you(packet.header.nonce, src_id, addr)
                    .await;
            }
        };

        // The packet decrypted and arrived from the address this session was established
        // from, which is the only thing that keeps a session alive. Both halves are
        // refreshed together: keys outliving their `session_ips` entry would silently lose
        // the IP-rebinding check above. Placed after the match so a packet from a
        // mismatched IP, or one that failed to decrypt, extends nothing.
        self.contacts.touch_session(&src_id);
        if let Some(discv5) = self.discv5.as_mut()
            && let Some(source) = discv5.session_ips.get_mut(&src_id)
        {
            source.last_used = Instant::now();
        }

        tracing::trace!(protocol = "discv5", received = %ordinary.message, from = %src_id, %addr);

        self.discv5_handle_message(ordinary, addr, None).await
    }

    async fn discv5_handle_who_are_you(
        &mut self,
        packet: Packet,
        addr: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        let nonce = packet.header.nonce;
        let discv5 = self.discv5.as_mut().expect("discv5 state must exist");
        let Some((node, message, _)) = discv5.pending_by_nonce.remove(&nonce) else {
            tracing::trace!(
                protocol = "discv5",
                "Received unexpected WhoAreYou packet. Ignoring it"
            );
            return Ok(());
        };
        tracing::trace!(protocol = "discv5", received = "WhoAreYou", from = %node.node_id(), %addr);

        let challenge_data = build_challenge_data(
            &packet.masking_iv,
            &packet.header.static_header,
            &packet.header.authdata,
        );

        let ephemeral_key = SecretKey::new(&mut OsRng);
        let ephemeral_pubkey = ephemeral_key.public_key(secp256k1::SECP256K1).serialize();

        let Some(dest_pubkey) = compress_pubkey(node.public_key) else {
            return Err(DiscoveryServerError::CryptographyError(
                "Invalid public key".to_string(),
            ));
        };

        let session = derive_session_keys(
            &ephemeral_key,
            &dest_pubkey,
            &self.local_node.node_id(),
            &node.node_id(),
            &challenge_data,
            true,
        );

        let signature = create_id_signature(
            &self.signer,
            &challenge_data,
            &ephemeral_pubkey,
            &node.node_id(),
        );

        self.contacts.set_session(node.node_id(), session);

        let whoareyou = WhoAreYou::decode(&packet)?;
        let record = (self.local_node_record.seq != whoareyou.enr_seq)
            .then(|| self.local_node_record.clone());
        self.discv5_send_handshake(message, signature, &ephemeral_pubkey, node, record)
            .await
    }

    async fn discv5_handle_handshake(
        &mut self,
        packet: Packet,
        addr: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        let authdata = HandshakeAuthdata::decode(&packet.header.authdata)?;
        let src_id = authdata.src_id;

        let discv5 = self.discv5.as_mut().expect("discv5 state must exist");
        let Some((challenge_data, _, _)) = discv5.pending_challenges.remove(&src_id) else {
            trace!(protocol = "discv5", from = %src_id, %addr, "Received unexpected Handshake packet");
            return Ok(());
        };

        let eph_pubkey = PublicKey::from_slice(&authdata.eph_pubkey).map_err(|_| {
            DiscoveryServerError::CryptographyError("Invalid ephemeral pubkey".into())
        })?;

        let src_pubkey = if let Some(contact) = self.contacts.get_contact(&src_id) {
            compress_pubkey(contact.node.public_key)
        } else if let Some(record) = &authdata.record {
            if !record.verify_signature() {
                trace!(from = %src_id, "Handshake ENR signature verification failed");
                return Ok(());
            }
            let pairs = record.pairs();
            let pubkey = pairs
                .secp256k1
                .and_then(|pk| PublicKey::from_slice(pk.as_bytes()).ok());

            if let Some(pk) = &pubkey {
                let uncompressed = pk.serialize_uncompressed();
                let derived_node_id = node_id(&H512::from_slice(&uncompressed[1..]));
                if derived_node_id != src_id {
                    trace!(from = %src_id, "Handshake ENR node_id mismatch");
                    return Ok(());
                }
            }

            pubkey
        } else {
            None
        };

        let Some(src_pubkey) = src_pubkey else {
            trace!(protocol = "discv5", from = %src_id, "Cannot verify handshake: unknown sender public key");
            return Ok(());
        };

        let signature = Signature::from_compact(&authdata.id_signature).map_err(|_| {
            DiscoveryServerError::CryptographyError("Invalid signature format".into())
        })?;

        if !verify_id_signature(
            &src_pubkey,
            &challenge_data,
            &authdata.eph_pubkey,
            &self.local_node.node_id(),
            &signature,
        ) {
            trace!(protocol = "discv5", from = %src_id, "Handshake signature verification failed");
            return Ok(());
        }

        if let Some(record) = &authdata.record {
            self.contacts
                .new_contact_records(vec![record.clone()])
                .await;
        }

        let session = derive_session_keys(
            &self.signer,
            &eph_pubkey,
            &src_id,
            &self.local_node.node_id(),
            &challenge_data,
            false,
        );

        self.contacts.set_session(src_id, session.clone());
        let discv5 = self.discv5.as_mut().expect("discv5 state must exist");
        discv5.session_ips.insert(
            src_id,
            SessionSource {
                ip: addr.ip(),
                last_used: std::time::Instant::now(),
            },
        );

        let mut encrypted = packet.encrypted_message.clone();
        decrypt_message(&session.inbound_key, &packet, &mut encrypted)?;
        let message = Message::decode(&encrypted)?;
        trace!(protocol = "discv5", received = %message, from = %src_id, %addr, "Handshake completed");

        let ordinary = Ordinary { src_id, message };
        self.discv5_handle_message(ordinary, addr, Some(session.outbound_key))
            .await
    }

    pub(crate) async fn discv5_revalidate(&mut self) -> Result<(), DiscoveryServerError> {
        if let Some(contact) = self
            .contacts
            .contact_to_revalidate(REVALIDATION_INTERVAL, DiscoveryProtocol::Discv5)
        {
            let node = contact.node.clone();
            if let Err(e) = self.discv5_send_ping(&node).await {
                trace!(protocol = "discv5", node = %node.node_id(), err = ?e, "Failed to send revalidation PING");
            }
        }
        Ok(())
    }

    pub(crate) async fn discv5_lookup(&mut self) -> Result<(), DiscoveryServerError> {
        if self.discv5.is_none() {
            return Ok(());
        }

        // Remove finished lookups
        self.discv5
            .as_mut()
            .expect("discv5 state must exist")
            .active_lookups
            .retain(|(_, l)| !l.is_finished());

        // If a lookup is already active, advance it instead of starting a new
        // one. Lookups are timer-driven: each tick sends the next alpha queries.
        // Responses feed results into the lookup but don't trigger new queries,
        // which naturally throttles traffic.
        if !self
            .discv5
            .as_ref()
            .expect("discv5 state must exist")
            .active_lookups
            .is_empty()
        {
            return self.advance_v5_lookup().await;
        }

        let mut rng = OsRng;
        let target_id: H256 = rng.r#gen();

        // Seed with closest known nodes from the connection pool
        let seed = self
            .contacts
            .closest_from_pool(target_id, LOOKUP_BUCKET_SIZE);
        if seed.is_empty() {
            trace!(
                protocol = "discv5",
                "No seeds for lookup, connection pool empty"
            );
            return Ok(());
        }

        trace!(
            protocol = "discv5",
            seeds = seed.len(),
            "Starting new iterative lookup"
        );
        let lookup = IterativeLookup::new(target_id, seed);
        let discv5 = self.discv5.as_mut().expect("discv5 state must exist");
        let id = discv5.next_lookup_id();
        discv5.active_lookups.push((id, lookup));

        // Fire the initial queries for the new lookup
        self.advance_v5_lookup().await
    }

    async fn advance_v5_lookup(&mut self) -> Result<(), DiscoveryServerError> {
        let discv5 = match &mut self.discv5 {
            Some(s) => s,
            None => return Ok(()),
        };

        if discv5.active_lookups.is_empty() {
            return Ok(());
        }

        // Collect queries from all active lookups
        let mut queries: Vec<(LookupId, H256, H256, Node)> = Vec::new();
        for (id, lookup) in &mut discv5.active_lookups {
            let target = lookup.target;
            for (node_id, node) in lookup.next_to_query(LOOKUP_ALPHA) {
                queries.push((*id, target, node_id, node));
            }
        }

        for (id, target, node_id, node) in queries {
            let distances = find_node_distances(&target, &node_id);
            if let Err(e) = self.discv5_send_find_node(&node, distances, Some(id)).await {
                debug!(protocol = "discv5", sending = "FindNode", addr = ?node.udp_addr(), err=?e, "Error sending message");
                self.contacts.set_disposable(&node_id);
                METRICS.record_new_discarded_node();
                if let Some(discv5) = &mut self.discv5
                    && let Some((_, lookup)) = discv5
                        .active_lookups
                        .iter_mut()
                        .find(|(lookup_id, _)| *lookup_id == id)
                {
                    lookup.record_timeout();
                }
            }
        }
        Ok(())
    }

    /// Start a predicate query, or tell the caller why it cannot run.
    pub(crate) async fn discv5_start_query(&mut self, request: FindNodesRequest) {
        let Some(discv5) = self.discv5.as_mut() else {
            let _ = request.reply.send(Err(FindNodesError::Discv5Disabled));
            return;
        };
        if discv5.active_queries.len() >= MAX_ACTIVE_QUERIES {
            let _ = request.reply.send(Err(FindNodesError::TooManyQueries));
            return;
        }
        let id = discv5.next_lookup_id();
        trace!(
            protocol = "discv5",
            query = id.0,
            want = request.want,
            "Starting predicate query"
        );
        discv5
            .active_queries
            .push(NodeQuery::new(id, request, Instant::now()));
        self.discv5_drive_query(id).await;
    }

    /// Give up on silent requests and stop queries past their deadline. Run on
    /// the query tick, which is what notices a query whose nodes went quiet.
    pub(crate) async fn discv5_tick_queries(&mut self) {
        let Some(discv5) = self.discv5.as_mut() else {
            return;
        };
        let now = Instant::now();
        let ids: Vec<LookupId> = discv5
            .active_queries
            .iter_mut()
            .map(|query| {
                query.expire_requests(now);
                query.id
            })
            .collect();
        for id in ids {
            self.discv5_drive_query(id).await;
        }
    }

    /// Move a predicate query forward until it waits on the network: send what
    /// it asks to send, start a new walk when the last one is spent, and answer
    /// the caller once it is done.
    async fn discv5_drive_query(&mut self, id: LookupId) {
        loop {
            let now = Instant::now();
            let Some(discv5) = self.discv5.as_mut() else {
                return;
            };
            let Some(pos) = discv5.active_queries.iter().position(|q| q.id == id) else {
                return;
            };
            match discv5.active_queries[pos].poll(now) {
                QueryStep::Wait => return,
                QueryStep::Abandoned => {
                    trace!(
                        protocol = "discv5",
                        query = id.0,
                        "Caller stopped waiting; dropping query"
                    );
                    discv5.active_queries.swap_remove(pos);
                    return;
                }
                QueryStep::Finish(outcome) => {
                    discv5.active_queries.swap_remove(pos).finish(outcome);
                    return;
                }
                QueryStep::Walk => {
                    let target: H256 = OsRng.r#gen();
                    let seeds = self.contacts.closest_records(target, LOOKUP_BUCKET_SIZE);
                    let contacts = &self.contacts;
                    let discv5 = self.discv5.as_mut().expect("discv5 state must exist");
                    if seeds.is_empty() {
                        discv5
                            .active_queries
                            .swap_remove(pos)
                            .finish(QueryOutcome::Exhausted);
                        return;
                    }
                    discv5.active_queries[pos]
                        .start_walk(target, &seeds, |node_id| contacts.could_dial(node_id));
                }
                QueryStep::Send { target, nodes } => {
                    for (node_id, node) in nodes {
                        let distances = find_node_distances(&target, &node_id);
                        match self.discv5_send_find_node(&node, distances, Some(id)).await {
                            Ok(req_id) => {
                                if let Some(query) = self.discv5_query_mut(id) {
                                    query.record_sent(node_id, req_id, now);
                                }
                            }
                            // Its slot stays free, so the next poll refills it.
                            Err(e) => {
                                debug!(protocol = "discv5", sending = "FindNode", addr = ?node.udp_addr(), err=?e, "Error sending message");
                                self.contacts.set_disposable(&node_id);
                                METRICS.record_new_discarded_node();
                            }
                        }
                    }
                }
            }
        }
    }

    fn discv5_query_mut(&mut self, id: LookupId) -> Option<&mut NodeQuery> {
        self.discv5
            .as_mut()?
            .active_queries
            .iter_mut()
            .find(|query| query.id == id)
    }

    /// Send a FINDNODE for `distances`, remembering which lookup asked, if
    /// any, so its NODES response reaches that lookup and no other. Returns
    /// the request id.
    async fn discv5_send_find_node(
        &mut self,
        node: &Node,
        distances: Vec<u32>,
        owner: Option<LookupId>,
    ) -> Result<Bytes, DiscoveryServerError> {
        let req_id = generate_req_id();
        let find_node = Message::FindNode(FindNodeMessage {
            req_id: req_id.clone(),
            distances,
        });
        self.discv5_send_ordinary(find_node, node).await?;
        if let Some(owner) = owner
            && let Some(discv5) = self.discv5.as_mut()
        {
            discv5
                .findnode_owners
                .insert((node.node_id(), req_id.clone()), owner);
        }
        Ok(req_id)
    }

    /// Ask `node` for its current ENR, with a FINDNODE for distance 0. The
    /// answer lands in the contact table like any other record, newer `seq`
    /// and filter included.
    async fn discv5_request_enr(&mut self, node: &Node) -> Result<(), DiscoveryServerError> {
        self.discv5_send_find_node(node, vec![0], None).await?;
        Ok(())
    }

    /// Refresh the ENR of a contact the consumer asked about. A node we hold
    /// no discv5 contact for is skipped: there is no endpoint to ask.
    pub(crate) async fn discv5_refresh_enr(&mut self, node_id: H256) {
        if self.discv5.is_none() {
            return;
        }
        let Some(node) = self
            .contacts
            .get_contact(&node_id)
            .filter(|contact| contact.is_discv5)
            .map(|contact| contact.node.clone())
        else {
            trace!(protocol = "discv5", node = %node_id, "No discv5 contact to refresh the ENR of");
            return;
        };
        if let Err(e) = self.discv5_request_enr(&node).await {
            debug!(protocol = "discv5", node = %node_id, err = ?e, "Failed to request ENR refresh");
        }
    }

    async fn discv5_handle_ping(
        &mut self,
        ping_message: PingMessage,
        sender_id: H256,
        sender_addr: SocketAddr,
        outbound_key: Option<[u8; 16]>,
    ) -> Result<(), DiscoveryServerError> {
        trace!(protocol = "discv5", from = %sender_id, enr_seq = ping_message.enr_seq, "Received PING");

        let pong = Message::Pong(PongMessage {
            req_id: ping_message.req_id,
            enr_seq: self.local_node_record.seq,
            recipient_addr: sender_addr,
        });

        if outbound_key.is_none()
            && let Some(node) = self
                .contacts
                .get_contact(&sender_id)
                .map(|c| c.node.clone())
        {
            return self.discv5_send_ordinary(pong, &node).await;
        }
        let key = self
            .discv5_resolve_outbound_key(&sender_id, outbound_key)
            .await?;
        self.discv5_send_ordinary_to(pong, &sender_id, sender_addr, &key)
            .await?;

        Ok(())
    }

    pub async fn discv5_handle_pong(
        &mut self,
        pong_message: PongMessage,
        sender_id: H256,
    ) -> Result<(), DiscoveryServerError> {
        self.contacts
            .record_pong_received(&sender_id, &pong_message.req_id);

        // Copied out rather than held: the table is plain state, so a live
        // `&Contact` would block the `&mut self` send below.
        if let Some((node, cached_seq)) = self
            .contacts
            .get_contact(&sender_id)
            .map(|c| (c.node.clone(), c.record.as_ref().map_or(0, |r| r.seq)))
            && pong_message.enr_seq > cached_seq
        {
            trace!(
                protocol = "discv5",
                from = %sender_id,
                cached_seq,
                pong_seq = pong_message.enr_seq,
                "ENR seq mismatch, requesting updated ENR (FINDNODE distance 0)"
            );
            self.discv5_request_enr(&node).await?;
        }

        if let Some(ip) = self
            .ip_predictor
            .record_ip_vote(pong_message.recipient_addr.ip(), sender_id)
        {
            self.apply_predicted_ip(ip, "discv5");
        }

        Ok(())
    }

    async fn discv5_handle_find_node(
        &mut self,
        find_node_message: FindNodeMessage,
        sender_id: H256,
        sender_addr: SocketAddr,
        outbound_key: Option<[u8; 16]>,
    ) -> Result<(), DiscoveryServerError> {
        let send_to_contact = match self.contacts.validate_contact(sender_id, sender_addr.ip()) {
            ContactValidation::Valid(contact) => Some(*contact),
            ContactValidation::UnknownContact => None,
            reason => {
                trace!(from = %sender_id, ?reason, "Rejected FINDNODE");
                return Ok(());
            }
        };

        let mut nodes = self
            .contacts
            .nodes_at_distances(&find_node_message.distances);
        if find_node_message.distances.contains(&0) {
            nodes.push(self.local_node_record.clone());
        }

        let key = self
            .discv5_resolve_outbound_key(&sender_id, outbound_key)
            .await?;

        let chunks: Vec<_> = nodes.chunks(MAX_ENRS_PER_MESSAGE).collect();
        if chunks.is_empty() {
            let nodes_message = Message::Nodes(NodesMessage {
                req_id: find_node_message.req_id,
                total: 1,
                nodes: vec![],
            });
            if let Some(contact) = &send_to_contact {
                self.discv5_send_ordinary(nodes_message, &contact.node)
                    .await?;
            } else {
                self.discv5_send_ordinary_to(nodes_message, &sender_id, sender_addr, &key)
                    .await?;
            }
        } else {
            for chunk in &chunks {
                let nodes_message = Message::Nodes(NodesMessage {
                    req_id: find_node_message.req_id.clone(),
                    total: chunks.len() as u64,
                    nodes: chunk.to_vec(),
                });
                if let Some(contact) = &send_to_contact {
                    self.discv5_send_ordinary(nodes_message, &contact.node)
                        .await?;
                } else {
                    self.discv5_send_ordinary_to(nodes_message, &sender_id, sender_addr, &key)
                        .await?;
                }
            }
        }

        Ok(())
    }

    async fn discv5_handle_nodes_message(
        &mut self,
        nodes_message: NodesMessage,
        sender_id: H256,
    ) -> Result<(), DiscoveryServerError> {
        // Only accept a NODES that answers a FINDNODE we actually sent to this
        // peer. Without the check, any peer that has completed a handshake can
        // push ENRs of its choosing into our contact table with an unsolicited
        // NODES, and we then hand them back out in our own FINDNODE responses.
        let solicited = self.discv5.as_ref().is_some_and(|discv5| {
            discv5
                .pending_findnodes
                .contains_key(&(sender_id, nodes_message.req_id.clone()))
        });
        if !solicited {
            trace!(
                protocol = "discv5",
                from = %sender_id,
                "Dropping unsolicited NODES response"
            );
            return Ok(());
        }

        let req_id = nodes_message.req_id;
        let records = self.contacts.new_contact_records(nodes_message.nodes).await;

        // Only the lookup that sent the FINDNODE hears the answer. One with no
        // owner, such as an ENR refresh, or whose lookup is gone, feeds only
        // the contact table above.
        let Some(discv5) = self.discv5.as_mut() else {
            return Ok(());
        };
        let Some(owner) = discv5
            .findnode_owners
            .get(&(sender_id, req_id.clone()))
            .copied()
        else {
            return Ok(());
        };

        // The crawl is not advanced here: its timer drives it, so that its
        // traffic stays controlled.
        if let Some((_, lookup)) = discv5
            .active_lookups
            .iter_mut()
            .find(|(id, _)| *id == owner)
        {
            lookup.feed_results(
                records
                    .into_iter()
                    .map(|(node_id, node, _)| (node_id, node))
                    .collect(),
            );
            lookup.record_response();
            return Ok(());
        }

        // A predicate query is: waiting for the next tick would cost it a slot.
        let contacts = &self.contacts;
        if let Some(query) = discv5.active_queries.iter_mut().find(|q| q.id == owner) {
            query.on_nodes(sender_id, &req_id, &records, |node_id| {
                contacts.could_dial(node_id)
            });
            self.discv5_drive_query(owner).await;
        }

        Ok(())
    }

    async fn discv5_send_ping(&mut self, node: &Node) -> Result<(), DiscoveryServerError> {
        let req_id = generate_req_id();

        let ping = Message::Ping(PingMessage {
            req_id: req_id.clone(),
            enr_seq: self.local_node_record.seq,
        });

        self.discv5_send_ordinary(ping, node).await?;
        self.contacts.record_ping_sent(&node.node_id(), req_id);

        Ok(())
    }

    async fn discv5_send_ordinary(
        &mut self,
        message: Message,
        node: &Node,
    ) -> Result<(), DiscoveryServerError> {
        #[cfg(feature = "metrics")]
        {
            use ethrex_metrics::p2p::METRICS_P2P;
            METRICS_P2P.inc_discv5_outgoing(message.metric_label());
        }
        let ordinary = Ordinary {
            src_id: self.local_node.node_id(),
            message: message.clone(),
        };
        let encrypt_key = match self.contacts.session(&node.node_id()) {
            Some(s) => s.outbound_key,
            None => {
                trace!(
                    protocol = "discv5",
                    node = %node.node_id(),
                    "No session found in send_ordinary, using zeroed key to trigger handshake"
                );
                [0; 16]
            }
        };

        let discv5 = self.discv5.as_mut().expect("discv5 state must exist");
        // Remember the request id so the matching NODES response is accepted.
        // Done in every send path so none can issue a FINDNODE unregistered.
        if let Message::FindNode(find_node) = &message {
            discv5
                .pending_findnodes
                .insert((node.node_id(), find_node.req_id.clone()), Instant::now());
        }
        let mut rng = OsRng;
        let masking_iv: u128 = rng.r#gen();
        let nonce = discv5.next_nonce(&mut rng);

        let packet = ordinary.encode(&nonce, masking_iv.to_be_bytes(), &encrypt_key)?;

        self.discv5_send_packet(&packet, &node.node_id(), node.udp_addr())
            .await?;
        let discv5 = self.discv5.as_mut().expect("discv5 state must exist");
        discv5
            .pending_by_nonce
            .insert(nonce, (node.clone(), message, Instant::now()));
        Ok(())
    }

    async fn discv5_resolve_outbound_key(
        &mut self,
        node_id: &H256,
        key: Option<[u8; 16]>,
    ) -> Result<[u8; 16], DiscoveryServerError> {
        if let Some(key) = key {
            return Ok(key);
        }
        match self.contacts.session(node_id) {
            Some(s) => Ok(s.outbound_key),
            None => {
                trace!(
                    protocol = "discv5",
                    node = %node_id,
                    "No session found in resolve_outbound_key, using zeroed key"
                );
                Ok([0; 16])
            }
        }
    }

    async fn discv5_send_ordinary_to(
        &mut self,
        message: Message,
        dest_id: &H256,
        addr: SocketAddr,
        encrypt_key: &[u8; 16],
    ) -> Result<(), DiscoveryServerError> {
        #[cfg(feature = "metrics")]
        {
            use ethrex_metrics::p2p::METRICS_P2P;
            METRICS_P2P.inc_discv5_outgoing(message.metric_label());
        }
        let discv5 = self.discv5.as_mut().expect("discv5 state must exist");
        // Remember the request id so the matching NODES response is accepted.
        // Done in every send path so none can issue a FINDNODE unregistered.
        if let Message::FindNode(find_node) = &message {
            discv5
                .pending_findnodes
                .insert((*dest_id, find_node.req_id.clone()), Instant::now());
        }
        let ordinary = Ordinary {
            src_id: self.local_node.node_id(),
            message,
        };

        let mut rng = OsRng;
        let masking_iv: u128 = rng.r#gen();
        let nonce = discv5.next_nonce(&mut rng);

        let packet = ordinary.encode(&nonce, masking_iv.to_be_bytes(), encrypt_key)?;

        self.discv5_send_packet(&packet, dest_id, addr).await?;
        Ok(())
    }

    async fn discv5_send_handshake(
        &mut self,
        message: Message,
        signature: Signature,
        eph_pubkey: &[u8],
        node: Node,
        record: Option<NodeRecord>,
    ) -> Result<(), DiscoveryServerError> {
        #[cfg(feature = "metrics")]
        {
            use ethrex_metrics::p2p::METRICS_P2P;
            METRICS_P2P.inc_discv5_outgoing("Handshake");
        }
        let handshake = Handshake {
            src_id: self.local_node.node_id(),
            id_signature: signature.serialize_compact().to_vec(),
            eph_pubkey: eph_pubkey.to_vec(),
            record,
            message: message.clone(),
        };
        let encrypt_key = match self.contacts.session(&node.node_id()) {
            Some(s) => s.outbound_key,
            None => {
                trace!(
                    protocol = "discv5",
                    node = %node.node_id(),
                    "No session found in send_handshake, using zeroed key"
                );
                [0; 16]
            }
        };

        let discv5 = self.discv5.as_mut().expect("discv5 state must exist");
        // Remember the request id so the matching NODES response is accepted.
        // Done in every send path so none can issue a FINDNODE unregistered.
        if let Message::FindNode(find_node) = &message {
            discv5
                .pending_findnodes
                .insert((node.node_id(), find_node.req_id.clone()), Instant::now());
        }
        let mut rng = OsRng;
        let masking_iv: u128 = rng.r#gen();
        let nonce = discv5.next_nonce(&mut rng);

        let packet = handshake.encode(&nonce, masking_iv.to_be_bytes(), &encrypt_key)?;

        self.discv5_send_packet(&packet, &node.node_id(), node.udp_addr())
            .await?;
        let discv5 = self.discv5.as_mut().expect("discv5 state must exist");
        discv5
            .pending_by_nonce
            .insert(nonce, (node, message, Instant::now()));
        Ok(())
    }

    pub async fn discv5_send_who_are_you(
        &mut self,
        nonce: [u8; 12],
        src_id: H256,
        addr: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        #[cfg(feature = "metrics")]
        {
            use ethrex_metrics::p2p::METRICS_P2P;
            METRICS_P2P.inc_discv5_outgoing("WhoAreYou");
        }
        let discv5 = self.discv5.as_mut().expect("discv5 state must exist");

        let rate_key = (addr.ip(), src_id);
        let now = Instant::now();

        // Global rate limit
        if now.duration_since(discv5.whoareyou_global_window_start) >= Duration::from_secs(1) {
            discv5.whoareyou_global_count = 0;
            discv5.whoareyou_global_window_start = now;
        }
        if discv5.whoareyou_global_count >= GLOBAL_WHOAREYOU_RATE_LIMIT {
            if discv5.whoareyou_global_count == GLOBAL_WHOAREYOU_RATE_LIMIT {
                discv5.whoareyou_global_count = GLOBAL_WHOAREYOU_RATE_LIMIT + 1;
                warn!(
                    protocol = "discv5",
                    "Global WHOAREYOU rate limit reached ({GLOBAL_WHOAREYOU_RATE_LIMIT}/s), \
                     dropping excess packets. This is normal during initial discovery or \
                     network churn; persistent occurrences may indicate a DoS attempt"
                );
            }
            return Ok(());
        }

        // Resend existing challenge if pending
        if let Some((_, _, raw_bytes)) = discv5.pending_challenges.get(&src_id) {
            trace!(
                protocol = "discv5",
                to = %src_id,
                %addr,
                "Resending existing WhoAreYou challenge"
            );
            self.udp_socket.send_to(raw_bytes, addr).await?;
            return Ok(());
        }

        // Per-(IP, node) rate limit
        if !IpPredictor::is_private_ip(addr.ip())
            && let Some(last_sent) = discv5.whoareyou_rate_limit.get(&rate_key)
            && now.duration_since(*last_sent) < WHOAREYOU_RATE_LIMIT
        {
            trace!(
                protocol = "discv5",
                to_ip = %addr.ip(),
                "Rate limiting WHOAREYOU packet (amplification attack prevention)"
            );
            return Ok(());
        }

        discv5.whoareyou_rate_limit.push(rate_key, now);
        discv5.whoareyou_global_count += 1;

        let mut rng = OsRng;

        let enr_seq = self
            .contacts
            .get_contact(&src_id)
            .map_or(0, |c| c.record.as_ref().map_or(0, |r| r.seq));

        let who_are_you = WhoAreYou {
            id_nonce: rng.r#gen(),
            enr_seq,
        };

        let masking_iv: u128 = rng.r#gen();
        let packet = who_are_you.encode(&nonce, masking_iv.to_be_bytes(), &[0; 16])?;

        let mut raw_buf = BytesMut::new();
        packet.encode(&mut raw_buf, &src_id)?;
        let raw_bytes = raw_buf.to_vec();

        let challenge_data = build_challenge_data(
            &masking_iv.to_be_bytes(),
            &packet.header.static_header,
            &packet.header.authdata,
        );
        let discv5 = self.discv5.as_mut().expect("discv5 state must exist");
        discv5
            .pending_challenges
            .insert(src_id, (challenge_data, Instant::now(), raw_bytes.clone()));

        self.udp_socket.send_to(&raw_bytes, addr).await?;
        trace!(protocol = "discv5", to = %src_id, %addr, flag = packet.header.flag, "Sent packet");

        Ok(())
    }

    async fn discv5_send_packet(
        &mut self,
        packet: &Packet,
        dest_id: &H256,
        addr: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        let mut buf = BytesMut::new();
        packet.encode(&mut buf, dest_id)?;
        self.udp_socket.send_to(&buf, addr).await?;
        trace!(protocol = "discv5", to = %dest_id, %addr, flag = packet.header.flag, "Sent packet");
        Ok(())
    }

    async fn discv5_handle_message(
        &mut self,
        ordinary: Ordinary,
        sender_addr: SocketAddr,
        outbound_key: Option<[u8; 16]>,
    ) -> Result<(), DiscoveryServerError> {
        let sender_id = ordinary.src_id;
        if sender_id == self.local_node.node_id() {
            return Ok(());
        }
        #[cfg(feature = "metrics")]
        {
            use ethrex_metrics::p2p::METRICS_P2P;
            METRICS_P2P.inc_discv5_incoming(ordinary.message.metric_label());
        }
        match ordinary.message {
            Message::Ping(ping_message) => {
                if ping_message.req_id.len() > 8 {
                    trace!(protocol = "discv5", from = %sender_id, "Dropping PING with oversized req_id");
                    return Ok(());
                }
                self.discv5_handle_ping(ping_message, sender_id, sender_addr, outbound_key)
                    .await?
            }
            Message::Pong(pong_message) => {
                self.discv5_handle_pong(pong_message, sender_id).await?;
            }
            Message::FindNode(find_node_message) => {
                if find_node_message.req_id.len() > 8 {
                    trace!(protocol = "discv5", from = %sender_id, "Dropping FINDNODE with oversized req_id");
                    return Ok(());
                }
                self.discv5_handle_find_node(
                    find_node_message,
                    sender_id,
                    sender_addr,
                    outbound_key,
                )
                .await?;
            }
            Message::Nodes(nodes_message) => {
                self.discv5_handle_nodes_message(nodes_message, sender_id)
                    .await?;
            }
            Message::TalkReq(talk_req_message) => {
                if talk_req_message.req_id.len() > 8 {
                    trace!(protocol = "discv5", from = %sender_id, "Dropping TALKREQ with oversized req_id");
                    return Ok(());
                }
                let talk_res = Message::TalkRes(TalkResMessage {
                    req_id: talk_req_message.req_id,
                    response: vec![],
                });
                let key = self
                    .discv5_resolve_outbound_key(&sender_id, outbound_key)
                    .await?;
                self.discv5_send_ordinary_to(talk_res, &sender_id, sender_addr, &key)
                    .await?;
            }
            Message::TalkRes(_talk_res_message) => (),
            Message::Ticket(_ticket_message) => (),
        }
        Ok(())
    }
}

fn generate_req_id() -> Bytes {
    let mut rng = OsRng;
    Bytes::from(rng.r#gen::<u64>().to_be_bytes().to_vec())
}

/// The log-distances to ask `node_id` for, to learn about nodes near
/// `target`: the distance between the two, and its neighbours.
///
/// FINDNODE distances count from the recipient, and the ones near
/// `log2(node_id ^ target)` are where it keeps whoever is close to `target`.
/// The result stays within `1..=256`: half of all random targets sit at 256,
/// which must not wrap, and 0 would return only the recipient's own record.
fn find_node_distances(target: &H256, node_id: &H256) -> Vec<u32> {
    let center = (distance(target, node_id) as u32).clamp(1, MAX_LOG_DISTANCE);
    let mut distances = vec![center];
    for step in 1..=u32::from(DISTANCES_PER_FIND_NODE_MSG / 2) {
        if center + step <= MAX_LOG_DISTANCE {
            distances.push(center + step);
        }
        if center > step {
            distances.push(center - step);
        }
    }
    distances
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discv5::server::SESSION_TTL;
    use crate::discv5::session::Session;
    use crate::peer_filter::AcceptAllFilter;
    use std::net::IpAddr;
    use std::sync::Arc;
    use tokio::net::UdpSocket;

    /// The peer's outbound key, which is our inbound key: what a packet from it is
    /// encrypted with and what the handler must look up to decrypt one.
    const PEER_KEY: [u8; 16] = [7; 16];
    const SESSION_IP: &str = "127.0.0.1";

    async fn server_with_session(session_ip: IpAddr) -> (DiscoveryServer, H256) {
        let local_node = Node::from_enode_url(
            "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303",
        ).expect("Bad enode url");
        let signer = SecretKey::new(&mut OsRng);
        let local_node_record = NodeRecord::from_node(&local_node, 1, &signer).unwrap();
        let mut server = DiscoveryServer::new_for_discv5_test(
            local_node,
            local_node_record,
            signer,
            Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap()),
            Box::new(AcceptAllFilter),
        );

        let peer_id = H256::repeat_byte(0xab);
        server.contacts_mut().set_session(
            peer_id,
            Session {
                outbound_key: [8; 16],
                inbound_key: PEER_KEY,
            },
        );
        server
            .discv5
            .as_mut()
            .expect("discv5 state must exist")
            .session_ips
            .insert(
                peer_id,
                SessionSource {
                    ip: session_ip,
                    last_used: Instant::now(),
                },
            );
        (server, peer_id)
    }

    /// An unsolicited NODES message: it reaches the refresh, then `discv5_handle_message`
    /// drops it for having no matching request, so the packet has no other effect.
    fn packet_from(peer_id: H256) -> Packet {
        Ordinary {
            src_id: peer_id,
            message: Message::Nodes(NodesMessage {
                req_id: Bytes::from_static(&[1, 2, 3]),
                total: 1,
                nodes: vec![],
            }),
        }
        .encode(&[0; 12], [0; 16], &PEER_KEY)
        .expect("failed to encode test packet")
    }

    fn age_session(server: &mut DiscoveryServer, by: Duration) {
        server.contacts_mut().age_sessions_for_test(by);
        for source in server
            .discv5
            .as_mut()
            .expect("discv5 state must exist")
            .session_ips
            .values_mut()
        {
            source.last_used -= by;
        }
    }

    /// Runs both halves' sweeps and reports which survived, as `(keys, session_ips)`.
    fn survives_sweeps(server: &mut DiscoveryServer, peer_id: &H256) -> (bool, bool) {
        server.contacts_mut().prune();
        let keys = server.contacts_mut().session(peer_id).is_some();
        let discv5 = server.discv5.as_mut().expect("discv5 state must exist");
        discv5.cleanup_stale_entries();
        (keys, discv5.session_ips.contains_key(peer_id))
    }

    #[tokio::test]
    async fn a_packet_that_decrypts_refreshes_both_halves_of_the_session() {
        // The halves must move together. Keys that outlive their `session_ips` entry
        // silently lose the IP-rebinding check, since a missing entry reads as nothing
        // to compare against.
        let (mut server, peer_id) = server_with_session(SESSION_IP.parse().unwrap()).await;
        age_session(&mut server, SESSION_TTL);

        server
            .discv5_handle_ordinary(
                packet_from(peer_id),
                format!("{SESSION_IP}:30304").parse().unwrap(),
            )
            .await
            .expect("handling an unsolicited NODES packet should not fail");

        assert_eq!(
            survives_sweeps(&mut server, &peer_id),
            (true, true),
            "using a session must keep both its keys and its IP guard alive"
        );
    }

    #[test]
    fn find_node_distances_stay_within_the_log_distance_range() {
        let target = H256::zero();
        // Top bit differs: log-distance 256, which used to wrap to 0 as a u8.
        let mut far = H256::zero();
        far.0[0] = 0x80;
        assert_eq!(find_node_distances(&target, &far), vec![256, 255]);
        // The recipient is the target: nothing to ask for at distance 0.
        assert_eq!(find_node_distances(&target, &target), vec![1, 2]);
        assert_eq!(
            find_node_distances(&target, &H256::from_low_u64_be(0b1000)),
            vec![4, 5, 3]
        );
    }

    #[tokio::test]
    async fn a_nodes_response_reaches_only_the_lookup_that_asked() {
        let (mut server, peer_id) = server_with_session(SESSION_IP.parse().unwrap()).await;
        let discv5 = server.discv5.as_mut().expect("discv5 state must exist");
        let asked = discv5.next_lookup_id();
        let other = discv5.next_lookup_id();
        // Seeded with one node each so neither lookup is empty; the response
        // below carries a node neither has seen.
        let seed = |byte| {
            let node = Node::new(
                SESSION_IP.parse().unwrap(),
                30303,
                30303,
                H512::repeat_byte(byte),
            );
            vec![(node.node_id(), node)]
        };
        discv5
            .active_lookups
            .push((asked, IterativeLookup::new(H256::zero(), seed(1))));
        discv5
            .active_lookups
            .push((other, IterativeLookup::new(H256::zero(), seed(2))));
        let req_id = Bytes::from_static(&[9]);
        discv5
            .pending_findnodes
            .insert((peer_id, req_id.clone()), Instant::now());
        discv5
            .findnode_owners
            .insert((peer_id, req_id.clone()), asked);

        let signer = SecretKey::new(&mut OsRng);
        let pubkey = crate::utils::public_key_from_signing_key(&signer);
        let found = Node::new(SESSION_IP.parse().unwrap(), 30305, 30305, pubkey);
        let record = NodeRecord::from_node(&found, 1, &signer).unwrap();
        server
            .discv5_handle_nodes_message(
                NodesMessage {
                    req_id,
                    total: 1,
                    nodes: vec![record],
                },
                peer_id,
            )
            .await
            .unwrap();

        let discv5 = server.discv5.as_mut().expect("discv5 state must exist");
        let mut next = |id| {
            let (_, lookup) = discv5
                .active_lookups
                .iter_mut()
                .find(|(lookup_id, _)| *lookup_id == id)
                .unwrap();
            lookup
                .next_to_query(LOOKUP_BUCKET_SIZE)
                .into_iter()
                .map(|(node_id, _)| node_id)
                .collect::<Vec<_>>()
        };
        assert!(next(asked).contains(&found.node_id()));
        assert!(!next(other).contains(&found.node_id()));
    }

    #[tokio::test]
    async fn a_packet_from_another_ip_refreshes_nothing() {
        // The rebinding check answers this one with a WHOAREYOU. Refreshing here would
        // let whoever can reach us from another address hold a session open forever.
        let (mut server, peer_id) = server_with_session(SESSION_IP.parse().unwrap()).await;
        age_session(&mut server, SESSION_TTL);

        // The WHOAREYOU reply goes out over a real socket to a port with no listener;
        // whether that send reports an error is irrelevant to what is asserted here.
        let _ = server
            .discv5_handle_ordinary(packet_from(peer_id), "127.0.0.2:30304".parse().unwrap())
            .await;

        assert_eq!(
            survives_sweeps(&mut server, &peer_id),
            (false, false),
            "an aged session used from the wrong address is still reaped"
        );
    }
}
