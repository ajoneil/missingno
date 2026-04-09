//! BGB 1.4 TCP link cable protocol implementation.
//!
//! Allows Missingno to connect to other emulators (including other Missingno
//! instances and BGB) over TCP for Game Boy serial link cable emulation.
//!
//! See <https://bgb.bircd.org/bgblink.html> for the protocol specification.

mod protocol;

use std::io::{self, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};

use missingno_gb::serial_transfer::SerialLink;
use protocol::Packet;

/// Transfer state machine.
enum State {
    /// Listening for incoming TCP connections (server mode, pre-connect).
    Listening,
    /// Connected and idle — no active transfer.
    Idle,
    /// We received Sync1 from the remote (they are master, we are slave).
    /// Clocking in their byte while shifting out ours.
    SlaveTransfer {
        remote_byte: u8,
        bits_shifted: u8,
        our_byte: u8,
    },
    /// We are master (internal clock). We sent Sync1 and have the remote's
    /// response byte buffered for bit-by-bit delivery via exchange_bit().
    MasterTransfer {
        response_byte: u8,
        bits_shifted: u8,
    },
}

pub struct BgbLink {
    listener: Option<TcpListener>,
    stream: Option<TcpStream>,
    state: State,
    /// Partial receive buffer for assembling 8-byte packets from TCP.
    recv_buf: [u8; Packet::SIZE],
    recv_len: usize,
    /// 2 MHz timestamp counter (incremented by 2 per M-cycle).
    timestamp: u32,
    /// Buffered Sync1 packet from tick(), delivered by clock().
    pending_sync1: Option<Packet>,
}

#[derive(Debug)]
pub enum BgbError {
    Io(io::Error),
    HandshakeFailed(String),
}

impl From<io::Error> for BgbError {
    fn from(e: io::Error) -> Self {
        BgbError::Io(e)
    }
}

impl std::fmt::Display for BgbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BgbError::Io(e) => write!(f, "I/O error: {e}"),
            BgbError::HandshakeFailed(msg) => write!(f, "BGB handshake failed: {msg}"),
        }
    }
}

impl BgbLink {
    /// Connect to a BGB server. Performs the version/status handshake (blocking).
    pub fn connect(addr: &str) -> Result<Self, BgbError> {
        let stream = TcpStream::connect(addr)?;
        stream.set_nodelay(true)?;

        let mut link = BgbLink {
            listener: None,
            stream: Some(stream),
            state: State::Idle,
            recv_buf: [0; Packet::SIZE],
            recv_len: 0,
            timestamp: 0, pending_sync1: None,
        };

        link.handshake()?;
        link.stream.as_ref().unwrap().set_nonblocking(true)?;

        Ok(link)
    }

    /// Start listening for incoming BGB connections. Returns immediately;
    /// the actual accept happens asynchronously in clock().
    pub fn listen(port: u16) -> Result<Self, BgbError> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        listener.set_nonblocking(true)?;
        eprintln!("link cable: listening on port {port}");

