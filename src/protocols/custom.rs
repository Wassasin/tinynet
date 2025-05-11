use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::Packageable;

#[derive(Debug, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Packet<T>(T);

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
    type Error = postcard::Error;
    const MAX_SIZE: usize = Self::POSTCARD_MAX_SIZE;

    fn package(&self, packet_body: &mut [u8]) -> Result<usize, Self::Error> {
        Ok(self.to_bytes(packet_body)?.len())
    }
}
