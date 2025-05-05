//! Packet-based embedded protocol providing discovery, assignment and routing.
//!
//! Does not provide reliability.

use core::ops::BitOrAssign;

use bitfields::bitfield;
use postcard::experimental::max_size::MaxSize;
use scapegoat::SgMap;
use serde::{Deserialize, Serialize};

#[derive(
    Default, Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Serialize, Deserialize, MaxSize,
)]
pub struct Address(u16);
pub const ADDRESS_MULTICAST: Address = Address(0x000);

pub const NUMBER_OF_ADDRESSES: usize = 2usize.pow(12);

#[derive(Debug)]
pub struct AddressOutOfRange;

impl TryFrom<u16> for Address {
    type Error = AddressOutOfRange;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Address::try_from(value)
    }
}

impl Address {
    const fn try_from(value: u16) -> Result<Self, AddressOutOfRange> {
        if value < ADDRESS_MULTICAST.0 {
            Ok(Address(value))
        } else {
            Err(AddressOutOfRange)
        }
    }

    const fn from_bits(value: u16) -> Self {
        match Address::try_from(value) {
            Ok(value) => value,
            Err(_) => panic!(),
        }
    }

    const fn into_bits(self) -> u16 {
        self.0
    }
}

#[bitfield(u32)]
struct Header {
    #[bits(4)]
    protocol: u8,
    #[bits(4)]
    ttl: u8,
    #[bits(12)]
    src: Address,
    #[bits(12)]
    dst: Address,
}

pub struct PortId(u8);

#[derive(Default, Clone, Copy)]
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

impl PortSet {
    pub const fn empty() -> Self {
        PortSet(0)
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
        if self.set == 0u8 {
            return None;
        }

        while self.set & 0b1 != 0b1 {
            self.set >>= 1;
            self.index += 1;
        }

        Some(PortId(self.index))
    }
}

pub struct Router<const TABLE_SIZE: usize> {
    table: SgMap<Address, PortSet, TABLE_SIZE>,
}

impl<const TABLE_SIZE: usize> Router<TABLE_SIZE> {
    pub fn new() -> Self {
        Self {
            table: SgMap::new(),
        }
    }

    pub fn add_route(&mut self, port: PortId, address: Address) {
        let port = PortSet::from(port);
        self.table
            .entry(address)
            .and_modify(|set| *set |= port)
            .or_insert(port);
    }

    pub fn get_route(&self, address: Address) -> PortSet {
        if let Some(set) = self.table.get(&address) {
            *set
        } else {
            PortSet::empty()
        }
    }
}
