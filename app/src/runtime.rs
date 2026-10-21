//! Diagnostic runners. TCP carries USB framing for lab tests, not wireless CarLife.
use crate::carlife::{Decoder, Limits, Session};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::thread;
use std::time::{Duration, Instant};

pub fn log(event: &str, detail: &str) {
    println!("{{\"event\":{},\"detail\":{}}}", crate::quote(event), crate::quote(detail));
}

pub struct Endpoint {
    decoder: Decoder,
    session: Session,
}

impl Endpoint {
    pub fn new() -> Self {
        Self { decoder: Decoder::new(Limits::default()), session: Session::new() }
    }

    pub fn receive(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        let mut packets = Vec::new();
        for message in self.decoder.feed(bytes).map_err(|e| e.to_string())? {
            log("carlife_rx", &format!("channel={:?} id=0x{:08x} bytes={}",
                message.channel, message.id, message.payload.len()));
            let outcome = self.session.handle(&message).map_err(|e| e.to_string())?;
            for event in outcome.events {
                log("carlife_state", &format!("{event:?}"));
            }
            for outgoing in outcome.outgoing {
                let packet = outgoing.encode().map_err(|e| e.to_string())?;
                log("carlife_tx", &format!("channel={:?} id=0x{:08x} bytes={}",
                    outgoing.channel, outgoing.id, outgoing.payload.len()));
                packets.push(packet);
            }
        }
        Ok(packets)
    }

    pub fn finish(&mut self) -> Result<(), String> {
        self.decoder.finish().map_err(|e| e.to_string())
    }
}

pub fn tcp(address: SocketAddr, duration: Duration) -> Result<(), String> {
    if !address.ip().is_loopback() {
        return Err("TCP lab listener must use a loopback address; use SSH forwarding for board tests".into());
    }
    let listener = TcpListener::bind(address).map_err(|e| format!("listen: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let actual = listener.local_addr().map_err(|e| e.to_string())?;
    log("tcp_listening", &actual.to_string());
    let deadline = Instant::now() + duration;
    let mut stream = loop {
        if Instant::now() >= deadline { return Err("TCP lab deadline waiting for host".into()); }
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(10)),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("accept: {error}")),
        }
    };
    stream.set_nodelay(true).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_millis(100))).map_err(|e| e.to_string())?;
    let mut endpoint = Endpoint::new();
    let mut bytes = [0_u8; 16384];
    log("tcp_connected", "one development session");
    loop {
        if Instant::now() >= deadline { return Err("TCP lab session deadline".into()); }
        match stream.read(&mut bytes) {
            Ok(0) => {
                endpoint.finish()?;
                log("transport_closed", "clean EOF");
                return Ok(());
            }
            Ok(count) => {
                for packet in endpoint.receive(&bytes[..count])? {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() { return Err("TCP lab session deadline".into()); }
                    stream.set_write_timeout(Some(remaining.min(Duration::from_secs(2))))
                        .map_err(|e| e.to_string())?;
                    stream.write_all(&packet).map_err(|e| format!("write: {e}"))?;
                }
            }
            Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => {},
            Err(error) => return Err(format!("read: {error}")),
        }
    }
}
