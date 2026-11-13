# crsf-codec

CRSF codec for the MARV V2 flight controller, which listens to an ExpressLRS receiver on a UART. The crate is `no_std`, has no dependencies (except an optional `defmt` 1.x behind the feature `defmt`) and does no I/O: bytes in, validated frames out. It reads RC channels and link statistics, passes every other frame type through as raw bytes, and writes one frame, the battery sensor. It has no time, no allocation, no statics and no `unsafe`, and it uses integer arithmetic only. It never panics on any input.

**Status: version 0.1.0 (`publish = false`). The API may still change. It is tested on the host, and on 2026-11-13, through `rc-port` on the MARV V2 Rev 1 board, it decoded the CRSF of a real ExpressLRS 4.0.1 receiver at 420000 baud: about 246 RC frames and 10 link-statistics frames per second for several minutes, with 0 CRC errors in CRSF mode. Its battery frame was shown by the handset as sent (12.3 V, 0.0 A, 0 mAh, 50 %).** Frames captured from the receiver have not yet been added as test vectors; that is still planned.

## Works with

Any target and any byte source: the crate is pure. It does not know the UART, the baud rate or the executor.

## Frame format

`[address][len][type][payload...][crc8]`. The address is 0xC8 (the flight controller). `len` counts type, payload and CRC, so a frame is `len + 2` bytes and at most 64 bytes. The CRC is CRC-8 with polynomial 0xD5 (initial value 0, no reflection, no final xor) over type and payload.

The deframer accepts a frame only if the address byte is 0xC8, `len` is 2 to 62 and the CRC matches. On a bad length or a bad CRC it drops one byte and rescans from the next byte, so a stray 0xC8 in garbage does not swallow a real frame that follows. A stray 0xC8 followed by a plausible `len` is waited for to its end (at most 64 bytes from the 0xC8) before it is rejected, so a real frame behind it comes out when that many bytes have arrived; and with probability 1/256 the false candidate, which spans garbage and the start of the real frame, passes the CRC and the real frame is lost.

Decoded types:

- 0x16 RC channels packed, payload exactly 22 bytes: 16 raw 11-bit values, channel 1 in the lowest bits of the first byte, little-endian bit packing.
- 0x14 link statistics, payload exactly 10 bytes: uplink RSSI antenna 1, uplink RSSI antenna 2, uplink link quality, uplink SNR (`i8`), active antenna, RF mode, uplink TX power index, downlink RSSI, downlink link quality, downlink SNR (`i8`). All raw, as received.

Every other type, and a 0x16 or 0x14 frame with any other payload length, comes out as `Frame::Other { frame_type, payload }` and is counted.

## API

```text
pub struct Deframer;
impl Deframer {
    pub const fn new() -> Self;
    pub fn push<F: FnMut(Frame<'_>)>(&mut self, bytes: &[u8], on_frame: F);
    pub const fn stats(&self) -> Stats;
    pub fn clear(&mut self); // drops the buffered bytes (counted as garbage), e.g. after a baud rate change
}
pub enum Frame<'a> { // non_exhaustive
    RcChannels(RcChannels),            // raw: [u16; 16]
    LinkStatistics(LinkStatistics),
    Other { frame_type: u8, payload: &'a [u8] },
}
pub const fn raw_to_us(v: u16) -> u16;
pub fn encode_battery(battery: &Battery, out: &mut [u8]) -> Result<usize, EncodeError>;
```

`push` takes the bytes in any chunking, down to one byte per call, and calls `on_frame` once per accepted frame, in stream order. The result is the same however the stream is split. The frame borrows the deframer's buffer and is valid inside the call only.

