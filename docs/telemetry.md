# Sending: battery and GPS frames

Part of [crsf-codec](../README.md)

Two encoders write telemetry frames into the start of the caller's slice and return the length. Out-of-range input is rejected,
never saturated or truncated, and nothing is written on an error. All multi-byte payload fields are big-endian.

## Battery sensor (type 0x08, 12 bytes)

`encode_battery(&Battery, &mut [u8]) -> Result<usize, EncodeError>`; address 0xC8.

| Field | Wire | Unit |
|---|---|---|
| Voltage | `u16` | 0.1 V (every `u16` accepted) |
| Current | `u16` | 0.1 A (every `u16` accepted) |
| Capacity used | `u24` | mAh, at most 16 777 215 |
| Remaining | `u8` | percent, at most 100 |

## GPS (type 0x02, 19 bytes, payload 15)

`encode_gps(&Gps, &mut [u8]) -> Result<usize, EncodeError>`; address 0xC8.

| Field (`Gps`) | Wire | Unit |
|---|---|---|
| `latitude_e7` | `i32` | 1e-7 degree |
| `longitude_e7` | `i32` | 1e-7 degree |
| `groundspeed_dkmh` | `u16` | 0.1 km/h |
| `heading_cdeg` | `u16` | 0.01 degree |
| `altitude_m` | `u16` | metres plus 1000; `i32` input from -1000 to 64 535 |
| satellites | `u8` | count |

Latitude, longitude, speed, heading and satellites are written as given. The layout follows Betaflight and ArduPilot and agrees
with `uf-crsf` 0.7.0; ground speed is 0.1 km/h per unit as in both flight stacks' code. Betaflight clamps the wire altitude to
0 to 5000 (-1000 to 4000 m) and ArduPilot clamps the metres to 0 to 5000 before adding 1000; a caller that wants either applies it
before encoding.

## Errors

| `EncodeError` | When |
|---|---|
| `CapacityOutOfRange` | Capacity above 16 777 215 mAh |
| `PercentOutOfRange` | Percent above 100 |
| `AltitudeOutOfRange` | Altitude outside -1000 to 64 535 m |
| `BufferTooSmall` | Slice shorter than 12 (battery) or 19 (GPS) bytes |

Constants: `BATTERY_FRAME_LEN`, `MAX_CAPACITY_MAH`, `GPS_FRAME_LEN`, `MIN_ALTITUDE_M`, `MAX_ALTITUDE_M`.

## Code

`src/battery.rs`, `src/gps.rs`.

## Related

[frames](frames.md); [testing](testing.md) for the cross-check against `uf-crsf`.
