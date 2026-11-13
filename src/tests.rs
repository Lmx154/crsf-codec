//! Host tests. The numbered ones (I1 to I6) are the acceptance criteria of the design decision.

use crate::crc::crc8;
use crate::{
    BATTERY_FRAME_LEN, Battery, Deframer, EncodeError, Frame, LinkStatistics, MAX_CAPACITY_MAH,
    MAX_FRAME_LEN, RcChannels, Stats, encode_battery, raw_to_us,
};
use proptest::prelude::*;
use std::vec::Vec;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Owned {
    Rc(RcChannels),
    Link(LinkStatistics),
    Other(u8, Vec<u8>),
}

fn own(frame: Frame<'_>) -> Owned {
    match frame {
        Frame::RcChannels(rc) => Owned::Rc(rc),
        Frame::LinkStatistics(ls) => Owned::Link(ls),
        Frame::Other {
            frame_type,
            payload,
        } => Owned::Other(frame_type, payload.to_vec()),
    }
}

/// Bytes an accepted frame took on the wire.
fn wire_len(frame: &Owned) -> usize {
    match frame {
        Owned::Rc(_) => 26,
        Owned::Link(_) => 14,
        Owned::Other(_, payload) => payload.len() + 4,
    }
}

/// A complete frame with address 0xC8.
fn frame(frame_type: u8, payload: &[u8]) -> Vec<u8> {
    let mut f = std::vec![0xC8, u8::try_from(payload.len() + 2).unwrap(), frame_type];
    f.extend_from_slice(payload);
    f.push(crc8(&f[2..]));
    f
}

/// Test-only packer: 16 values of 11 bits, least significant bit first, one bit at a time.
fn pack(raw: &[u16; 16]) -> [u8; 22] {
    let mut out = [0u8; 22];
    for (channel, value) in raw.iter().enumerate() {
        for bit in 0..11 {
            if (value >> bit) & 1 == 1 {
                let position = channel * 11 + bit;
                out[position / 8] |= 1 << (position % 8);
            }
        }
    }
    out
}

fn run_chunks(chunks: &[&[u8]]) -> (Vec<Owned>, Stats, usize) {
    let mut d = Deframer::new();
    let mut out = Vec::new();
    for chunk in chunks {
        d.push(chunk, |f| out.push(own(f)));
        assert!(d.buffered() <= MAX_FRAME_LEN);
    }
    (out, d.stats(), d.buffered())
}

fn run_whole(bytes: &[u8]) -> (Vec<Owned>, Stats, usize) {
    run_chunks(&[bytes])
}

fn run_cut(bytes: &[u8], cuts: &[usize]) -> (Vec<Owned>, Stats, usize) {
    let mut points: Vec<usize> = cuts.iter().map(|c| c % (bytes.len() + 1)).collect();
    points.push(0);
    points.push(bytes.len());
    points.sort_unstable();
    let chunks: Vec<&[u8]> = points.windows(2).map(|w| &bytes[w[0]..w[1]]).collect();
    run_chunks(&chunks)
}

fn run_bytewise(bytes: &[u8]) -> (Vec<Owned>, Stats, usize) {
    let chunks: Vec<&[u8]> = bytes.chunks(1).collect();
    run_chunks(&chunks)
}

// ---------------------------------------------------------------- CRC

/// Independent formulation: long division of the message followed by eight zero bits by the 9-bit
/// polynomial 0x1D5, on a 16-bit register.
fn crc8_division(data: &[u8]) -> u8 {
    let mut bits: Vec<bool> = Vec::new();
    for byte in data {
        for i in (0..8).rev() {
            bits.push((byte >> i) & 1 == 1);
        }
    }
    bits.extend([false; 8]);
    let mut reg: u16 = 0;
    for bit in bits {
        reg = (reg << 1) | u16::from(bit);
        if reg & 0x100 != 0 {
            reg ^= 0x1D5;
        }
    }
    u8::try_from(reg & 0xFF).unwrap()
}

#[test]
fn crc_check_value() {
    assert_eq!(crc8(b"123456789"), 0xBC);
    assert_eq!(crc8_division(b"123456789"), 0xBC);
    assert_eq!(crc8(&[]), 0);
}

proptest! {
    #[test]
    fn crc_matches_reference(data in proptest::collection::vec(any::<u8>(), 0..80)) {
        prop_assert_eq!(crc8(&data), crc8_division(&data));
    }
}

// ---------------------------------------------------------------- raw_to_us (I6)

