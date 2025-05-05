pub mod discovery;

trait PacketPipe {
    const MTU: usize;
    type Error;
    async fn receive(&mut self, rx_packet: &mut [u8]) -> Result<usize, Self::Error>;
    async fn send(&mut self, tx_packet: &mut [u8]) -> Result<(), Self::Error>;
}
