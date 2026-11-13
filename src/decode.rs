//! The frames the deframer hands out, and their decoders.

/// Payload length of an RC channels packed frame (type 0x16).
const RC_PAYLOAD_LEN: usize = 22;
/// Frame type of RC channels packed.
pub(crate) const TYPE_RC_CHANNELS: u8 = 0x16;
/// Frame type of link statistics.
pub(crate) const TYPE_LINK_STATISTICS: u8 = 0x14;

/// The 16 channels of an RC channels packed frame (type 0x16), raw 11-bit values (0 to 2047).
///
/// Channel 1 is `raw[0]`. Use [`raw_to_us`] for microseconds. The values are as received: nothing is
/// clamped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct RcChannels {
    /// Raw channel values, 11 bits each.
    pub raw: [u16; 16],
}

/// The ten fields of a link statistics frame (type 0x14), raw as received.
///
/// The RSSI fields are the received power in dBm with the sign dropped (a value of 60 means -60 dBm).
/// The other fields are as ExpressLRS fills them; this crate does not interpret them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct LinkStatistics {
    /// Uplink RSSI, antenna 1 (dBm, sign dropped).
    pub uplink_rssi_1: u8,
    /// Uplink RSSI, antenna 2 (dBm, sign dropped).
    pub uplink_rssi_2: u8,
    /// Uplink link quality (percent).
    pub uplink_link_quality: u8,
    /// Uplink SNR (dB).
    pub uplink_snr: i8,
    /// Active antenna.
    pub active_antenna: u8,
    /// RF mode (packet rate profile).
    pub rf_mode: u8,
    /// Uplink TX power index.
    pub uplink_tx_power: u8,
    /// Downlink RSSI (dBm, sign dropped).
    pub downlink_rssi: u8,
    /// Downlink link quality (percent).
    pub downlink_link_quality: u8,
    /// Downlink SNR (dB).
    pub downlink_snr: i8,
}

/// One accepted frame.
///
/// A 0x16 frame whose payload is not exactly 22 bytes and a 0x14 frame whose payload is not exactly
/// 10 bytes are not decoded: they come out as [`Frame::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Frame<'a> {
    /// RC channels packed (type 0x16, payload exactly 22 bytes).
    RcChannels(RcChannels),
    /// Link statistics (type 0x14, payload exactly 10 bytes).
    LinkStatistics(LinkStatistics),
    /// Any other frame type, or a 0x16 or 0x14 frame of the wrong payload length.
    Other {
        /// The type byte.
        frame_type: u8,
        /// The payload, without type and CRC.
        payload: &'a [u8],
    },
}

impl<'a> Frame<'a> {
    pub(crate) fn decode(frame_type: u8, payload: &'a [u8]) -> Self {
        match frame_type {
            TYPE_RC_CHANNELS if payload.len() == RC_PAYLOAD_LEN => {
                Frame::RcChannels(unpack_channels(payload))
            }
            TYPE_LINK_STATISTICS => match *payload {
                [a, b, c, d, e, f, g, h, i, j] => Frame::LinkStatistics(LinkStatistics {
                    uplink_rssi_1: a,
                    uplink_rssi_2: b,
                    uplink_link_quality: c,
                    uplink_snr: i8::from_ne_bytes([d]),
                    active_antenna: e,
                    rf_mode: f,
                    uplink_tx_power: g,
                    downlink_rssi: h,
                    downlink_link_quality: i,
                    downlink_snr: i8::from_ne_bytes([j]),
                }),
                _ => Frame::Other {
                    frame_type,
                    payload,
                },
            },
            _ => Frame::Other {
                frame_type,
                payload,
            },
        }
    }
}

/// 16 values of 11 bits, least significant bit first, channel 1 in the lowest bits of the first byte.
fn unpack_channels(payload: &[u8]) -> RcChannels {
    let mut raw = [0u16; 16];
    let mut slots = raw.iter_mut();
    let mut acc = 0u32;
    let mut bits = 0u32;
    for &byte in payload {
        acc |= u32::from(byte) << bits;
        bits += 8;
        while bits >= 11 {
            if let Some(slot) = slots.next() {
                *slot = (acc & 0x7FF) as u16;
            }
            acc >>= 11;
            bits -= 11;
        }
    }
    RcChannels { raw }
}

/// Converts a raw 11-bit channel value to microseconds: `(v * 5 + 4) / 8 + 880`.
///
/// 172 gives 988, 992 gives 1500, 1811 gives 2012. The result is not clamped; it is defined for every
/// `u16` (values above 2047 cannot come from the wire) and does not overflow.
pub const fn raw_to_us(v: u16) -> u16 {
    let us = (v as u32 * 5 + 4) / 8 + 880;
    if us > u16::MAX as u32 {
        u16::MAX
    } else {
        us as u16
    }
}
