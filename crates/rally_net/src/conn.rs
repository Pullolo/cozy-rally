//! Non-blocking framed TCP connection (u32 LE length + bincode payload).
use crate::protocol::MAX_FRAME;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io::{self, ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};

pub struct Conn {
    stream: TcpStream,
    inbuf: Vec<u8>,
    outbuf: Vec<u8>,
    closed: bool,
    pub peer: Option<SocketAddr>,
}

impl Conn {
    pub fn new(stream: TcpStream) -> io::Result<Conn> {
        stream.set_nonblocking(true)?;
        stream.set_nodelay(true)?;
        let peer = stream.peer_addr().ok();
        Ok(Conn { stream, inbuf: Vec::new(), outbuf: Vec::new(), closed: false, peer })
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Queue a message (sent on `flush`)
    pub fn send<T: Serialize>(&mut self, msg: &T) {
        if self.closed {
            return;
        }
        match bincode::serialize(msg) {
            Ok(bytes) if bytes.len() <= MAX_FRAME => {
                self.outbuf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                self.outbuf.extend_from_slice(&bytes);
            }
            _ => {}
        }
        // a peer that stops reading must not grow our buffer forever
        if self.outbuf.len() > 4 * 1024 * 1024 {
            self.close();
        }
    }

    /// Write as much queued data as the socket takes right now
    pub fn flush(&mut self) {
        while !self.closed && !self.outbuf.is_empty() {
            match self.stream.write(&self.outbuf) {
                Ok(0) => self.close(),
                Ok(n) => {
                    self.outbuf.drain(..n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => self.close(),
            }
        }
    }

    /// Read everything available and decode complete frames
    pub fn recv_all<T: DeserializeOwned>(&mut self) -> Vec<T> {
        let mut buf = [0u8; 16 * 1024];
        while !self.closed {
            match self.stream.read(&mut buf) {
                Ok(0) => self.close(),
                Ok(n) => self.inbuf.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => self.close(),
            }
        }
        let mut out = Vec::new();
        loop {
            if self.inbuf.len() < 4 {
                break;
            }
            let len = u32::from_le_bytes([self.inbuf[0], self.inbuf[1], self.inbuf[2], self.inbuf[3]]) as usize;
            if len > MAX_FRAME {
                self.close();
                break;
            }
            if self.inbuf.len() < 4 + len {
                break;
            }
            match bincode::deserialize::<T>(&self.inbuf[4..4 + len]) {
                Ok(m) => out.push(m),
                Err(_) => {
                    // garbage from a wrong client / version: drop the connection
                    self.close();
                    break;
                }
            }
            self.inbuf.drain(..4 + len);
        }
        out
    }

    pub fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            let _ = self.stream.shutdown(std::net::Shutdown::Both);
        }
    }
}
