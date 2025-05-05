//! Discovery protocol for management of network addresses.

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

use crate::routing::Address;

use super::PacketPipe;

const MTU: usize = Packet::POSTCARD_MAX_SIZE;

#[derive(Serialize, Deserialize, MaxSize)]
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
}

struct Client<P: PacketPipe> {
    hardware_address: HardwareAddress,
    state: State,
    pipe: P,
}

impl<P: PacketPipe> Client<P> {
    pub fn new(pipe: P, hardware_address: HardwareAddress) -> Self {
        Self {
            hardware_address,
            state: State::Unassigned,
            pipe,
        }
    }

    async fn handle_packet(&mut self, packet: &Packet) {
        match packet {
            Packet::UnassignAll => self.state = State::Unassigned,
            Packet::WhoHas(hardware_address) => todo!(),
            Packet::IHave(hardware_address) => todo!(),
            Packet::Request(hardware_address) => todo!(),
            Packet::Assign(hardware_address, address) => todo!(),
        }
    }

    pub async fn run(&mut self) -> ! {
        let mut rx_packet = [0u8; MTU];
        loop {
            match self.pipe.receive(&mut rx_packet).await {
                Ok(size) => match postcard::from_bytes::<Packet>(&rx_packet[..size]) {
                    Ok(packet) => self.handle_packet(&packet).await,
                    Err(e) => todo!(),
                },
                Err(e) => {
                    todo!()
                }
            }
        }
    }
}
