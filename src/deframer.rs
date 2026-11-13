//! The sans-IO deframer.

use crate::ADDRESS;
use crate::crc::crc8;
use crate::decode::Frame;

/// Longest frame, address and CRC included.
pub const MAX_FRAME_LEN: usize = 64;
/// Smallest and largest value of the length byte (type + payload + CRC).
const MIN_LEN_BYTE: usize = 2;
const MAX_LEN_BYTE: usize = 62;

/// Counters of a [`Deframer`]. All saturate at `u32::MAX`.
///
/// Every byte pushed is exactly one of: still buffered, part of an accepted frame, or counted in
/// `garbage_bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub struct Stats {
    /// Bytes discarded without being part of an accepted frame: bytes before an address byte, and the
    /// first byte of a candidate frame rejected for its length or CRC.
    pub garbage_bytes: u32,
    /// Candidate frames (address byte 0xC8 followed by a length byte) whose length byte is outside
    /// 2 to 62.
    pub bad_length: u32,
    /// Candidate frames whose CRC did not match.
    pub crc_errors: u32,
    /// Frames accepted, of every type.
    pub frames: u32,
    /// Accepted frames that came out as [`Frame::Other`]: types not decoded, and 0x16 or 0x14 frames
    /// of the wrong payload length.
    pub other_frames: u32,
}

/// Sans-IO CRSF deframer.
///
/// Holds at most [`MAX_FRAME_LEN`] bytes. The result depends only on the byte sequence, not on how it
/// is split across calls.
#[derive(Debug, Clone)]
pub struct Deframer {
    buf: [u8; MAX_FRAME_LEN],
    len: usize,
    stats: Stats,
}

impl Default for Deframer {
    fn default() -> Self {
        Self::new()
    }
}

impl Deframer {
    /// An empty deframer.
    pub const fn new() -> Self {
        Self {
            buf: [0; MAX_FRAME_LEN],
            len: 0,
            stats: Stats {
                garbage_bytes: 0,
                bad_length: 0,
                crc_errors: 0,
                frames: 0,
                other_frames: 0,
            },
        }
    }

    /// The counters.
    pub const fn stats(&self) -> Stats {
        self.stats
    }

    /// Feeds `bytes` in any chunking, down to one byte per call. `on_frame` is called once per
    /// accepted frame, in stream order, before the next byte is looked at.
    ///
    /// A frame is accepted only if the address byte is 0xC8, the length byte is 2 to 62 and the CRC-8
    /// (polynomial 0xD5) over type and payload matches. On a bad length or a bad CRC one byte is
    /// dropped and the scan restarts at the next byte, so a stray 0xC8 never swallows a real frame
    /// that follows it. A candidate with a plausible length byte, however, is waited for to its
    /// end: a real frame behind it is handed out when that many bytes (at most 64 from the stray
    /// 0xC8) have arrived.
    pub fn push<F: FnMut(Frame<'_>)>(&mut self, bytes: &[u8], mut on_frame: F) {
        for &byte in bytes {
            if let Some(slot) = self.buf.get_mut(self.len) {
                *slot = byte;
                self.len += 1;
            }
            self.scan(&mut on_frame);
        }
    }

    fn scan<F: FnMut(Frame<'_>)>(&mut self, on_frame: &mut F) {
        loop {
            let lead = self
                .buf
                .get(..self.len)
                .and_then(|held| held.iter().position(|&b| b == ADDRESS))
                .unwrap_or(self.len);
            if lead > 0 {
                self.discard(lead);
            }
            if self.len < 2 {
                return;
            }
            let len_byte = usize::from(self.buf.get(1).copied().unwrap_or(0));
            if !(MIN_LEN_BYTE..=MAX_LEN_BYTE).contains(&len_byte) {
                self.stats.bad_length = self.stats.bad_length.saturating_add(1);
                self.discard(1);
                continue;
            }
            let total = len_byte + 2;
            if self.len < total {
                return;
            }
            let (Some(&frame_type), Some(payload), Some(&crc)) = (
                self.buf.get(2),
                self.buf.get(3..total - 1),
                self.buf.get(total - 1),
            ) else {
                return;
            };
            let body = self.buf.get(2..total - 1).unwrap_or(&[]);
            if crc8(body) != crc {
                self.stats.crc_errors = self.stats.crc_errors.saturating_add(1);
                self.discard(1);
                continue;
            }
            let frame = Frame::decode(frame_type, payload);
            self.stats.frames = self.stats.frames.saturating_add(1);
            if matches!(frame, Frame::Other { .. }) {
                self.stats.other_frames = self.stats.other_frames.saturating_add(1);
            }
            on_frame(frame);
            self.remove(total);
        }
    }

    fn discard(&mut self, n: usize) {
        let n = n.min(self.len);
        self.stats.garbage_bytes = self
            .stats
            .garbage_bytes
            .saturating_add(u32::try_from(n).unwrap_or(u32::MAX));
        self.remove(n);
    }

    fn remove(&mut self, n: usize) {
        let n = n.min(self.len);
        self.buf.copy_within(n..self.len, 0);
        self.len -= n;
    }

    #[cfg(test)]
    pub(crate) fn set_stats(&mut self, stats: Stats) {
        self.stats = stats;
    }

    #[cfg(test)]
    pub(crate) fn buffered(&self) -> usize {
        self.len
    }
}