#[test]
fn i6_raw_to_us_spec_points() {
    assert_eq!(raw_to_us(172), 988);
    assert_eq!(raw_to_us(992), 1500);
    assert_eq!(raw_to_us(1811), 2012);
    assert_eq!(raw_to_us(0), 880);
    assert_eq!(raw_to_us(2047), 2159);
    assert_eq!(raw_to_us(65535), 41839);
}

#[test]
fn i6_raw_to_us_monotonic_total_and_close_to_model() {
    let mut previous = 0;
    for v in 0..=u16::MAX {
        let us = raw_to_us(v);
        assert!(us >= previous, "not monotonic at {v}");
        previous = us;
    }
    for v in 172u32..=1811 {
        let model = (v - 172) * 1024 / 1639 + 988;
        let ours = u32::from(raw_to_us(v as u16));
        assert!(
            ours.abs_diff(model) <= 1,
            "v {v}: ours {ours}, model {model}"
        );
    }
}

// ---------------------------------------------------------------- RC and link statistics decoding

fn decode_rc(payload: &[u8]) -> Owned {
    let (frames, stats, _) = run_whole(&frame(0x16, payload));
    assert_eq!(stats.frames, 1);
    frames.into_iter().next().unwrap()
}

#[test]
fn rc_all_zero_and_all_max() {
    assert_eq!(decode_rc(&[0; 22]), Owned::Rc(RcChannels { raw: [0; 16] }));
    assert_eq!(
        decode_rc(&[0xFF; 22]),
        Owned::Rc(RcChannels { raw: [2047; 16] })
    );
}

#[test]
fn rc_channel_k_is_k_times_100() {
    let mut raw = [0u16; 16];
    for (i, v) in raw.iter_mut().enumerate() {
        *v = (i as u16 + 1) * 100;
    }
    assert_eq!(decode_rc(&pack(&raw)), Owned::Rc(RcChannels { raw }));
}

#[test]
fn rc_hand_packed_examples() {
    // Channel 1 = 1, channel 2 = 2, channel 3 = 4, the rest 0. Channel n starts at bit 11 * (n - 1):
    //   channel 1 bit 0 is stream bit 0  -> byte 0, bit 0 -> 0x01
    //   channel 2 bit 1 is stream bit 12 -> byte 1, bit 4 -> 0x10
    //   channel 3 bit 2 is stream bit 24 -> byte 3, bit 0 -> 0x01
    let mut payload = [0u8; 22];
    payload[0] = 0x01;
    payload[1] = 0x10;
    payload[3] = 0x01;
    let mut raw = [0u16; 16];
    raw[0] = 1;
    raw[1] = 2;
    raw[2] = 4;
    assert_eq!(decode_rc(&payload), Owned::Rc(RcChannels { raw }));
    assert_eq!(pack(&raw), payload);

    // Channel 1 = 0x7FF: bits 0 to 10 -> byte 0 = 0xFF, byte 1 bits 0 to 2 = 0x07.
    // Channel 16 = 0x7FF: stream bits 165 to 175 -> byte 20 bits 5 to 7 = 0xE0, byte 21 = 0xFF.
    let mut payload = [0u8; 22];
    payload[0] = 0xFF;
    payload[1] = 0x07;
    payload[20] = 0xE0;
    payload[21] = 0xFF;
    let mut raw = [0u16; 16];
    raw[0] = 2047;
    raw[15] = 2047;
    assert_eq!(decode_rc(&payload), Owned::Rc(RcChannels { raw }));
    assert_eq!(pack(&raw), payload);

    // Stick centre on every channel: 992 = 0x3E0 = 0b011_1110_0000, bits 5 to 9 set.
    // Byte 0 = stream bits 0 to 7 = channel 1 bits 5, 6, 7 -> 0xE0.
    // Byte 1 = stream bits 8 to 15 = channel 1 bits 8, 9 -> 0x03 (channel 2 starts at stream bit 11
    // and its low five bits are 0).
    // Byte 2 = stream bits 16 to 23 = channel 2 bits 5 to 9 at stream bits 16 to 20 -> 0x1F
    // (channel 3 starts at stream bit 22 and its low five bits are 0).
    let payload = pack(&[992; 16]);
    assert_eq!(payload[..3], [0xE0, 0x03, 0x1F]);
    assert_eq!(
        decode_rc(&payload),
        Owned::Rc(RcChannels { raw: [992; 16] })
    );
}

#[test]
fn link_statistics_hand_vector() {
    let payload = [16, 20, 100, 0xF6, 1, 3, 4, 42, 99, 0x05];
    assert_eq!(
        own(Frame::decode(0x14, &payload)),
        Owned::Link(LinkStatistics {
            uplink_rssi_1: 16,
            uplink_rssi_2: 20,
            uplink_link_quality: 100,
            uplink_snr: -10,
            active_antenna: 1,
            rf_mode: 3,
            uplink_tx_power: 4,
            downlink_rssi: 42,
            downlink_link_quality: 99,
            downlink_snr: 5,
        })
    );
}

