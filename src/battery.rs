//! The battery sensor encoder (type 0x08).

use crate::ADDRESS;
use crate::crc::crc8;

/// Length of a battery sensor frame: address, length, type, 8 payload bytes, CRC.
pub const BATTERY_FRAME_LEN: usize = 12;
const TYPE_BATTERY: u8 = 0x08;
/// Largest capacity the 24-bit field holds.
pub const MAX_CAPACITY_MAH: u32 = 0x00FF_FFFF;

/// Input of [`encode_battery`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Battery {
    /// Voltage in units of 0.1 V (120 is 12.0 V). Every `u16` is representable.
    pub voltage_dv: u16,
    /// Current in units of 0.1 A (15 is 1.5 A). Every `u16` is representable.
    pub current_da: u16,
    /// Capacity used in mAh, 0 to [`MAX_CAPACITY_MAH`] (16 777 215): the wire field has 24 bits.
    pub capacity_used_mah: u32,
    /// Remaining capacity in percent, 0 to 100.
    pub remaining_percent: u8,
}

/// Why [`encode_battery`] refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum EncodeError {
    /// The output buffer is shorter than [`BATTERY_FRAME_LEN`].
    BufferTooSmall,
    /// `capacity_used_mah` is above [`MAX_CAPACITY_MAH`].
    CapacityOutOfRange,
    /// `remaining_percent` is above 100.
    PercentOutOfRange,
}

/// Writes a battery sensor frame into the start of `out` and returns its length,
/// [`BATTERY_FRAME_LEN`].
///
/// Out-of-range values are rejected, never saturated or truncated; on an error nothing is written.
/// The payload is voltage (u16), current (u16) and capacity used (u24), all big-endian, then the
/// remaining percent (u8).
pub fn encode_battery(battery: &Battery, out: &mut [u8]) -> Result<usize, EncodeError> {
    let Some(dst) = out.first_chunk_mut::<BATTERY_FRAME_LEN>() else {
        return Err(EncodeError::BufferTooSmall);
    };
    if battery.capacity_used_mah > MAX_CAPACITY_MAH {
        return Err(EncodeError::CapacityOutOfRange);
    }
    if battery.remaining_percent > 100 {
        return Err(EncodeError::PercentOutOfRange);
    }
    let [v1, v0] = battery.voltage_dv.to_be_bytes();
    let [c1, c0] = battery.current_da.to_be_bytes();
    let [_, m2, m1, m0] = battery.capacity_used_mah.to_be_bytes();
    let body = [
        TYPE_BATTERY,
        v1,
        v0,
        c1,
        c0,
        m2,
        m1,
        m0,
        battery.remaining_percent,
    ];
    let crc = crc8(&body);
    let [t, p0, p1, p2, p3, p4, p5, p6, p7] = body;
    *dst = [ADDRESS, 10, t, p0, p1, p2, p3, p4, p5, p6, p7, crc];
    Ok(BATTERY_FRAME_LEN)
}