        Ok(BgbLink {
            listener: Some(listener),
            stream: None,
            state: State::Listening,
            recv_buf: [0; Packet::SIZE],
            recv_len: 0,
            timestamp: 0, pending_sync1: None,
        })
    }

    /// Perform the BGB version + status handshake (blocking).
    fn handshake(&mut self) -> Result<(), BgbError> {
        let stream = self.stream.as_mut().unwrap();

        // Send version
        stream.write_all(&Packet::version().to_bytes())?;

        // Receive version
        let mut buf = [0u8; Packet::SIZE];
        stream.read_exact(&mut buf)?;
        let version = Packet::from_bytes(&buf);
        if !version.is_version() {
            return Err(BgbError::HandshakeFailed(format!(
                "expected Version packet, got command {}",
                version.command
            )));
        }

        // Send status (running)
        stream.write_all(&Packet::status(true).to_bytes())?;

        // Receive status
        stream.read_exact(&mut buf)?;
        let status = Packet::from_bytes(&buf);
        if !status.is_status() {
            return Err(BgbError::HandshakeFailed(format!(
                "expected Status packet, got command {}",
                status.command
            )));
        }

        eprintln!("link cable: connected");
        Ok(())
    }

    /// Try to accept a pending connection (non-blocking accept, blocking handshake).
    fn try_accept(&mut self) -> bool {
        let listener = match &self.listener {
            Some(l) => l,
            None => return false,
        };

        match listener.accept() {
            Ok((stream, addr)) => {
                eprintln!("link cable: accepted connection from {addr}");
                stream.set_nodelay(true).ok();
                self.stream = Some(stream);

                match self.handshake() {
                    Ok(()) => {
                        self.stream.as_ref().unwrap().set_nonblocking(true).ok();
                        self.state = State::Idle;
                        true
                    }
                    Err(e) => {
                        eprintln!("link cable: handshake failed: {e}");
                        self.stream = None;
                        false
                    }
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => false,
            Err(e) => {
                eprintln!("link cable: accept error: {e}");
                false
            }
        }
    }

    /// Try to read one complete packet from the TCP stream (non-blocking).
    /// Returns None if no complete packet is available yet.
    fn try_recv(&mut self) -> Option<Packet> {
        let stream = self.stream.as_mut()?;
        loop {
            let remaining = Packet::SIZE - self.recv_len;
            if remaining == 0 {
                let packet = Packet::from_bytes(&self.recv_buf);
                self.recv_len = 0;
                return Some(packet);
            }
            match stream.read(&mut self.recv_buf[self.recv_len..]) {
                Ok(0) => {
                    // Connection closed
                    self.handle_disconnect();
                    return None;
                }
                Ok(n) => {
                    self.recv_len += n;
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    if self.recv_len == Packet::SIZE {
                        let packet = Packet::from_bytes(&self.recv_buf);
                        self.recv_len = 0;
                        return Some(packet);
                    }
                    return None;
                }
                Err(_) => {
                    self.handle_disconnect();
                    return None;
                }
            }
        }
    }

    /// Send a packet (non-blocking best-effort).
    fn send(&mut self, packet: Packet) {
        if let Some(stream) = &mut self.stream {
            if stream.write_all(&packet.to_bytes()).is_err() {
                self.handle_disconnect();
            }
        }
    }

    fn handle_disconnect(&mut self) {
        eprintln!("link cable: disconnected");
        self.stream = None;
        self.recv_len = 0;
        if self.listener.is_some() {
            // Server mode: go back to listening
            self.state = State::Listening;
        } else {
            self.state = State::Idle;
        }
    }

    /// Process non-transfer packets. Sync3 responses are suppressed to
    /// avoid ping-pong during rapid exchanges.
    fn handle_housekeeping(&mut self, _packet: &Packet) {
    }
}

impl SerialLink for BgbLink {
    fn tick(&mut self) {
        self.timestamp = self.timestamp.wrapping_add(2);

        if matches!(self.state, State::Listening) {
            self.try_accept();
        }

        // Drain incoming packets. Buffer sync1 for clock() to deliver.
        // All other packets (sync3, status) are silently discarded --
        // responding to sync3 here creates a ping-pong that deadlocks
        // during rapid upload exchanges.
        if self.stream.is_some() && self.pending_sync1.is_none() {
            while let Some(packet) = self.try_recv() {
                if packet.is_sync1() {
                    self.pending_sync1 = Some(packet);
                    break;
                }
            }
        }
    }

    fn clock(&mut self) -> bool {
        match self.state {
            State::Listening => false,
            State::Idle => {
                if let Some(packet) = self.pending_sync1.take() {
                    self.state = State::SlaveTransfer {
                        remote_byte: packet.b2,
                        bits_shifted: 0,
                        our_byte: 0,
                    };
                    return true;
                }
                false
            }
            State::SlaveTransfer { .. } => true,
            State::MasterTransfer { .. } => false,
        }
    }

    fn exchange_bit(&mut self, out_bit: bool) -> bool {
        match &mut self.state {
            State::SlaveTransfer {
                remote_byte,
                bits_shifted,
                our_byte,
            } => {
                // Shift in: MSB of remote byte
                let in_bit = (*remote_byte & 0x80) != 0;
                *remote_byte <<= 1;

                // Collect our outgoing bit
                *our_byte = (*our_byte << 1) | (out_bit as u8);
                *bits_shifted += 1;

                if *bits_shifted == 8 {
                    let response = *our_byte;
                    self.send(Packet::sync2(response));
                    self.state = State::Idle;
                }

                in_bit
            }
            State::MasterTransfer {
                response_byte,
                bits_shifted,
            } => {
                // Shift in: MSB of response byte
                let in_bit = (*response_byte & 0x80) != 0;
                *response_byte <<= 1;
                *bits_shifted += 1;

                if *bits_shifted == 8 {
                    self.send(Packet::sync3_ack());
                    self.state = State::Idle;
                }

                in_bit
            }
            _ => {
                // No active transfer — floating line
                true
            }
        }
    }

    fn notify_transfer_start(&mut self, data: u8, internal_clock: bool) {
        if !internal_clock {
            return;
        }

        // We are master: send Sync1 and try to get Sync2 response
        match self.state {
            State::Listening => {
                self.try_accept();
                if !matches!(self.state, State::Idle) {
                    return;
                }
            }
            State::Idle => {}
            _ => return,
        }

        if self.stream.is_none() {
            return;
        }

        self.send(Packet::sync1(data, self.timestamp));

        // Try to receive Sync2 response. On localhost this should arrive
        // well within the ~64 M-cycles before the first exchange_bit() call.
        // We do a few non-blocking read attempts here.
        for _ in 0..16 {
            if let Some(packet) = self.try_recv() {
                if packet.is_sync2() {
                    self.state = State::MasterTransfer {
                        response_byte: packet.b2,
                        bits_shifted: 0,
                    };
                    return;
                }
                self.handle_housekeeping(&packet);
            }
        }

        // Response hasn't arrived yet — set up master transfer with 0xFF
        // (floating). The real response may arrive during exchange_bit() calls
        // but for simplicity we accept the 0xFF fallback for now.
        self.state = State::MasterTransfer {
            response_byte: 0xFF,
            bits_shifted: 0,
        };
    }
}