#[test]
fn wrong_payload_lengths_are_other_and_counted() {
    for (ty, len) in [
        (0x16, 21),
        (0x16, 23),
        (0x16, 0),
        (0x14, 9),
        (0x14, 11),
        (0x14, 0),
    ] {
        let payload = std::vec![0x11; len];
        let (frames, stats, _) = run_whole(&frame(ty, &payload));
        assert_eq!(
            frames,
            [Owned::Other(ty, payload)],
            "type {ty:#x} len {len}"
        );
        assert_eq!((stats.frames, stats.other_frames), (1, 1));
    }
    let (frames, stats, _) = run_whole(&frame(0x16, &[0; 22]));
    assert_eq!(frames.len(), 1);
    assert_eq!((stats.frames, stats.other_frames), (1, 0));
}

proptest! {
    #[test]
    fn rc_encode_decode_identity(raw in proptest::array::uniform16(0u16..2048)) {
        prop_assert_eq!(decode_rc(&pack(&raw)), Owned::Rc(RcChannels { raw }));
    }
}

// ---------------------------------------------------------------- deframer edge cases

#[test]
fn length_byte_limits_and_address() {
    // 2 is the smallest legal length byte (type and CRC only), 62 the largest (a 64-byte frame).
    let (frames, _, _) = run_whole(&frame(0x30, &[]));
    assert_eq!(frames, [Owned::Other(0x30, Vec::new())]);
    let (frames, stats, left) = run_whole(&frame(0x30, &[7; 60]));
    assert_eq!(frames, [Owned::Other(0x30, std::vec![7; 60])]);
    assert_eq!((stats.garbage_bytes, left), (0, 0));

    for bad in [0u8, 1, 63, 64, 201, 255] {
        let (frames, stats, _) = run_whole(&[0xC8, bad, 0, 0]);
        assert!(frames.is_empty());
        assert_eq!(stats.bad_length, 1, "length byte {bad}");
    }
    // Another address with a correct CRC is not accepted.
    let mut f = frame(0x30, &[1, 2, 3]);
    f[0] = 0xEA;
    let (frames, stats, _) = run_whole(&f);
    assert!(frames.is_empty());
    assert_eq!(stats.garbage_bytes, f.len() as u32);
}

#[test]
fn counters_add_up_on_a_known_stream() {
    let mut stream = std::vec![1, 2, 3];
    stream.extend(frame(0x16, &pack(&[992; 16])));
    stream.extend([0xC8, 0xFF]); // bad length
    stream.extend(frame(0x08, &[0; 8]));
    let mut bad_crc = frame(0x14, &[0; 10]);
    *bad_crc.last_mut().unwrap() ^= 1;
    stream.extend(bad_crc);
    stream.extend(frame(0x14, &[0; 10]));
    let (frames, stats, left) = run_whole(&stream);
    assert_eq!(frames.len(), 3);
    assert_eq!(stats.frames, 3);
    assert_eq!(stats.other_frames, 1);
    assert_eq!(stats.bad_length, 1);
    assert_eq!(stats.crc_errors, 1);
    // 3 leading bytes, the 0xC8 of the bad length, the 0xFF behind it, and the whole bad-CRC frame
    // (14 bytes, dropped one at a time).
    assert_eq!(stats.garbage_bytes, 3 + 2 + 14);
    let accepted: usize = frames.iter().map(wire_len).sum();
    assert_eq!(stream.len(), accepted + stats.garbage_bytes as usize + left);
}

#[test]
fn counters_saturate() {
    let mut d = Deframer::new();
    let mut near_full = d.stats();
    near_full.garbage_bytes = u32::MAX - 1;
    near_full.frames = u32::MAX;
    d.set_stats(near_full);
    d.push(&[1, 2, 3], |_| {});
    assert_eq!(d.stats().garbage_bytes, u32::MAX);
    d.push(&frame(0x30, &[]), |_| {});
    assert_eq!(d.stats().frames, u32::MAX);
}

// ---------------------------------------------------------------- I1 chunking

/// Bytes that make awkward garbage: many 0xC8 and small length bytes.
fn garbage_byte() -> impl Strategy<Value = u8> {
    prop_oneof![Just(0xC8u8), 0u8..=64, any::<u8>()]
}

