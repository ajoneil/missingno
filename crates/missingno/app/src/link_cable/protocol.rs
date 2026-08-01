/// BGB 1.4 link cable protocol packet format.
///
/// All packets are exactly 8 bytes, little-endian on the wire.
/// See <https://bgb.bircd.org/bgblink.html> for the full specification.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet {
    pub command: u8,
    pub b2: u8,
    pub b3: u8,
    pub b4: u8,
    pub timestamp: u32,
}

// Command IDs
const VERSION: u8 = 1;
const SYNC1: u8 = 104;
const SYNC2: u8 = 105;
const SYNC3: u8 = 106;
const STATUS: u8 = 108;

#[allow(dead_code)]
impl Packet {
    pub const SIZE: usize = 8;

    pub fn to_bytes(self) -> [u8; 8] {
        let ts = self.timestamp.to_le_bytes();
        [
            self.command,
            self.b2,
            self.b3,
            self.b4,
            ts[0],
            ts[1],
            ts[2],
            ts[3],
        ]
    }

    pub fn from_bytes(buf: &[u8; 8]) -> Self {
        Packet {
            command: buf[0],
            b2: buf[1],
            b3: buf[2],
            b4: buf[3],
            timestamp: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
        }
    }

    /// Protocol version handshake (must be first packet sent/received).
    pub fn version() -> Self {
        Packet {
            command: VERSION,
            b2: 1, // major
            b3: 4, // minor
            b4: 0,
            timestamp: 0,
        }
    }

    /// Emulator status (running/paused). Sent after version handshake.
    pub fn status(running: bool) -> Self {
        let mut flags = 0u8;
        if running {
            flags |= 1;
        }
        // bit 2: support_reconnect — advertise for future use
        flags |= 4;
        Packet {
            command: STATUS,
            b2: flags,
            b3: 0,
            b4: 0,
            timestamp: 0,
        }
    }

    /// Master transfer: send our serial byte and request the slave's byte.
    /// `control` bit 0 = 1 (internal clock), bit 7 = 1 (transfer enabled).
    pub fn sync1(data: u8, timestamp: u32) -> Self {
        Packet {
            command: SYNC1,
            b2: data,
            b3: 0x81, // bit 0 = internal clock, bit 7 = transfer active
            b4: 0,
            timestamp,
        }
    }

    /// Slave response: our serial byte in reply to a Sync1.
    pub fn sync2(data: u8) -> Self {
        Packet {
            command: SYNC2,
            b2: data,
            b3: 0x80, // bit 7 = transfer active
            b4: 0,
            timestamp: 0,
        }
    }

    /// Timestamp synchronization (sent when idle to keep clocks aligned).
    pub fn sync3_timestamp(timestamp: u32) -> Self {
        Packet {
            command: SYNC3,
            b2: 0,
            b3: 0,
            b4: 0,
            timestamp,
        }
    }

    /// Transfer acknowledgment (b2=1).
    pub fn sync3_ack() -> Self {
        Packet {
            command: SYNC3,
            b2: 1,
            b3: 0,
            b4: 0,
            timestamp: 0,
        }
    }

    pub fn is_version(&self) -> bool {
        self.command == VERSION
    }

    pub fn is_sync1(&self) -> bool {
        self.command == SYNC1
    }

    pub fn is_sync2(&self) -> bool {
        self.command == SYNC2
    }

    pub fn is_sync3(&self) -> bool {
        self.command == SYNC3
    }

    pub fn is_status(&self) -> bool {
        self.command == STATUS
    }
}
