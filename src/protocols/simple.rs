use core::ops::Deref;

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::routing::Header;

use super::{PackageError, Packageable, PacketPipe};

#[derive(Debug, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Packet<T>(T);

impl<T> Packet<T> {
    pub const fn new(t: T) -> Self {
        Self(t)
    }
}

impl<T: DeserializeOwned> Packet<T> {
    pub fn parse(buf: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes::<Self>(buf)
    }
}

impl<T: Serialize> Packet<T> {
    pub fn to_bytes<'a>(&self, tx_body: &'a mut [u8]) -> Result<&'a mut [u8], postcard::Error> {
        postcard::to_slice(self, tx_body)
    }
}

impl<T: Serialize + MaxSize> Packageable for Packet<T> {
    type Error = PackageError;
    const MAX_SIZE: usize = Self::POSTCARD_MAX_SIZE;

    fn package(&self, packet_body: &mut [u8]) -> Result<usize, Self::Error> {
        Ok(self.to_bytes(packet_body)?.len())
    }
}

impl<T> Deref for Packet<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct Socket<P, H> {
    pipe: P,
    handler: H,
}

pub trait PacketHandler {
    type Data: MaxSize + Serialize + DeserializeOwned;
    fn handle_packet<P: PacketPipe>(
        &mut self,
        pipe: &mut P,
        header: Header,
        packet: Packet<Self::Data>,
    ) -> impl Future<Output = ()> + Send;
    fn handle_error<P: PacketPipe>(
        &mut self,
        pipe: &mut P,
        error: P::Error,
    ) -> impl Future<Output = ()> + Send;
}

impl<P: PacketPipe, H: PacketHandler> Socket<P, H> {
    pub fn new(pipe: P, handler: H) -> Self {
        Self { pipe, handler }
    }

    pub const fn mtu() -> usize {
        Packet::<H::Data>::POSTCARD_MAX_SIZE
    }

    pub async fn run<const MTU: usize>(&mut self) {
        const { core::assert!(Self::mtu() <= MTU) }

        let mut rx_body = [0u8; MTU];
        loop {
            match self.pipe.receive(&mut rx_body).await {
                Ok((header, size)) => match Packet::parse(&rx_body[..size]) {
                    Ok(packet) => {
                        self.handler
                            .handle_packet(&mut self.pipe, header, packet)
                            .await
                    }
                    Err(_e) => {
                        error!("Got malformed packet");
                    }
                },
                Err(e) => self.handler.handle_error(&mut self.pipe, e).await,
            }
        }
    }
}
