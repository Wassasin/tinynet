#![cfg_attr(not(test), no_std)]

// This mod MUST go first, so that the others see its macros.
#[doc(hidden)]
pub mod fmt;

pub(crate) mod buf;

pub mod protocols;
pub mod ptp_packets;
pub mod routing;