```rust
use crsf_codec::{Deframer, Frame, raw_to_us};

// Two stray bytes, then an RC frame with channel 1 = 172, channel 2 = 992, channel 3 = 1811.
let stream: [u8; 28] = [
    0x00, 0x7F, 0xC8, 0x18, 0x16, 0xAC, 0x00, 0xDF, 0xC4, 0xC1, 0x07, 0x3E, 0xF0, 0x81, 0x0F,
    0x7C, 0xE0, 0x03, 0x1F, 0xF8, 0xC0, 0x07, 0x3E, 0xF0, 0x81, 0x0F, 0x7C, 0xB5,
];

let mut deframer = Deframer::new();
let mut sticks_us = None;
for chunk in stream.chunks(5) {
    deframer.push(chunk, |frame| {
        if let Frame::RcChannels(rc) = frame {
            sticks_us = Some(rc.raw.map(raw_to_us));
        }
    });
}
let sticks_us = sticks_us.unwrap();
assert_eq!(sticks_us[..3], [988, 1500, 2012]);
assert_eq!(deframer.stats().frames, 1);
assert_eq!(deframer.stats().garbage_bytes, 2);
```

`raw_to_us(v)` is `(v * 5 + 4) / 8 + 880` in wider arithmetic: 172 gives 988, 992 gives 1500, 1811 gives 2012. It is not clamped, and it is defined for every `u16` without overflow. In the switch modes that do not send 16 channels, channels 15 and 16 carry link quality and RSSI; the crate passes them as received.

`encode_battery` writes a battery sensor frame (type 0x08, address 0xC8, 12 bytes) into the start of the caller's slice and returns 12. Payload: voltage `u16` big-endian in units of 0.1 V, current `u16` big-endian in units of 0.1 A, capacity used `u24` big-endian in mAh, remaining `u8` in percent. Out-of-range input is rejected, never saturated or truncated, and nothing is written on an error: a capacity above 16 777 215 mAh gives `EncodeError::CapacityOutOfRange`, a percent above 100 gives `EncodeError::PercentOutOfRange`, and a slice shorter than 12 bytes gives `EncodeError::BufferTooSmall`. Voltage and current take every `u16`.

## Counters

`Deframer::stats()` returns a `Stats` struct, all `u32`, all saturating:

- `garbage_bytes`: bytes discarded without being part of an accepted frame (bytes before an address byte, and the first byte of each candidate rejected for its length or CRC). Every byte pushed is exactly one of: buffered, in an accepted frame, or counted here.
- `bad_length`: candidates whose length byte is outside 2 to 62.
- `crc_errors`: candidates whose CRC did not match.
- `frames`: frames accepted, of every type.
- `other_frames`: accepted frames that came out as `Frame::Other`.

## Deliberately not in the crate

Subset RC frames (0x17), link statistics RX and TX (0x1C, 0x1D), MSP, device ping and info, parameter and command frames, baud negotiation, encoders for flight mode, GPS and attitude, time, loss detection, and any policy (what to do when frames stop, which stick source is in charge). The CRC function is not public.

## Tests

`cargo test` runs the host tests. Property tests (`proptest`): the same frames and the same counters however the stream is chunked (whole, split at random points, one byte at a time); arbitrary bytes, several kB per case, never panic and the buffer never holds more than 64 bytes, with every byte accounted for; the packing round trip through a test-only packer; the CRC against an independent long-division reference and the standard check value 0xBC for "123456789". Exhaustive tests: a stray 0xC8 with a plausible length at every offset 1 to 63 before a real frame; every single-bit flip of a reference RC, link statistics, battery and other frame, with the exact outcome counts (no flip decodes to the original; with a zero trailer some flips of the length byte give a different valid frame, listed in the test); `raw_to_us` over all of `u16`, monotonic and within 1 of the integer model `(v - 172) * 1024 / 1639 + 988` on 172 to 1811. Negative controls: reference frames built with a wrong CRC polynomial are refused by the bit-flip harness. Hand-worked vectors cover the RC packing, link statistics and the battery frame. A differential test checks the RC and link statistics decoders against `uf-crsf` 0.7.0 (a dev-dependency pinned `=0.7.0`, `Cargo.lock` committed) on random payloads, and that `uf-crsf` parses our battery frame back to the same values and writes the same bytes.

## Licence

MIT, see `LICENSE`.
