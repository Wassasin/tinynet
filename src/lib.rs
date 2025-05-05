#![cfg_attr(not(test), no_std)]

pub(crate) mod buf;

pub mod ptp_packets;

#[macro_export]
macro_rules! unwrap {
    ($cond:expr) => {
        #[allow(unused)]
        let value = $cond;
        #[cfg(feature = "defmt")]
        defmt::unwrap!(value);
        #[cfg(feature = "log")]
        value.unwrap();
    };
}
