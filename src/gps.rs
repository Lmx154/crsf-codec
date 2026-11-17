//! The GPS encoder (type 0x02).

use crate::ADDRESS;
use crate::EncodeError;
use crate::crc::crc8;

/// Length of a GPS frame: address, length, type, 15 payload bytes, CRC.
pub const GPS_FRAME_LEN: usize = 19;
const TYPE_GPS: u8 = 0x02;
const ALTITUDE_OFFSET_M: i32 = 1000;
/// Lowest altitude the wire field holds: wire value 0.
pub const MIN_ALTITUDE_M: i32 = -ALTITUDE_OFFSET_M;
/// Highest altitude the wire field holds: wire value 65535.
pub const MAX_ALTITUDE_M: i32 = 0xFFFF - ALTITUDE_OFFSET_M;

/// Input of [`encode_gps`].
///
/// The codec does not clamp altitude, and it does not scale or limit any other field either.
/// Betaflight and ArduPilot each clamp altitude differently before they encode it: Betaflight
/// clamps the wire value (metres plus 1000) to 0 to 5000 (`betaflight-telemetry-crsf.c:285`),
/// ArduPilot clamps the metres to 0 to 5000 and then adds 1000 (`ardupilot-AP_CRSF_Telem.cpp:1118`),
/// so the wire range is 1000 to 6000. That is display policy of the caller, not part of the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Gps {
    /// Latitude in units of 1e-7 degree (450 000 000 is 45.0 degrees north), signed.
    pub latitude_e7: i32,
    /// Longitude in units of 1e-7 degree (90 000 000 is 9.0 degrees east), signed.
    pub longitude_e7: i32,
    /// Ground speed in units of 0.1 km/h (360 is 36.0 km/h, 10 m/s). Written as given.
    pub groundspeed_dkmh: u16,
    /// Heading in units of 0.01 degree (9000 is 90.00 degrees). Written as given.
    pub heading_cdeg: u16,
    /// Altitude in metres, [`MIN_ALTITUDE_M`] (-1000) to [`MAX_ALTITUDE_M`] (64 535). On the wire it
    /// is the `u16` `altitude_m + 1000`.
    pub altitude_m: i32,
    /// Satellites in use. Written as given.
    pub satellites: u8,
}

/// Writes a GPS frame into the start of `out` and returns its length, [`GPS_FRAME_LEN`].
///
/// The only value rejected is an altitude the wire field cannot hold
/// ([`EncodeError::AltitudeOutOfRange`]); nothing is written on an error. The payload is latitude
/// (i32), longitude (i32), ground speed (u16), heading (u16) and altitude plus 1000 (u16), all
/// big-endian, then the satellite count (u8).
pub fn encode_gps(gps: &Gps, out: &mut [u8]) -> Result<usize, EncodeError> {
    let Some(dst) = out.first_chunk_mut::<GPS_FRAME_LEN>() else {
        return Err(EncodeError::BufferTooSmall);
    };
    let Some(altitude) = gps
        .altitude_m
        .checked_add(ALTITUDE_OFFSET_M)
        .and_then(|a| u16::try_from(a).ok())
    else {
        return Err(EncodeError::AltitudeOutOfRange);
    };
    let [la3, la2, la1, la0] = gps.latitude_e7.to_be_bytes();
    let [lo3, lo2, lo1, lo0] = gps.longitude_e7.to_be_bytes();
    let [s1, s0] = gps.groundspeed_dkmh.to_be_bytes();
    let [h1, h0] = gps.heading_cdeg.to_be_bytes();
    let [a1, a0] = altitude.to_be_bytes();
    let body = [
        TYPE_GPS,
        la3,
        la2,
        la1,
        la0,
        lo3,
        lo2,
        lo1,
        lo0,
        s1,
        s0,
        h1,
        h0,
        a1,
        a0,
        gps.satellites,
    ];
    let crc = crc8(&body);
    let [
        t,
        p0,
        p1,
        p2,
        p3,
        p4,
        p5,
        p6,
        p7,
        p8,
        p9,
        p10,
        p11,
        p12,
        p13,
        p14,
    ] = body;
    *dst = [
        ADDRESS, 17, t, p0, p1, p2, p3, p4, p5, p6, p7, p8, p9, p10, p11, p12, p13, p14, crc,
    ];
    Ok(GPS_FRAME_LEN)
}
