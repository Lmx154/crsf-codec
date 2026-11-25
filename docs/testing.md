# Testing and scope

Part of [crsf-codec](../README.md)

`cargo test` runs the host tests; the README example runs as a doctest.

| Kind | Checks |
|---|---|
| Property (`proptest`) | Same frames and counters however the stream is chunked (whole, random splits, one byte at a time); arbitrary bytes, several kB per case, never panic, the buffer never holds more than 64 bytes and every byte is accounted for; packing round trip through a test-only packer; CRC against an independent long-division reference and the check value 0xBC for "123456789" |
| Exhaustive | A stray 0xC8 with a plausible length at every offset 1 to 63 before a real frame; every single-bit flip of a reference RC, link-statistics, battery and other frame with the exact outcome counts (with a zero trailer some length-byte flips give a different valid frame, listed in the test); `raw_to_us` over all of `u16`, monotonic and within 1 of `(v - 172) * 1024 / 1639 + 988` on 172 to 1811 |
| Negative control | Reference frames built with a wrong CRC polynomial are refused by the bit-flip harness |
| Hand-worked vectors | RC packing, link statistics, battery, GPS (made-up coordinates, altitude limits and errors) |
| Differential | RC and link-statistics decoders against `uf-crsf` 0.7.0 (dev-dependency pinned `=0.7.0`, `Cargo.lock` committed) on random payloads; `uf-crsf` parses our battery and GPS frames back to the same values and writes the same bytes |

## Deliberately not in the crate

Subset RC frames (0x17), link statistics RX and TX (0x1C, 0x1D), MSP, device ping and info, parameter and command frames, baud
negotiation, encoders for flight mode, attitude and other telemetry, time, loss detection, and any policy (what to do when frames
stop, which stick source is in charge).

## Code

`src/tests.rs`.

## Related

[frames](frames.md), [telemetry](telemetry.md).
