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
    live: Option<crate::live::LiveSource>,
    transport_ready: bool,
    live_generation: [u64;2],
    clock: Instant,
    playing: bool,
    handshake_complete: bool,
    audio_on: bool,
    audio_allowed: bool,
    hu_muted: bool,
    epochs: [u64; 2],
    last_video: Duration,
    first_header: Vec<u8>,
    input: crate::input::Input,
}

impl Endpoint {
    pub fn new() -> Self {
        Self::with_media(None)
    }

    pub fn with_media(media: Option<TestMedia>) -> Self {
        let session = media.as_ref().map_or_else(Session::new, |m| Session::with_video_source(m.config()));
        Self { decoder: Decoder::new(Limits::default()), session, media, live: None, transport_ready: true, live_generation: [0;2], clock: Instant::now(),
            playing: false, handshake_complete: false, audio_on: false, audio_allowed: true, hu_muted: false, epochs: [0; 2], last_video: Duration::ZERO,
            first_header: Vec::with_capacity(8), input: crate::input::Input::default() }
    }

    pub fn with_source(media: Option<TestMedia>, live_path: Option<std::path::PathBuf>) -> Result<Self,String> {
        let mut endpoint=Self::with_media(media);
        if let Some(path)=live_path { endpoint.live=Some(crate::live::LiveSource::bind(&path)?); endpoint.session=Session::with_video_source(crate::live::CONFIG); }
        Ok(endpoint)
    }
    fn sync_live_epoch(&mut self) {
        if let Some(live)=self.live.as_ref() {
            for (lane,value) in live.generation().into_iter().enumerate(){
                if value!=self.live_generation[lane]{self.live_generation[lane]=value;self.epochs[lane]+=1;}
            }
        }
    }
    pub fn transport_ready(&mut self, ready:bool) {
        if self.transport_ready!=ready {self.transport_ready=ready;if let Some(live)=self.live.as_mut(){live.set_playing(ready && self.playing);}self.sync_live_epoch();}
    }
    /// VIDEO_START completed this USB negotiation. PAUSE and a silent/static
    /// source preserve the established connection rather than rearming it.
    pub fn handshake_complete(&self)->bool {self.handshake_complete}

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
        if self.media.is_some() && self.playing && self.audio_allowed && !self.hu_muted && !self.audio_on {
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
            if let Some(live)=self.live.as_mut() {
                if !live.accepts_input(){self.input=crate::input::Input::default();}
                if let Some(config)=self.session.video_config().filter(|_|live.accepts_input()) {
                    match self.input.handle(&message,self.clock.elapsed().as_millis() as u64) {
                        Ok(keys)=>for code in keys {live.feedback(&format!("{{\"type\":\"key-press\",\"code\":{code}}}"));},
                        Err(error)=>log("input_ignored",&error),
                    }
                    match crate::live::touch(&message,config) {
                        Ok(Some(control))=>live.feedback(&control),
                        Ok(None)=>{},
                        Err(error)=>log("touch_ignored",&error),
                    }
                }
            }
            let outcome = self.session.handle(&message).map_err(|e| e.to_string())?;
            for event in outcome.events {
                log("carlife_state", &format!("{event:?}"));
                match event {
                    Event::VideoNegotiated(config) => {
                        if self.live.is_some() && self.session.video_state()!=VideoState::AwaitingVideoSource {self.playing=false;if let Some(live)=self.live.as_mut(){live.set_playing(false);}}
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
                        self.handshake_complete=true;
                        if let Some(live)=self.live.as_mut(){self.playing=true;live.set_playing(self.transport_ready);}
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
                        if let Some(live)=self.live.as_mut(){live.set_playing(false);}
                        if let Some(media) = self.media.as_mut() { media.set_playing(false, self.clock.elapsed()); }
                        self.stop_audio(&mut packets)?;
                    },
                    Event::VideoReset { .. } if self.playing => {
                        if let Some(live)=self.live.as_mut(){live.reset();}
                        self.epochs[0] += 1;
                        if let Some(media) = self.media.as_mut() { media.set_playing(true, self.clock.elapsed()); }
                    },
                    Event::FrameRateChanged { fps } => {
                        if let Some(live)=self.live.as_mut(){live.reset();}
                        self.epochs[0] += 1;
                        if let Some(media) = self.media.as_mut() {
                            media.set_fps(fps)?;
                            if self.playing { media.set_playing(true, self.clock.elapsed()); }
                        }
                    },
                    Event::AudioTransport { usb } => {
                        self.audio_allowed = usb;
                        if let Some(live)=self.live.as_mut(){live.set_audio_allowed(usb);}
                        if !usb { self.stop_audio(&mut packets)?; }
                        else { self.start_audio(&mut packets)?; }
                    },
                    Event::MusicControl { status } if self.media.is_some() || self.live.is_some() => {
                        if status == 0 { self.hu_muted = true; self.stop_audio(&mut packets)?;if let Some(live)=self.live.as_mut(){live.set_audio_allowed(false);} }
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
        self.sync_live_epoch();
        Ok(packets)
    }

    pub fn finish(&mut self) -> Result<(), String> {
        self.decoder.finish().map_err(|e| e.to_string())
    }

    pub fn poll_media(&mut self) -> Result<Option<Vec<u8>>, String> {
        let now = self.clock.elapsed();
        if let Some(live)=self.live.as_mut(){
            live.poll_io()?;
            live.clock_floor(now.as_millis() as u32);
        }
        // Video liveness has its own deadline. Continuous PCM must not keep
        // returning before the heartbeat when the picture is static or waiting
        // for a recovery IDR.
        if (self.live.is_some() || self.media.is_some()) && self.session.video_config().is_some()
            && now.saturating_sub(self.last_video)>=Duration::from_secs(1) {
            self.last_video=now;
            let timestamp=self.live.as_mut().map_or(now.as_millis() as u32,
                |live|live.heartbeat(now.as_millis() as u32));
            log("video_heartbeat", "independent video deadline");
            return Message{channel:crate::carlife::Channel::Video,id:0x20002,
                timestamp_ms:Some(timestamp),reserved:0,payload:Vec::new()}
                .encode().map(Some).map_err(|e|e.to_string());
        }
        if let Some(live)=self.live.as_mut(){
            let message=live.next(self.audio_allowed && !self.hu_muted && self.transport_ready);
            self.sync_live_epoch();
            if let Some(message)=message {
                if message.channel==crate::carlife::Channel::Video {self.last_video=now;}
                let state=match message.id {0x30001=>Some(true),0x30002=>Some(false),_=>None};
                let mut wire=message.encode().map_err(|e|e.to_string())?;
                if let Some(on)=state {self.audio_on=on;wire.extend(self.music_status()?);}
                return Ok(Some(wire));
            }
            return Ok(None);
        }
        if let Some(media) = self.media.as_mut() {
            if let Some(message) = media.next(now)? {
                if message.channel == crate::carlife::Channel::Video { self.last_video = now; }
                return message.encode().map(Some).map_err(|e| e.to_string());
            }
        }
        Ok(None)
    }

    pub fn epochs(&self) -> [u64; 2] { self.epochs }

    pub fn media_backpressure(&mut self, video: bool) {
        if let Some(live)=self.live.as_mut() {
            if video {live.recover_video();self.sync_live_epoch();}
            return;
        }
        if video && self.playing {
            self.epochs[0] += 1;
            if let Some(media) = self.media.as_mut() { media.set_playing(true, self.clock.elapsed()); }
        }
    }
}

pub fn tcp(address: SocketAddr, duration: Duration, media: Option<TestMedia>, live_path: Option<std::path::PathBuf>) -> Result<(), String> {
    if !address.ip().is_loopback() {
        return Err("TCP lab listener must use a loopback address; use SSH forwarding for board tests".into());
    }
    let listener = TcpListener::bind(address).map_err(|e| format!("listen: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let actual = listener.local_addr().map_err(|e| e.to_string())?;
    log("tcp_listening", &actual.to_string());
    let deadline = Instant::now() + if duration.is_zero(){Duration::from_secs(100*365*86400)}else{duration};
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
    let mut endpoint = Endpoint::with_source(media,live_path)?;
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn usb_handshake_requires_start_and_remains_complete_when_paused(){
        let mut endpoint=Endpoint::new();
        for (id,payload) in [
            (ids::HU_PROTOCOL_VERSION,vec![8,4,16,1]),
            (ids::VIDEO_INIT,vec![8,0xa0,6,16,0xe0,3,24,30]),
            (ids::VIDEO_PAUSE,vec![]),
        ] {endpoint.receive(&Message::command(id,payload).unwrap().encode().unwrap()).unwrap();}
        assert!(!endpoint.handshake_complete());
        let mut start_and_pause=Message::command(ids::VIDEO_START,vec![]).unwrap().encode().unwrap();
        start_and_pause.extend(Message::command(ids::VIDEO_PAUSE,vec![]).unwrap().encode().unwrap());
        endpoint.receive(&start_and_pause).unwrap();
        assert!(endpoint.handshake_complete());assert!(!endpoint.playing);
        endpoint.transport_ready(false);assert!(endpoint.handshake_complete());
        endpoint.transport_ready(true);assert!(endpoint.handshake_complete());
        assert!(!Endpoint::new().handshake_complete());
    }

    #[test]
    fn continuous_audio_cannot_starve_video_heartbeat() {
        let path=std::env::temp_dir().join(format!("carlink-heartbeat-{}",std::process::id()));
        let mut endpoint=Endpoint::with_source(None,Some(path.clone())).unwrap();
        for (id,payload) in [
            (ids::HU_PROTOCOL_VERSION,vec![8,4,16,1]),
            (ids::VIDEO_INIT,vec![8,0xa0,6,16,0xe0,3,24,30]),
            (ids::VIDEO_START,vec![]),
        ] {endpoint.receive(&Message::command(id,payload).unwrap().encode().unwrap()).unwrap();}
        let mut producer=std::os::unix::net::UnixStream::connect(&path).unwrap();
        let frame=|kind:u8,flags:u8,payload:&[u8]| {
            let mut wire=((12+payload.len()) as u32).to_le_bytes().to_vec();
            wire.extend([kind,flags,0,0]);wire.extend(0u64.to_le_bytes());wire.extend(payload);wire
        };
        let mut config=Vec::new();for n in [800u32,480,30]{config.extend(n.to_le_bytes());}
        producer.write_all(&frame(4,1,&[])).unwrap();
        producer.write_all(&frame(1,0,&config)).unwrap();
        endpoint.poll_media().unwrap();
        for _ in 0..4 {producer.write_all(&frame(3,0,&[0;3528])).unwrap();}
        endpoint.clock=Instant::now()-Duration::from_secs(2);
        let heartbeat=endpoint.poll_media().unwrap().unwrap();
        let messages=Decoder::new(Limits::default()).feed(&heartbeat).unwrap();
        assert_eq!(messages[0].id,0x20002);
        assert!(messages[0].payload.is_empty());
        let audio=endpoint.poll_media().unwrap().unwrap();
        let messages=Decoder::new(Limits::default()).feed(&audio).unwrap();
        assert_eq!(messages[0].channel,crate::carlife::Channel::Media);
        assert_eq!(endpoint.last_video.as_secs(),2);
    }
    #[test]
    fn invalid_touch_does_not_terminate_live_session() {
        let path=std::env::temp_dir().join(format!("carlink-touch-{}",std::process::id()));
        let mut endpoint=Endpoint::with_source(None,Some(path.clone())).unwrap();
        let version=Message::command(ids::HU_PROTOCOL_VERSION,vec![8,4,16,1]).unwrap().encode().unwrap();
        endpoint.receive(&version).unwrap();
        let init=Message::command(ids::VIDEO_INIT,vec![8,0xa0,6,16,0xe0,3,24,30]).unwrap().encode().unwrap();
        endpoint.receive(&init).unwrap();
        endpoint.receive(&Message::command(ids::VIDEO_START,vec![]).unwrap().encode().unwrap()).unwrap();
        let mut producer=std::os::unix::net::UnixStream::connect(&path).unwrap();
        let mut active=12u32.to_le_bytes().to_vec();active.extend([4,1,0,0]);active.extend(0u64.to_le_bytes());
        producer.write_all(&active).unwrap();endpoint.poll_media().unwrap();
        assert!(endpoint.live.as_ref().unwrap().accepts_input());
        for payload in [vec![8,0,16,1,24,1,42,5,1],vec![8,0,16,0xff,0x7f,24,1]] {
            let touch=Message::command(0x68001,payload).unwrap().encode().unwrap();
            endpoint.receive(&touch).expect("bad contact must not disconnect USB");
            endpoint.receive(&init).expect("session must still process commands");
        }
    }
}