fn valid_frame() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        proptest::array::uniform16(0u16..2048).prop_map(|raw| frame(0x16, &pack(&raw))),
        proptest::collection::vec(any::<u8>(), 10).prop_map(|p| frame(0x14, &p)),
        proptest::collection::vec(any::<u8>(), 8).prop_map(|p| frame(0x08, &p)),
        (any::<u8>(), proptest::collection::vec(any::<u8>(), 0..=60))
            .prop_map(|(ty, p)| frame(ty, &p)),
    ]
}

fn mixed_stream() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(
        prop_oneof![
            valid_frame(),
            proptest::collection::vec(garbage_byte(), 0..40),
        ],
        0..30,
    )
    .prop_map(|parts| parts.concat())
}

proptest! {
    #[test]
    fn i1_chunking_does_not_change_frames_or_counters(
        stream in mixed_stream(),
        cuts in proptest::collection::vec(any::<usize>(), 0..20),
    ) {
        let whole = run_whole(&stream);
        prop_assert_eq!(&run_cut(&stream, &cuts), &whole);
        prop_assert_eq!(&run_bytewise(&stream), &whole);
    }
}

#[test]
fn i1_the_mixed_stream_does_deliver_frames() {
    // Guard against a strategy that never produces a frame.
    let mut stream = frame(0x16, &pack(&[992; 16]));
    stream.extend([5, 6, 0xC8]);
    stream.extend(frame(0x14, &[1; 10]));
    let (frames, _, _) = run_bytewise(&stream);
    assert_eq!(frames.len(), 2);
}

// ---------------------------------------------------------------- I2 arbitrary input

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn i2_arbitrary_bytes_never_panic_and_buffer_stays_bounded(
        bytes in proptest::collection::vec(garbage_byte(), 4096..9000),
        chunk in 1usize..200,
    ) {
        let mut d = Deframer::new();
        let mut accepted = 0usize;
        for piece in bytes.chunks(chunk) {
            d.push(piece, |f| accepted += wire_len(&own(f)));
            prop_assert!(d.buffered() <= MAX_FRAME_LEN);
        }
        let s = d.stats();
        prop_assert_eq!(bytes.len(), accepted + s.garbage_bytes as usize + d.buffered());
        prop_assert!(s.other_frames <= s.frames);
    }

    #[test]
    fn i2_uniform_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 4096..9000)) {
        let (_, _, left) = run_bytewise(&bytes);
        prop_assert!(left <= MAX_FRAME_LEN);
    }
}

// ---------------------------------------------------------------- I3 false start

/// Bytes below 0xC8, from a small xorshift generator, so a stray 0xC8 is the only candidate.
fn filler(seed: u32, n: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x % 0xC8) as u8
        })
        .collect()
}

#[test]
fn i3_stray_address_byte_never_swallows_a_real_frame() {
    // Every offset 1 to 63 of the real frame behind a stray 0xC8 and every plausible length byte
    // (2 to 62), with 4 different fillers each. The false candidate is rejected and the scan restarts
    // one byte on, so the real frame is found. The one way to lose it is a CRC-8 collision of the false
    // candidate (it spans garbage and the start of the real frame, and passes with probability 1/256):
    // such a case is counted, and in every other case the real frame must come out, once.
    let real = frame(0x16, &pack(&[992; 16]));
    let expected = Owned::Rc(RcChannels { raw: [992; 16] });
    let (mut found, mut collisions) = (0, 0);
    for prefix_len in 1..=63usize {
        for len_byte in 2..=62u8 {
            for variant in 0..4u32 {
                let mut stream = filler(prefix_len as u32 * 1000 + variant, prefix_len);
                stream[0] = 0xC8;
                if prefix_len >= 2 {
                    stream[1] = len_byte;
                }
                stream.extend(&real);
                stream.extend(filler(7 + variant, 64));
                let total = usize::from(stream[1]) + 2;
                let collides = prefix_len >= 2 && crc8(&stream[2..total - 1]) == stream[total - 1];
                let frames = run_whole(&stream).0;
                assert_eq!(run_bytewise(&stream).0, frames);
                if collides {
                    collisions += 1;
                    assert!(!frames.contains(&expected) || frames.len() > 1);
                } else {
                    found += 1;
                    assert_eq!(
                        frames,
                        std::slice::from_ref(&expected),
                        "prefix {prefix_len}, length byte {len_byte}, variant {variant}"
                    );
                }
            }
        }
    }
    assert_eq!((found, collisions), EXPECTED_I3);
}

const EXPECTED_I3: (usize, usize) = (15324, 48);

// ---------------------------------------------------------------- I4 bit flips

#[derive(Debug, Default, PartialEq, Eq)]
struct FlipOutcomes {
    bits: usize,
    rejected: usize,
    same_as_original: usize,
    different_valid: std::vec::Vec<usize>,
}

