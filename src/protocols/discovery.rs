//! Discovery protocol for management of network addresses.

use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    watch::{Sender, Watch},
};
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

#[derive(Clone)]
enum State {
    Unassigned,
    Assigned(Address),
}

pub struct Client<P: PacketPipe, const N: usize = 1> {
    state: Watch<NoopRawMutex, State, N>,
    pipe: P,
    protocol_id: u8,
    hardware_address: HardwareAddress,
}

struct ClientCore<'a, P: PacketPipe, const N: usize> {
    state: Sender<'a, NoopRawMutex, State, N>,
    pipe: &'a mut P,
    protocol_id: u8,
    hardware_address: &'a HardwareAddress,
}

pub struct ClientView<'a, const N: usize> {
    state: &'a Watch<NoopRawMutex, State, N>,
}

impl<P: PacketPipe, const N: usize> Client<P, N> {
    pub fn new(pipe: P, protocol_id: u8, hardware_address: HardwareAddress) -> Self {
        let state = Watch::new();
        state.sender().send(State::Unassigned);

        Self {
            state,
            protocol_id,
            hardware_address,
            pipe,
        }
    }

    pub fn run(&mut self) -> (impl Future<Output = ()>, ClientView<'_, N>) {
        let task = async {
            let mut core = ClientCore {
                state: self.state.sender(),
                pipe: &mut self.pipe,
                protocol_id: self.protocol_id,
                hardware_address: &self.hardware_address,
            };

            core.run().await
        };

        (task, ClientView { state: &self.state })
    }
}

impl<'a, P: PacketPipe, const N: usize> ClientCore<'a, P, N> {
    fn our_address(&self) -> Option<Address> {
        match unwrap!(self.state.try_get()) {
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
                self.state.send(State::Unassigned);
            }
            Packet::WhoHas(hardware_address) => {
                if hardware_address == self.hardware_address {
                    match unwrap!(self.state.try_get()) {
                        State::Assigned(_) => {
                            let _ = self
                                .send_packet(
                                    &Packet::IHave(self.hardware_address.clone()),
                                    header.src(),
                                )
                                .await;
                        }
                        _ => {}
                    }
                } else {
                    // Ignore
                }
            }
            Packet::IHave(_) | Packet::Request(_) => {
                // Ignore
            }
            Packet::Assign(hardware_address, address) => {
                if hardware_address == self.hardware_address {
                    self.state.send(State::Assigned(*address));
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

    pub async fn run(&mut self) {
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

impl<'a, const N: usize> ClientView<'a, N> {
    pub fn state(&self) -> Option<Address> {
        match self.state.try_get() {
            Some(State::Assigned(address)) => Some(address),
            _ => None,
        }
    }
}
