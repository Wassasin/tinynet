use crate::routing::{Address, Header};
use core::fmt::Debug;

pub mod discovery;
pub mod simple;

#[allow(async_fn_in_trait)]
pub trait PacketPipe {
    const MTU: usize;

    #[cfg(feature = "defmt")]
    type Error: defmt::Format + Debug;
    #[cfg(not(feature = "defmt"))]
    type Error: Debug;

    /// Await until a full packet is received.
    ///
    /// Must be cancel-safe.
    async fn receive(&mut self, rx_body: &mut [u8]) -> Result<(Header, usize), Self::Error>;

    /// Send a full packet.
    async fn send(&mut self, dest: Address, tx_packet: &[u8]) -> Result<(), Self::Error>;
}

pub trait Packageable {
    #[cfg(feature = "defmt")]
    type Error: defmt::Format + Debug;
    #[cfg(not(feature = "defmt"))]
    type Error: Debug;

    const MAX_SIZE: usize;

    /// Write the current packet into `packet_body`, returning length of the slice that was written.
    fn package(&self, packet_body: &mut [u8]) -> Result<usize, Self::Error>;
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PackageError;

impl From<postcard::Error> for PackageError {
    fn from(_value: postcard::Error) -> Self {
        PackageError
    }
}