/// The decoded form of a reference frame, by feeding it unflipped. Err when it is not accepted
/// exactly once.
fn reference(bytes: &[u8]) -> Result<Owned, &'static str> {
    let (frames, stats, left) = run_whole(bytes);
    if frames.len() != 1 || stats.garbage_bytes != 0 || left != 0 {
        return Err("reference frame not accepted cleanly");
    }
    Ok(frames[0].clone())
}

fn flip_all(bytes: &[u8], trailer: &[u8]) -> Result<FlipOutcomes, &'static str> {
    let original = reference(bytes)?;
    let mut outcomes = FlipOutcomes::default();
    for bit in 0..bytes.len() * 8 {
        let mut flipped = bytes.to_vec();
        flipped[bit / 8] ^= 1 << (bit % 8);
        flipped.extend(trailer); // lets any candidate with a larger length byte finish
        let (frames, stats, _) = run_whole(&flipped);
        outcomes.bits += 1;
        if frames.is_empty() {
            assert!(
                stats.garbage_bytes > 0 && (stats.bad_length + stats.crc_errors > 0 || bit < 8),
                "bit {bit} rejected without being counted: {stats:?}"
            );
            outcomes.rejected += 1;
        } else if frames.contains(&original) {
            outcomes.same_as_original += 1;
        } else {
            outcomes.different_valid.push(bit);
        }
    }
    Ok(outcomes)
}

fn reference_frames() -> [(&'static str, Vec<u8>); 4] {
    let mut raw = [0u16; 16];
    for (i, v) in raw.iter_mut().enumerate() {
        *v = 172 + (i as u16) * 100;
    }
    let mut battery = [0u8; 12];
    let battery_len = encode_battery(
        &Battery {
            voltage_dv: 168,
            current_da: 25,
            capacity_used_mah: 1234,
            remaining_percent: 87,
        },
        &mut battery,
    )
    .unwrap();
    assert_eq!(battery_len, BATTERY_FRAME_LEN);
    [
        ("rc", frame(0x16, &pack(&raw))),
        (
            "link statistics",
            frame(0x14, &[16, 20, 100, 0xF6, 1, 3, 4, 42, 99, 5]),
        ),
        ("battery", battery.to_vec()),
        ("other (flight mode)", frame(0x21, b"ACRO\0")),
    ]
}

#[test]
fn i4_every_single_bit_flip_is_rejected_or_shown() {
    // Exact outcomes per reference frame, for every single-bit flip of the whole frame, each followed
    // by 64 bytes of a trailer so that a candidate with a larger length byte can finish.
    //
    // Trailer of pseudo-random bytes: (bits flipped, rejected, same as the original, different
    // valid frame). Every flip is rejected: CRC-8 with polynomial 0xD5 detects every single-bit
    // error in the bytes it covers (type, payload, CRC); a flip in the address byte makes the frame a
    // run of garbage; a flip in the length byte makes the CRC land on another byte, which does not match.
    //
    // Trailer of zeros: the same, except that a flip in the length byte that LENGTHENS the frame can be
    // accepted, because the CRC of a message followed by its own CRC is 0 and zero bytes keep it 0, so
    // the trailer's zero byte "matches". These are listed explicitly (bit numbers; bits 8 to 15 are the
    // length byte): each is a different frame (the original type with a longer payload), never the
    // original. In a real stream the bytes behind a frame are the next frame, which matches with
    // probability 1/256.
    let expected = [
        ("rc", 208, &[8usize, 9, 10, 13][..]),
        ("link statistics", 112, &[8, 9, 12, 13][..]),
        ("battery", 96, &[8, 10, 12, 13][..]),
        ("other (flight mode)", 72, &[11, 12, 13][..]),
    ];
    for ((name, bytes), (expected_name, bits, listed)) in reference_frames().iter().zip(expected) {
        assert_eq!(*name, expected_name);
        assert_eq!(bytes.len() * 8, bits);

        let o = flip_all(bytes, &filler(5, 64)).unwrap();
        assert_eq!(
            (
                o.bits,
                o.rejected,
                o.same_as_original,
                o.different_valid.len()
            ),
            (bits, bits, 0, 0),
            "{name}, pseudo-random trailer"
        );

        let o = flip_all(bytes, &[0; 64]).unwrap();
        assert_eq!(o.same_as_original, 0, "{name}");
        assert_eq!(o.bits, bits);
        assert_eq!(o.different_valid, listed, "{name}, zero trailer");
        assert_eq!(o.rejected, bits - listed.len(), "{name}");
        assert!(listed.iter().all(|bit| (8..16).contains(bit)));
    }
}

#[test]
fn i4_negative_controls() {
    // 1. The harness refuses reference frames built with a wrong polynomial (CRC-8/ATM, 0x07): the
    //    unflipped frame is not accepted, so the test would fail.
    for (name, bytes) in reference_frames() {
        let mut wrong = bytes.clone();
        let n = wrong.len();
        wrong[n - 1] = crc8_with_poly(0x07, &wrong[2..n - 1]);
        assert_ne!(
            wrong, bytes,
            "{name}: the wrong polynomial gives the same CRC"
        );
        assert!(flip_all(&wrong, &[0; 64]).is_err(), "{name}");
        let (frames, stats, _) = run_whole(&wrong);
        assert!(frames.is_empty() && stats.crc_errors >= 1, "{name}");
    }
    // 2. The classifier does report a silent acceptance: "flipping" no bit gives the original back.
    for (name, bytes) in reference_frames() {
        let original = reference(&bytes).unwrap();
        let mut unflipped = bytes.clone();
        unflipped.extend([0u8; 64]);
        let (frames, _, _) = run_whole(&unflipped);
        assert!(frames.contains(&original), "{name}");
    }
    // 3. A flip that also fixes the CRC is accepted as a different frame (one the classifier would
    //    list under different_valid): flip bit 0 of the type byte and recompute the CRC.
    for (name, bytes) in reference_frames() {
        let mut forged = bytes.clone();
        let n = forged.len();
        forged[2] ^= 1;
        forged[n - 1] = crc8(&forged[2..n - 1]);
        let (frames, _, _) = run_whole(&forged);
        assert_eq!(frames.len(), 1, "{name}");
        assert_ne!(frames[0], reference(&bytes).unwrap(), "{name}");
    }
}

fn crc8_with_poly(poly: u8, data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &byte in data {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ poly
            } else {
                crc << 1
            };
        }
    }
    crc
}

