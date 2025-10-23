#![cfg_attr(not(test), no_std)]
#![doc = include_str!(concat!("../", env!("CARGO_PKG_README")))]

// This mod MUST go first, so that the others see its macros.
pub(crate) mod fmt;

pub(crate) mod buf;

pub mod protocols;
pub mod ptp_packets;
pub mod routing;

pub const PROTOCOL_DISCOVERY: u8 = 0x00;
