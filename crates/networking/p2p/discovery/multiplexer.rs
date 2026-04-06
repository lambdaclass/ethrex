//! Discovery protocol multiplexer that routes packets between discv4 and discv5.

use std::{net::SocketAddr, sync::Arc};

use bytes::BytesMut;
use ethrex_common::{H256, utils::keccak};
use futures::StreamExt;
use spawned_concurrency::{
    actor,
    error::ActorError,
    protocol,
    tasks::{Actor, ActorRef, Context, Handler, send_message_on, spawn_listener},
};
use thiserror::Error;
use tokio::net::UdpSocket;
use tokio_util::udp::UdpFramed;
use tracing::{debug, info};

use super::codec::DiscriminatingCodec;
use crate::discv4::{
    messages::Packet as Discv4Packet,
    server::{DiscoveryServer as Discv4Server, Discv4Message, discv4_server_protocol},
};

use crate::discv5::{
    messages::Packet as Discv5Packet,
    server::{DiscoveryServer as Discv5Server, Discv5Message, discv5_server_protocol},
};

/// Minimum packet size for a valid discv4 packet.
/// hash (32) + signature (65) + type (1) = 98 bytes
const DISCV4_MIN_PACKET_SIZE: usize = 98;

/// Configuration for which discovery protocols to enable.
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    pub discv4_enabled: bool,
    pub discv5_enabled: bool,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            discv4_enabled: true,
            discv5_enabled: true,
        }
    }
}

#[protocol]
#[allow(dead_code)]
pub trait DiscoveryMultiplexerProtocol: Send + Sync {
    fn raw_packet(&self, data: BytesMut, from: SocketAddr) -> Result<(), ActorError>;
    fn shutdown(&self) -> Result<(), ActorError>;
}

/// The discovery multiplexer manages a shared UDP socket and routes packets
/// to the appropriate discovery protocol handler (discv4 or discv5).
pub struct DiscoveryMultiplexer {
    udp_socket: Arc<UdpSocket>,
    local_node_id: H256,
    config: DiscoveryConfig,
    discv4_handle: Option<ActorRef<Discv4Server>>,
    discv5_handle: Option<ActorRef<Discv5Server>>,
}

#[derive(Debug, Error)]
pub enum DiscoveryMultiplexerError {
    #[error("Internal actor error: {0}")]
    ActorError(#[from] ActorError),
}

/// Check if a packet is a discv4 packet by verifying the hash.
/// DiscV4 packets have structure: hash (32 bytes) || signature (65 bytes) || type (1 byte) || data
/// where hash == keccak256(rest_of_packet)
pub fn is_discv4_packet(data: &[u8]) -> bool {
    if data.len() < DISCV4_MIN_PACKET_SIZE {
        return false;
    }

    let packet_hash = &data[0..32];
    let computed_hash = keccak(&data[32..]);

    packet_hash == computed_hash.as_bytes()
}

#[actor(protocol = DiscoveryMultiplexerProtocol)]
impl DiscoveryMultiplexer {
    /// Create a new discovery multiplexer.
    pub fn new(
        udp_socket: Arc<UdpSocket>,
        local_node_id: H256,
        config: DiscoveryConfig,
        discv4_handle: Option<ActorRef<Discv4Server>>,
        discv5_handle: Option<ActorRef<Discv5Server>>,
    ) -> Self {
        Self {
            udp_socket,
            local_node_id,
            config,
            discv4_handle,
            discv5_handle,
        }
    }

    #[started]
    async fn started(&mut self, ctx: &Context<Self>) {
        let local_addr = self.udp_socket.local_addr();
        info!(
            local_addr=?local_addr,
            discv4_enabled=self.config.discv4_enabled,
            discv5_enabled=self.config.discv5_enabled,
            "Discovery multiplexer started, listening for UDP packets"
        );
        // Set up the UDP listener using the discriminating codec
        let stream = UdpFramed::new(self.udp_socket.clone(), DiscriminatingCodec::new());

        spawn_listener(
            ctx.clone(),
            stream.filter_map(|result| async move {
                match result {
                    Ok((data, from)) => {
                        Some(discovery_multiplexer_protocol::RawPacket { data, from })
                    }
                    Err(e) => {
                        debug!(error=?e, "Error receiving packet in multiplexer");
                        None
                    }
                }
            }),
        );

        // Set up shutdown handler
        send_message_on(
            ctx.clone(),
            tokio::signal::ctrl_c(),
            discovery_multiplexer_protocol::Shutdown,
        );
    }

    #[send_handler]
    async fn handle_raw_packet(
        &mut self,
        msg: discovery_multiplexer_protocol::RawPacket,
        _ctx: &Context<Self>,
    ) {
        self.route_packet(&msg.data, msg.from);
    }

    #[send_handler]
    async fn handle_shutdown(
        &mut self,
        _msg: discovery_multiplexer_protocol::Shutdown,
        ctx: &Context<Self>,
    ) {
        ctx.stop();
    }

    /// Route a packet to the appropriate protocol handler.
    fn route_packet(&mut self, data: &[u8], from: SocketAddr) {
        if is_discv4_packet(data) {
            self.route_to_discv4(data, from);
        } else {
            self.route_to_discv5(data, from);
        }
    }

    /// Route a packet to the discv4 handler.
    fn route_to_discv4(&mut self, data: &[u8], from: SocketAddr) {
        if !self.config.discv4_enabled {
            return;
        }

        let Some(handle) = &self.discv4_handle else {
            return;
        };

        // Decode the discv4 packet
        match Discv4Packet::decode(data) {
            Ok(packet) => {
                let msg = Discv4Message::from(packet, from);
                if let Err(e) = handle.send(discv4_server_protocol::RecvMessage {
                    message: Box::new(msg),
                }) {
                    debug!(error=?e, "Failed to send discv4 message to handler");
                }
            }
            Err(e) => {
                debug!(error=?e, "Failed to decode discv4 packet");
            }
        }
    }

    /// Route a packet to the discv5 handler.
    fn route_to_discv5(&mut self, data: &[u8], from: SocketAddr) {
        if !self.config.discv5_enabled {
            return;
        }

        let Some(handle) = &self.discv5_handle else {
            return;
        };

        // Decode the discv5 packet
        match Discv5Packet::decode(&self.local_node_id, data) {
            Ok(packet) => {
                let msg = Discv5Message::from(packet, from);
                if let Err(e) = handle.send(discv5_server_protocol::RecvMessage {
                    message: Box::new(msg),
                }) {
                    debug!(error=?e, "Failed to send discv5 message to handler");
                }
            }
            Err(e) => {
                debug!(error=?e, "Failed to decode discv5 packet");
            }
        }
    }
}
