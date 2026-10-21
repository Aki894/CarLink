//! Diagnostic runners. TCP carries USB framing for lab tests, not wireless CarLife.
use crate::carlife::{ids, Decoder, Event, Limits, Message, Session, VideoState};
use crate::media::TestMedia;
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
    media: Option<TestMedia>,
    clock: Instant,
    playing: bool,
    audio_on: bool,
    audio_allowed: bool,
    hu_muted: bool,
    epochs: [u64; 2],
    last_video: Duration,
    first_header: Vec<u8>,
}

impl Endpoint {
    pub fn new() -> Self {
        Self::with_media(None)
    }

    pub fn with_media(media: Option<TestMedia>) -> Self {
        let session = media.as_ref().map_or_else(Session::new, |m| Session::with_video_source(m.config()));
        Self { decoder: Decoder::new(Limits::default()), session, media, clock: Instant::now(),
            playing: false, audio_on: false, audio_allowed: true, hu_muted: false, epochs: [0; 2], last_video: Duration::ZERO,
            first_header: Vec::with_capacity(8) }
    }

    fn music_status(&self) -> Result<Vec<u8>, String> {
        Message::command(ids::MODULE_STATUS, vec![8,1,18,4,8,3,16,self.audio_on as u8])
            .and_then(|m| m.encode()).map_err(|e| e.to_string())
    }

    fn stop_audio(&mut self, packets: &mut Vec<Vec<u8>>) -> Result<(), String> {
        if self.audio_on {
            self.epochs[1] += 1;
            if let Some(media) = self.media.as_mut() {
                media.set_audio_playing(false, self.clock.elapsed());
                packets.push(media.audio_stop().encode().map_err(|e| e.to_string())?);
            }
            self.audio_on = false;
            packets.push(self.music_status()?);
        }
        Ok(())
    }

    fn start_audio(&mut self, packets: &mut Vec<Vec<u8>>) -> Result<(), String> {
        if self.playing && self.audio_allowed && !self.hu_muted && !self.audio_on {
            self.epochs[1] += 1;
            self.audio_on = true;
            packets.push(self.music_status()?);
            let media = self.media.as_mut().ok_or("audio source missing")?;
            packets.push(media.audio_init().encode().map_err(|e| e.to_string())?);
            media.set_audio_playing(true, self.clock.elapsed());
        }
        Ok(())
    }

    pub fn receive(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        let mut packets = Vec::new();
        let needed = 8 - self.first_header.len();
        self.first_header.extend_from_slice(&bytes[..bytes.len().min(needed)]);
        for message in self.decoder.feed(bytes).map_err(|e| format!("{e}; first_outer_header={:02x?} chunk_bytes={}", self.first_header, bytes.len()))? {
            log("carlife_rx", &format!("channel={:?} id=0x{:08x} bytes={}",
                message.channel, message.id, message.payload.len()));
            if message.id == ids::VIDEO_INIT {
                log("hu_video_preference", &format!("{:?}", crate::carlife::video_hint(&message.payload).map_err(|e| e.to_string())?));
            }
            let outcome = self.session.handle(&message).map_err(|e| e.to_string())?;
            for event in outcome.events {
                log("carlife_state", &format!("{event:?}"));
                match event {
                    Event::VideoNegotiated(config) => {
                        if let Some(media) = self.media.as_mut() {
                            media.set_fps(config.fps)?;
                            if self.session.video_state() != VideoState::AwaitingVideoSource {
                                self.epochs[0] += 1;
                                media.set_playing(false, self.clock.elapsed());
                                self.playing = false;
                            }
                        }
                        if !self.playing { self.stop_audio(&mut packets)?; }
                    },
                    Event::AwaitingVideoSource => {
                        if let Some(media) = self.media.as_mut().filter(|_| !self.playing) {
                            self.epochs[0] += 1;
                            media.set_playing(true, self.clock.elapsed());
                            self.playing = true;
                            log("test_media_started", "generated H.264 / PCM; relative monotonic millisecond timestamps");
                        }
                        self.start_audio(&mut packets)?;
                    },
                    Event::VideoPaused => {
                        self.epochs[0] += 1;
                        self.playing = false;
                        if let Some(media) = self.media.as_mut() { media.set_playing(false, self.clock.elapsed()); }
                        self.stop_audio(&mut packets)?;
                    },
                    Event::VideoReset { .. } if self.playing => {
                        self.epochs[0] += 1;
                        if let Some(media) = self.media.as_mut() { media.set_playing(true, self.clock.elapsed()); }
                    },
                    Event::FrameRateChanged { fps } => {
                        self.epochs[0] += 1;
                        if let Some(media) = self.media.as_mut() {
                            media.set_fps(fps)?;
                            if self.playing { media.set_playing(true, self.clock.elapsed()); }
                        }
                    },
                    Event::AudioTransport { usb } => {
                        self.audio_allowed = usb;
                        if !usb { self.stop_audio(&mut packets)?; }
                        else { self.start_audio(&mut packets)?; }
                    },
                    Event::MusicControl { status } if self.media.is_some() => {
                        if status == 0 { self.hu_muted = true; self.stop_audio(&mut packets)?; }
                        packets.push(self.music_status()?);
                    },
                    _ => {},
                }
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

    pub fn poll_media(&mut self) -> Result<Option<Vec<u8>>, String> {
        let now = self.clock.elapsed();
        if let Some(media) = self.media.as_mut() {
            if let Some(message) = media.next(now)? {
                if message.channel == crate::carlife::Channel::Video { self.last_video = now; }
                return message.encode().map(Some).map_err(|e| e.to_string());
            }
            if self.session.video_config().is_some() && now.saturating_sub(self.last_video) >= Duration::from_secs(1) {
                self.last_video = now;
                return Message { channel: crate::carlife::Channel::Video, id: 0x20002,
                    timestamp_ms: Some(now.as_millis() as u32), reserved: 0, payload: Vec::new() }
                    .encode().map(Some).map_err(|e| e.to_string());
            }
        }
        Ok(None)
    }

    pub fn epochs(&self) -> [u64; 2] { self.epochs }

    pub fn media_backpressure(&mut self, video: bool) {
        if video && self.playing {
            self.epochs[0] += 1;
            if let Some(media) = self.media.as_mut() { media.set_playing(true, self.clock.elapsed()); }
        }
    }
}

pub fn tcp(address: SocketAddr, duration: Duration, media: Option<TestMedia>) -> Result<(), String> {
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
    stream.set_read_timeout(Some(Duration::from_millis(2))).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).map_err(|e| e.to_string())?;
    let mut endpoint = Endpoint::with_media(media);
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
        // Service both due lanes before waiting for another control read.
        for _ in 0..4 {
            let Some(packet) = endpoint.poll_media()? else { break; };
            stream.write_all(&packet).map_err(|e| format!("write media: {e}"))?;
        }
    }
}
