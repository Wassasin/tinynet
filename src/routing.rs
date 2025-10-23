//! Packet-based embedded protocol providing discovery, assignment and routing.
//!
//! Does not provide reliability.

use core::ops::{BitOrAssign, RangeInclusive};

use bitfields::bitfield;
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

pub const NUMBER_OF_ADDRESSES: usize = 2usize.pow(12);

#[derive(
    Default, Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Serialize, Deserialize, MaxSize,
)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Address(u16);

pub const ADDRESS_MULTICAST: Address = Address(0x000);
pub const ADDRESS_FIRST: Address = Address(0x001);
pub const ADDRESS_LAST: Address = Address((NUMBER_OF_ADDRESSES - 1) as u16);
pub const ADDRESSES_VALID: RangeInclusive<Address> = ADDRESS_MULTICAST..=ADDRESS_LAST;

pub const TTL_UNLIMITED: u8 = 0b0000;
pub const TTL_DEFAULT: u8 = 0b1000;

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct AddressOutOfRange;

impl TryFrom<u16> for Address {
    type Error = AddressOutOfRange;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Address::try_from(value)
    }
}

impl Address {
    const fn try_from(value: u16) -> Result<Self, AddressOutOfRange> {
        if ADDRESSES_VALID.start().0 >= value && ADDRESSES_VALID.end().0 <= value {
            Ok(Address(value))
        } else {
            Err(AddressOutOfRange)
        }
    }

    const fn from_bits(value: u16) -> Self {
        match Address::try_from(value) {
            Ok(value) => value,
            Err(_) => {
                core::panic!("Address does not fit");
            }
        }
    }

    const fn into_bits(self) -> u16 {
        self.0
    }

    pub fn generate_all() -> impl Iterator<Item = Address> {
        (ADDRESS_FIRST.0..=ADDRESS_LAST.0).map(Address::from_bits)
    }
}

impl From<Address> for u16 {
    fn from(value: Address) -> Self {
        value.0
    }
}

#[bitfield(u32)]
#[derive(Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Header {
    #[bits(4)]
    protocol: u8,
    #[bits(4)]
    ttl: u8,
    #[bits(12)]
    src: Address,
    #[bits(12)]
    dst: Address,
}

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PacketTooShort;

impl Header {
    pub fn parse(packet: &[u8]) -> Result<(Header, &[u8]), PacketTooShort> {
        let (header, data) = packet.split_at_checked(4).ok_or(PacketTooShort)?;
        // Note(unwrap): we just ensured that the size matches.
        let header = u32::from_le_bytes(unwrap!(header.try_into()));
        Ok((Header::from_bits(header), data))
    }

    pub const fn to_bytes(self) -> [u8; 4] {
        self.into_bits().to_le_bytes()
    }
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PortId(u8);

impl From<u8> for PortId {
    fn from(value: u8) -> Self {
        PortId(value)
    }
}

impl From<PortId> for u8 {
    fn from(value: PortId) -> Self {
        value.0
    }
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PortSet(u8);

impl From<PortId> for PortSet {
    fn from(value: PortId) -> Self {
        PortSet(1 << value.0)
    }
}

impl BitOrAssign for PortSet {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0
    }
}

impl BitOrAssign<PortId> for PortSet {
    fn bitor_assign(&mut self, rhs: PortId) {
        self.bitor_assign(PortSet::from(rhs))
    }
}

impl PortSet {
    pub const fn empty() -> Self {
        PortSet(0)
    }

    pub fn is_empty(&self) -> bool {
        self.0 == Self::empty().0
    }

    pub fn remove(&mut self, port: PortId) {
        self.0 &= !(PortSet::from(port).0)
    }

    pub fn contains(&self, port: PortId) -> bool {
        self.0 & PortSet::from(port).0 != 0x00
    }
}

impl IntoIterator for PortSet {
    type Item = PortId;
    type IntoIter = PortSetIterator;

    fn into_iter(self) -> Self::IntoIter {
        PortSetIterator {
            set: self.0,
            index: 0,
        }
    }
}

pub struct PortSetIterator {
    set: u8,
    index: u8,
}

impl Iterator for PortSetIterator {
    type Item = PortId;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.set == 0u8 {
                return None;
            }

            let result = (self.set & 0b1 == 0b1).then_some(PortId(self.index));

            self.set >>= 1;
            self.index += 1;

            if result.is_some() {
                return result;
            }
        }
    }
}

pub struct RoutingTable<const TABLE_SIZE: usize> {
    table: [PortSet; TABLE_SIZE],
}

impl<const TABLE_SIZE: usize> RoutingTable<TABLE_SIZE> {
    pub const fn new() -> Self {
        Self {
            table: [PortSet::empty(); TABLE_SIZE],
        }
    }

    pub fn add_route(&mut self, port: PortId, address: Address) {
        let port = PortSet::from(port);
        self.table[address.0 as usize] |= port;
    }

    pub fn get_route(&self, address: Address) -> PortSet {
        self.table[address.0 as usize]
    }
}

impl<const TABLE_SIZE: usize> Default for RoutingTable<TABLE_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}