// ---------------------------------------------------------------- battery encoder

#[test]
fn battery_hand_vector() {
    // 16.8 V = 168 = 0x00A8, 2.5 A = 25 = 0x0019, 1234 mAh = 0x0004D2, 87 % = 0x57.
    let mut out = [0xAAu8; 64];
    let n = encode_battery(
        &Battery {
            voltage_dv: 168,
            current_da: 25,
            capacity_used_mah: 1234,
            remaining_percent: 87,
        },
        &mut out,
    )
    .unwrap();
    assert_eq!(n, 12);
    assert_eq!(
        &out[..11],
        &[
            0xC8, 0x0A, 0x08, 0x00, 0xA8, 0x00, 0x19, 0x00, 0x04, 0xD2, 0x57
        ]
    );
    assert_eq!(out[11], crc8(&out[2..11]));
    assert!(out[12..].iter().all(|&b| b == 0xAA), "wrote past the frame");
    let (frames, stats, _) = run_whole(&out[..n]);
    assert_eq!(frames, [Owned::Other(0x08, out[3..11].to_vec())]);
    assert_eq!(stats.other_frames, 1);
}

#[test]
fn battery_limits_and_errors() {
    let ok = Battery {
        voltage_dv: u16::MAX,
        current_da: u16::MAX,
        capacity_used_mah: MAX_CAPACITY_MAH,
        remaining_percent: 100,
    };
    let mut out = [0u8; 64];
    assert_eq!(encode_battery(&ok, &mut out), Ok(12));
    assert_eq!(
        &out[3..11],
        &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 100]
    );

    let mut untouched = [0x5Au8; 64];
    let capacity = Battery {
        capacity_used_mah: MAX_CAPACITY_MAH + 1,
        ..ok
    };
    assert_eq!(
        encode_battery(&capacity, &mut untouched),
        Err(EncodeError::CapacityOutOfRange)
    );
    let percent = Battery {
        remaining_percent: 101,
        ..ok
    };
    assert_eq!(
        encode_battery(&percent, &mut untouched),
        Err(EncodeError::PercentOutOfRange)
    );
    assert_eq!(untouched, [0x5A; 64]);
    assert_eq!(
        encode_battery(&ok, &mut out[..11]),
        Err(EncodeError::BufferTooSmall)
    );
    assert_eq!(
        encode_battery(&ok, &mut []),
        Err(EncodeError::BufferTooSmall)
    );
    assert_eq!(encode_battery(&ok, &mut out[..12]), Ok(12));
}

// ---------------------------------------------------------------- I5 differential against uf-crsf

mod differential {
    use super::*;
    use uf_crsf::packets::{Battery as UfBattery, LinkStatistics as UfLink, RcChannelsPacked};
    use uf_crsf::{CrsfParser, Packet, PacketAddress, write_packet_to_buffer};

