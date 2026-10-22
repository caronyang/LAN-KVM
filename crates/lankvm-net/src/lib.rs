//! Wire format for the LAN-KVM P0 loopback/LAN PoC.
//!
//! This is intentionally small and dependency-free. The production protocol
//! will use QUIC and a versioned, authenticated message set; this fixed-size
//! datagram is only for the first latency/edge-switching prototype.

pub const MAGIC: [u8; 4] = *b"LKVM";
pub const VERSION: u8 = 1;
pub const PACKET_LEN: usize = 28;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PacketKind {
    Hello = 1,
    Switch = 2,
    Motion = 3,
    Button = 4,
    Key = 5,
    Wheel = 6,
    Release = 7,
    Ping = 8,
    Pong = 9,
}

impl PacketKind {
    pub const fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::Hello,
            2 => Self::Switch,
            3 => Self::Motion,
            4 => Self::Button,
            5 => Self::Key,
            6 => Self::Wheel,
            7 => Self::Release,
            8 => Self::Ping,
            9 => Self::Pong,
            _ => return None,
        })
    }
}

/// Fixed-size packet. Field meanings depend on `kind`:
///
/// - Switch: `a` = entry Y, `b` = target width, `c` = target height
/// - Motion: `a` = dx, `b` = dy
/// - Button: `a` = button id, `b` = down(1)/up(0)
/// - Key: `a` = scan code, `b` = down(1)/up(0), `c` = flags
/// - Wheel: `a` = delta, `b` = horizontal(1)/vertical(0)
/// - Release: `a` = reason
/// - Ping/Pong: `a` = timestamp low 32 bits
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packet {
    pub kind: PacketKind,
    pub flags: u16,
    pub seq: u32,
    pub a: i32,
    pub b: i32,
    pub c: i32,
    pub d: i32,
}

impl Packet {
    pub const fn new(kind: PacketKind, seq: u32) -> Self {
        Self {
            kind,
            flags: 0,
            seq,
            a: 0,
            b: 0,
            c: 0,
            d: 0,
        }
    }

    pub fn encode(self) -> [u8; PACKET_LEN] {
        let mut out = [0u8; PACKET_LEN];
        out[0..4].copy_from_slice(&MAGIC);
        out[4] = VERSION;
        out[5] = self.kind as u8;
        out[6..8].copy_from_slice(&self.flags.to_le_bytes());
        out[8..12].copy_from_slice(&self.seq.to_le_bytes());
        out[12..16].copy_from_slice(&self.a.to_le_bytes());
        out[16..20].copy_from_slice(&self.b.to_le_bytes());
        out[20..24].copy_from_slice(&self.c.to_le_bytes());
        out[24..28].copy_from_slice(&self.d.to_le_bytes());
        out
    }

    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < PACKET_LEN || buf[0..4] != MAGIC || buf[4] != VERSION {
            return None;
        }
        Some(Self {
            kind: PacketKind::from_u8(buf[5])?,
            flags: u16::from_le_bytes([buf[6], buf[7]]),
            seq: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
            a: i32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
            b: i32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]),
            c: i32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]),
            d: i32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let packet = Packet {
            kind: PacketKind::Motion,
            flags: 0x1234,
            seq: 42,
            a: -7,
            b: 9,
            c: 1080,
            d: -1,
        };
        assert_eq!(Packet::decode(&packet.encode()), Some(packet));
    }

    #[test]
    fn rejects_bad_magic_and_version() {
        let mut data = Packet::new(PacketKind::Ping, 1).encode();
        data[0] = b'X';
        assert_eq!(Packet::decode(&data), None);

        let mut data = Packet::new(PacketKind::Ping, 1).encode();
        data[4] = 99;
        assert_eq!(Packet::decode(&data), None);
    }

    #[test]
    fn rejects_short_and_unknown_kind() {
        assert_eq!(Packet::decode(&[0u8; 10]), None);
        let mut data = Packet::new(PacketKind::Ping, 1).encode();
        data[5] = 200;
        assert_eq!(Packet::decode(&data), None);
    }
}
