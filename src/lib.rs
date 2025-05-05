#![cfg_attr(not(test), no_std)]

pub(crate) mod buf;

pub mod ptp_packets;

#[macro_export]
macro_rules! unwrap {
    ($cond:expr) => {
        #[cfg(feature = "defmt")]
        defmt::unwrap!($cond);
        #[cfg(feature = "log")]
        $cond.unwrap();
    };
}