    fn uf_parse(bytes: &[u8]) -> Packet {
        let mut parser = CrsfParser::new();
        let mut input = bytes;
        parser.next_packet(&mut input).unwrap().unwrap()
    }

    proptest! {
        #[test]
        fn i5_rc_payloads_decode_like_uf_crsf(payload in proptest::collection::vec(any::<u8>(), 22)) {
            let bytes = frame(0x16, &payload);
            let Packet::RCChannels(theirs) = uf_parse(&bytes) else {
                return Err(TestCaseError::fail("uf-crsf did not return RC channels"));
            };
            prop_assert_eq!(decode_rc(&payload), Owned::Rc(RcChannels { raw: theirs.0 }));
        }

        #[test]
        fn i5_rc_frames_written_by_uf_crsf_decode_to_the_same_values(
            raw in proptest::array::uniform16(0u16..2048),
        ) {
            let mut buf = [0u8; 64];
            let n = write_packet_to_buffer(&mut buf, PacketAddress::FlightController, &RcChannelsPacked(raw)).unwrap();
            prop_assert_eq!(&buf[..n], &frame(0x16, &pack(&raw))[..]);
            prop_assert_eq!(run_whole(&buf[..n]).0, [Owned::Rc(RcChannels { raw })]);
        }

        #[test]
        fn i5_link_statistics_decode_like_uf_crsf(payload in proptest::collection::vec(any::<u8>(), 10)) {
            let bytes = frame(0x14, &payload);
            let Packet::LinkStatistics(UfLink {
                uplink_rssi_1, uplink_rssi_2, uplink_link_quality, uplink_snr, active_antenna,
                rf_mode, uplink_tx_power, downlink_rssi, downlink_link_quality, downlink_snr,
            }) = uf_parse(&bytes) else {
                return Err(TestCaseError::fail("uf-crsf did not return link statistics"));
            };
            let ours = run_whole(&bytes).0;
            prop_assert_eq!(ours, [Owned::Link(LinkStatistics {
                uplink_rssi_1, uplink_rssi_2, uplink_link_quality, uplink_snr, active_antenna,
                rf_mode, uplink_tx_power, downlink_rssi, downlink_link_quality, downlink_snr,
            })]);
        }

        #[test]
        fn i5_our_battery_frame_parses_in_uf_crsf_to_the_same_values(
            voltage_dv in any::<u16>(),
            current_da in any::<u16>(),
            capacity_used_mah in 0..=MAX_CAPACITY_MAH,
            remaining_percent in 0u8..=100,
        ) {
            let mut out = [0u8; 64];
            let n = encode_battery(&Battery { voltage_dv, current_da, capacity_used_mah, remaining_percent }, &mut out).unwrap();
            let Packet::Battery(theirs) = uf_parse(&out[..n]) else {
                return Err(TestCaseError::fail("uf-crsf did not return a battery packet"));
            };
            // uf-crsf reads the two 16-bit fields as i16: the same bits.
            prop_assert_eq!(theirs.voltage as u16, voltage_dv);
            prop_assert_eq!(theirs.current as u16, current_da);
            prop_assert_eq!(theirs.capacity_used, capacity_used_mah);
            prop_assert_eq!(theirs.remaining, remaining_percent);
            prop_assert_eq!(theirs.id, 0);
            // And uf-crsf writes the same bytes.
            let mut buf = [0u8; 64];
            let theirs = UfBattery { voltage: voltage_dv as i16, current: current_da as i16, capacity_used: capacity_used_mah, remaining: remaining_percent, id: 0 };
            let m = write_packet_to_buffer(&mut buf, PacketAddress::FlightController, &theirs).unwrap();
            prop_assert_eq!(&buf[..m], &out[..n]);
        }
    }
}

// ---------------------------------------------------------------- clear

fn rc_frame() -> Vec<u8> {
    frame(0x16, &pack(&[992; 16]))
}

#[test]
fn clear_drops_a_partial_frame_and_the_next_frame_comes_out_clean() {
    let whole = rc_frame();
    let mut d = Deframer::new();
    let mut out = Vec::new();
    d.push(&whole[..10], |f| out.push(own(f)));
    assert_eq!(d.buffered(), 10);
    d.clear();
    assert_eq!(d.buffered(), 0);
    d.push(&whole, |f| out.push(own(f)));
    assert_eq!(out.len(), 1);
    assert!(matches!(out[0], Owned::Rc(_)));
    let s = d.stats();
    assert_eq!(s.garbage_bytes, 10);
    assert_eq!(s.frames, 1);
    assert_eq!(s.crc_errors, 0);
    assert_eq!(s.bad_length, 0);
    assert_eq!(d.buffered(), 0);
}

