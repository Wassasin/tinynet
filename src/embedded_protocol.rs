//! Packet-based embedded protocol providing discovery, assignment and routing.
//!
//! Does not provide reliability.

use bitfield_struct::bitfield;
use endian_num::{le16, le32};

#[bitfield(u16, repr = le16, from = le16::from_ne, into = le16::to_ne)]
struct Address {
    #[bits(12)]
    inner: u16,
}

#[bitfield(u32, repr = le32, from = le32::from_ne, into = le32::to_ne)]
struct Header {
    #[bits(12)]
    src: u16,
    #[bits(12)]
    dst: u16,
}
