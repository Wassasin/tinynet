//! Discovery protocol for management of network addresses.

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

use crate::routing::{ADDRESS_MULTICAST, Address, Header, HeaderBuilder, TTL_UNLIMITED};

use super::PacketPipe;

const MTU: usize = Packet::POSTCARD_MAX_SIZE;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize, MaxSize)]
pub struct HardwareAddress(pub [u8; 8]);

#[derive(Serialize, Deserialize, MaxSize)]
pub enum Packet {
    /// Broadcast directive to all nodes to un-assign assigned addresses.
    UnassignAll,
    /// Broadcast question, respond with IHave targetted to sender.
    WhoHas(HardwareAddress),
    /// Non-broadcast reply.
    IHave(HardwareAddress),
    /// Broadcast question to fetch an hardware address.
    Request(HardwareAddress),
    /// Request to set an address by the master of the network.
    Assign(HardwareAddress, Address),
}

enum State {
    Unassigned,
    Assigned(Address),
}

pub struct Client<P: PacketPipe> {
    protocol_id: u8,
    hardware_address: HardwareAddress,
    state: State,
    pipe: P,
}

impl<P: PacketPipe> Client<P> {
    pub fn new(pipe: P, protocol_id: u8, hardware_address: HardwareAddress) -> Self {
        Self {
            protocol_id,
            hardware_address,
            state: State::Unassigned,
            pipe,
        }
    }

    fn our_address(&self) -> Option<Address> {
        match self.state {
            State::Unassigned => None,
            State::Assigned(address) => Some(address),
        }
    }

    fn handle(&mut self, result: Result<(), P::Error>) {
        if let Err(e) = result {
            warn!("Underlying pipe error with {}", e);
            // TODO maybe change state?
        }
    }

    async fn send_packet(&mut self, packet: &Packet, dest: Address) {
        let our_address = self.our_address().unwrap_or(ADDRESS_MULTICAST);

        let mut tx_body = [0u8; MTU];
        let tx_body = unwrap!(postcard::to_slice(packet, &mut tx_body));

        let result = self
            .pipe
            .send(
                &HeaderBuilder::new()
                    .with_dst(dest)
                    .with_src(our_address)
                    .with_ttl(TTL_UNLIMITED)
                    .with_protocol(self.protocol_id)
                    .build(),
                tx_body,
            )
            .await;

        self.handle(result)
    }

    async fn handle_packet(&mut self, header: &Header, packet: &Packet) {
        match packet {
            Packet::UnassignAll => {
                info!("Address unassigned");
                self.state = State::Unassigned;
            }
            Packet::WhoHas(hardware_address) => {
                if *hardware_address == self.hardware_address {
                    let _ = self
                        .send_packet(&Packet::IHave(self.hardware_address.clone()), header.src())
                        .await;
                } else {
                    // Ignore
                }
            }
            Packet::IHave(_) | Packet::Request(_) => {
                // Ignore
            }
            Packet::Assign(hardware_address, address) => {
                if *hardware_address == self.hardware_address {
                    self.state = State::Assigned(*address);
                    info!("Got assigned address {}", address);

                    let _ = self
                        .send_packet(&Packet::IHave(self.hardware_address.clone()), header.src())
                        .await;
                } else {
                    // Ignore
                }
            }
        }
    }

    pub async fn run(&mut self) -> ! {
        let mut rx_body = [0u8; MTU];
        loop {
            match self.pipe.receive(&mut rx_body).await {
                Ok((header, size)) => match postcard::from_bytes::<Packet>(&rx_body[..size]) {
                    Ok(packet) => self.handle_packet(&header, &packet).await,
                    Err(e) => {
                        error!("Got malformed packet: {}", e);
                    }
                },
                Err(e) => self.handle(Err(e)),
            }
        }
    }
}