#[test]
fn clear_negative_control_without_clear_the_partial_bytes_cause_a_crc_error() {
    let whole = rc_frame();
    let mut d = Deframer::new();
    let mut out = Vec::new();
    d.push(&whole[..10], |f| out.push(own(f)));
    d.push(&whole, |f| out.push(own(f)));
    // The stale candidate (26 bytes announced) is judged on 10 old and 16 new bytes: CRC error.
    let s = d.stats();
    assert_eq!(s.crc_errors, 1);
    // The real frame is still found behind it, but only after the stale candidate was rejected.
    assert_eq!(out.len(), 1);
    assert_eq!(s.frames, 1);
    // Same accounting of garbage here, but it is reached through a CRC error that clear() avoids.
    assert_eq!(s.garbage_bytes, 10);
    assert_ne!(s, {
        let mut c = Deframer::new();
        c.push(&whole[..10], |_| {});
        c.clear();
        c.push(&whole, |_| {});
        c.stats()
    });
}

#[test]
fn clear_negative_control_a_longer_partial_delays_the_frame() {
    // A candidate announcing 62 (the maximum) swallows the next frame's bytes until 64 have arrived.
    let whole = rc_frame();
    let partial = [0xC8, 62, 0x30, 0, 0];
    let mut without = Deframer::new();
    let mut n_without = 0;
    without.push(&partial, |_| n_without += 1);
    without.push(&whole[..20], |_| n_without += 1);
    assert_eq!(n_without, 0);
    without.push(&whole[20..], |_| n_without += 1);
    // 5 + 26 = 31 < 64 bytes: the real frame has fully arrived and is still held back.
    assert_eq!(n_without, 0);
    assert_eq!(without.buffered(), 31);

    let mut with = Deframer::new();
    let mut n_with = 0;
    with.push(&partial, |_| n_with += 1);
    with.clear();
    with.push(&whole, |_| n_with += 1);
    assert_eq!(n_with, 1);
    assert_eq!(with.buffered(), 0);
}

#[test]
fn clear_on_an_empty_deframer_changes_nothing() {
    let mut d = Deframer::new();
    d.clear();
    assert_eq!(d.stats(), Stats::default());
    assert_eq!(d.buffered(), 0);

    let mut d = Deframer::new();
    d.push(&rc_frame(), |_| {});
    d.push(&[1, 2, 0xC8, 0xFF], |_| {});
    // 0xC8 0xFF is a bad length: both bytes are garbage, the buffer is empty.
    assert_eq!(d.buffered(), 0);
    let before = d.stats();
    d.clear();
    assert_eq!(d.stats(), before);
    assert_eq!(d.buffered(), 0);
}

#[test]
fn clear_counts_every_byte_and_saturates() {
    let mut d = Deframer::new();
    d.push(&[0xC8, 62, 1, 2], |_| {});
    assert_eq!(d.buffered(), 4);
    d.clear();
    assert_eq!(d.stats().garbage_bytes, 4);
    assert_eq!(d.stats().frames, 0);

    let mut near_full = d.stats();
    near_full.garbage_bytes = u32::MAX - 1;
    d.set_stats(near_full);
    d.push(&[0xC8, 62, 1, 2], |_| {});
    d.clear();
    assert_eq!(d.stats().garbage_bytes, u32::MAX);
}

proptest! {
    #[test]
    fn clear_at_random_points_keeps_the_accounting_and_equals_fresh_deframers(
        stream in mixed_stream(),
        cuts in proptest::collection::vec(any::<usize>(), 0..8),
    ) {
        let mut points: Vec<usize> = cuts.iter().map(|c| c % (stream.len() + 1)).collect();
        points.push(0);
        points.push(stream.len());
        points.sort_unstable();

        let mut d = Deframer::new();
        let mut got: Vec<Owned> = Vec::new();
        let mut expected: Vec<Owned> = Vec::new();
        let mut accepted = 0usize;
        for w in points.windows(2) {
            let segment = &stream[w[0]..w[1]];
            d.push(segment, |f| {
                let o = own(f);
                accepted += wire_len(&o);
                got.push(o);
            });
            prop_assert!(d.buffered() <= MAX_FRAME_LEN);
            d.clear();
            prop_assert_eq!(d.buffered(), 0);
            let (frames, _, _) = run_whole(segment);
            expected.extend(frames);
        }
        prop_assert_eq!(&got, &expected);
        let s = d.stats();
        prop_assert_eq!(stream.len(), accepted + s.garbage_bytes as usize + d.buffered());
        prop_assert_eq!(s.frames as usize, got.len());
    }
}
