use crate::routing::Header;

pub mod discovery;

#[allow(async_fn_in_trait)]
pub trait PacketPipe {
    const MTU: usize;
    type Error;

    /// Await until a full packet is received.
    ///
    /// Must be cancel-safe.
    async fn receive(&mut self, rx_body: &mut [u8]) -> Result<(Header, usize), Self::Error>;

    /// Send a full packet.
    async fn send(&mut self, tx_header: &Header, tx_packet: &mut [u8]) -> Result<(), Self::Error>;
}
