//! Discovery protocol for management of network addresses.

use core::fmt::{Display, Write};

use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    watch::{Receiver, Sender, Watch},
};
use embassy_time::{Duration, Instant, Timer};
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

use crate::routing::{ADDRESS_MULTICAST, Address, Header};

use super::{Packageable, PacketPipe};

const MTU: usize = Packet::POSTCARD_MAX_SIZE;
const REQUEST_PERIOD: Duration = Duration::from_secs(10);

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, MaxSize, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct HardwareAddress(pub [u8; 8]);

impl Display for HardwareAddress {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        for v in self.0.iter() {
            let lo = v & 0x0f;
            let hi = (v & 0xf0) >> 4;
            f.write_char(HEX[hi as usize] as char)?;
            f.write_char(HEX[lo as usize] as char)?;
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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
    Assign {
        /// Representation of our globally unique hardware address assigned at factory.
        hardware_address: HardwareAddress,
        /// Network address that we got assigned.
        address: Address,
        /// Time to live, in seconds.
        ttl_seconds: u32,
    },
}

impl Packet {
    pub fn parse(buf: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes::<Packet>(buf)
    }

    pub fn to_bytes<'a>(&self, tx_body: &'a mut [u8]) -> Result<&'a mut [u8], postcard::Error> {
        postcard::to_slice(self, tx_body)
    }
}

impl Packageable for Packet {
    type Error = postcard::Error;
    const MAX_SIZE: usize = Packet::POSTCARD_MAX_SIZE;

    fn package(&self, packet_body: &mut [u8]) -> Result<usize, Self::Error> {
        Ok(self.to_bytes(packet_body)?.len())
    }
}

#[derive(Clone)]
enum State {
    Unassigned,
    Assigned { address: Address, until: Instant },
}

pub struct Client<P: PacketPipe> {
    state: Watch<NoopRawMutex, State, 1>,
    pipe: P,
    hardware_address: HardwareAddress,
}

struct ClientCore<'a, P: PacketPipe> {
    state: Sender<'a, NoopRawMutex, State, 1>,
    pipe: &'a mut P,
    hardware_address: &'a HardwareAddress,
}

pub struct ClientView<'a> {
    state: Receiver<'a, NoopRawMutex, State, 1>,
}

impl<P: PacketPipe> Client<P> {
    pub fn new(pipe: P, hardware_address: HardwareAddress) -> Self {
        let state = Watch::new();
        state.sender().send(State::Unassigned);

        Self {
            state,
            hardware_address,
            pipe,
        }
    }

    pub fn run(&mut self) -> (impl Future<Output = ()>, ClientView<'_>) {
        let task = async {
            let mut core = ClientCore {
                state: self.state.sender(),
                pipe: &mut self.pipe,
                hardware_address: &self.hardware_address,
            };

            core.run().await
        };

        (
            task,
            ClientView {
                // Note(unwrap): got exactly 1 receiver.
                state: unwrap!(self.state.receiver()),
            },
        )
    }
}

impl<'a, P: PacketPipe> ClientCore<'a, P> {
    /// Convenience function to get state.
    fn state(&self) -> State {
        // Note(unwrap): self.state is guaranteed to be sent at construction.
        unwrap!(self.state.try_get())
    }

    fn handle(&mut self, result: Result<(), P::Error>) {
        if let Err(e) = result {
            warn!("Underlying pipe error with {:?}", e);
            // TODO maybe change state?
        }
    }

    async fn send_packet(&mut self, packet: &Packet, dest: Address) {
        let mut tx_body = [0u8; MTU];
        // Note(unwrap): to_bytes is infallible due to MaxSize.
        let tx_body = unwrap!(packet.to_bytes(&mut tx_body));

        let result = self.pipe.send(dest, tx_body).await;

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
                    match self.state() {
                        State::Assigned { .. } => {
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
            Packet::Assign {
                hardware_address,
                address,
                ttl_seconds,
            } => {
                if hardware_address == self.hardware_address {
                    self.state.send(State::Assigned {
                        address: *address,
                        until: Instant::now() + Duration::from_secs(*ttl_seconds as u64),
                    });
                    info!("Got assigned address {:?}", address);

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
        let mut last_request: Option<Instant> = None;
        let mut rx_body = [0u8; MTU];
        loop {
            let deadline_fut = {
                let state = self.state();
                async move {
                    match state {
                        State::Unassigned => {
                            if let Some(last_request) = last_request {
                                Timer::at(last_request + REQUEST_PERIOD).await
                            } else {
                                // Immediately return
                            };
                        }
                        State::Assigned { until, .. } => Timer::at(until).await,
                    }
                }
            };
            let receive_fut = self.pipe.receive(&mut rx_body);

            match embassy_futures::select::select(receive_fut, deadline_fut).await {
                embassy_futures::select::Either::First(tup) => match tup {
                    Ok((header, size)) => match Packet::parse(&rx_body[..size]) {
                        Ok(packet) => self.handle_packet(&header, &packet).await,
                        Err(e) => {
                            error!("Got malformed packet: {}", e);
                        }
                    },
                    Err(e) => self.handle(Err(e)),
                },
                embassy_futures::select::Either::Second(()) => {
                    last_request = Some(Instant::now());
                    self.send_packet(
                        &Packet::Request(self.hardware_address.clone()),
                        ADDRESS_MULTICAST,
                    )
                    .await
                }
            }
        }
    }
}

impl<'a> ClientView<'a> {
    pub fn address(&mut self) -> Option<Address> {
        match self.state.try_get() {
            Some(State::Assigned { address, .. }) => Some(address),
            _ => None,
        }
    }

    pub async fn wait_for_changed_address(&mut self) -> Option<Address> {
        let result = self.state.changed().await;

        match result {
            State::Unassigned => None,
            State::Assigned { address, .. } => Some(address),
        }
    }
}
