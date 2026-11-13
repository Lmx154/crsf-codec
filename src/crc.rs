//! CRC-8 with polynomial 0xD5 (CRC-8/DVB-S2): initial value 0, no reflection, no final xor.

const POLY: u8 = 0xD5;

/// CRC over `data`, as CRSF computes it over the type byte and the payload.
pub(crate) fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &byte in data {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ POLY
            } else {
                crc << 1
            };
        }
    }
    crc
}
