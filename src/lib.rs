//! CRSF codec for a flight controller that listens to an ExpressLRS receiver.
//!
//! [`Deframer`] turns bytes into validated [`Frame`]s: RC channels packed, link statistics, and every
//! other type as raw bytes. [`raw_to_us`] converts a raw channel value to microseconds and
//! [`encode_battery`] writes one telemetry frame. Pure `no_std`: no I/O, no time, no allocation.
#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::todo,
        clippy::unimplemented,
        clippy::indexing_slicing,
        clippy::undocumented_unsafe_blocks
    )
)]

#[cfg(test)]
extern crate std;

mod battery;
mod crc;
mod decode;
mod deframer;
#[cfg(test)]
mod tests;

pub use battery::{BATTERY_FRAME_LEN, Battery, EncodeError, MAX_CAPACITY_MAH, encode_battery};
pub use decode::{Frame, LinkStatistics, RcChannels, raw_to_us};
pub use deframer::{Deframer, MAX_FRAME_LEN, Stats};

/// The only address byte accepted and written: the flight controller.
pub(crate) const ADDRESS: u8 = 0xC8;

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
