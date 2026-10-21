// SPDX-License-Identifier: MIT
//! CarLife MD wire framing and a transport-independent handshake milestone.
//!
//! Protocol facts checked against (no implementation or authentication keys copied):
//! - headswap 99e888cf546ec5f040d4de377c59b478186afd5b:
//!   android/app/src/main/java/com/yegorkapomidorka/headswap/{Protocol,Handshake}.kt
//! - Carlife_on_box d9397d90a1f32670f034eb07cdbe28d887a179d8:
//!   tools/boot_native/md/src/org/carlife/hulab/NativeSessionProtocol.java
//!   docs/HEADSWAP_LESSONS.md
//!
//! This is a strict stream profile: each channel carries concatenated inner
//! messages, independently of outer-frame boundaries. It supports splitting
//! anywhere in an inner header/body and interleaving channels. It does not guess
//! around incorrect declared lengths or resynchronize after corruption. Some
//! real HUs require additional trace-backed length-quirk compatibility.
//! Test media is supplied separately; authentication/encryption/input are pending.

use std::fmt;

pub mod ids {
    pub const HU_PROTOCOL_VERSION: u32 = 0x18001;
    pub const VERSION_MATCH: u32 = 0x10002;
    pub const HU_INFO: u32 = 0x18003;
    pub const MD_INFO: u32 = 0x10004;
    pub const VIDEO_INIT: u32 = 0x18007;
    pub const VIDEO_INIT_DONE: u32 = 0x10008;
    pub const VIDEO_START: u32 = 0x18009;
    pub const VIDEO_PAUSE: u32 = 0x1800a;
    pub const VIDEO_RESET: u32 = 0x1800b;
    pub const VIDEO_FPS_CHANGE: u32 = 0x1800c;
    pub const VIDEO_FPS_DONE: u32 = 0x1000d;
    pub const SCREEN_ON: u32 = 0x10018;
    pub const FOREGROUND: u32 = 0x1001b;
    pub const STATISTIC_INFO: u32 = 0x18027;
    pub const MODULE_STATUS: u32 = 0x10026;
    pub const MODULE_CONTROL: u32 = 0x18028;
    pub const SUBSCRIBE: u32 = 0x18043;
    pub const SUBSCRIBE_DONE: u32 = 0x10044;
    pub const SUBSCRIBE_START: u32 = 0x18045;
    pub const SUBSCRIBE_STOP: u32 = 0x18046;
    pub const HU_AUTH_REQUEST: u32 = 0x18048;
    pub const HU_AUTH_RESULT: u32 = 0x1804a;
    pub const FEATURE_REQUEST: u32 = 0x10051;
    pub const HU_FEATURE_RESPONSE: u32 = 0x18052;
    pub const HU_RSA_KEY: u32 = 0x1806b;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Channel { Cmd = 1, Video = 2, Media = 3, Tts = 4, Vr = 5, Touch = 6 }

impl Channel {
    pub fn from_u32(value: u32) -> Result<Self, Error> {
        match value {
            1 => Ok(Self::Cmd), 2 => Ok(Self::Video), 3 => Ok(Self::Media),
            4 => Ok(Self::Tts), 5 => Ok(Self::Vr), 6 => Ok(Self::Touch),
            _ => Err(Error::UnknownChannel(value)),
        }
    }
    fn index(self) -> usize { self as usize - 1 }
    fn header_len(self) -> usize {
        if matches!(self, Self::Cmd | Self::Touch) { 8 } else { 12 }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    UnknownChannel(u32), InvalidLimits, LimitExceeded(&'static str),
    InvalidFrame(&'static str), TruncatedOuter { buffered: usize },
    TruncatedInner { channel: Channel, buffered: usize }, Closed,
    InvalidProtobuf(&'static str), InvalidState(&'static str),
    UnsupportedVersion { major: u32, minor: u32 },
    Unsupported(&'static str), InvalidVideoConfig,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{self:?}") }
}
impl std::error::Error for Error {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Message {
    pub channel: Channel,
    pub id: u32,
    /// CMD/TOUCH have no timestamp. Media timestamps are milliseconds on the wire.
    pub timestamp_ms: Option<u32>,
    /// Retained opaque CMD/TOUCH reserved bytes; do not assume they are zero on RX.
    pub reserved: u16,
    pub payload: Vec<u8>,
}

impl Message {
    pub fn command(id: u32, payload: Vec<u8>) -> Result<Self, Error> {
        let channel = Channel::from_u32(id >> 16)?;
        if !matches!(channel, Channel::Cmd | Channel::Touch) {
            return Err(Error::InvalidFrame("command must use CMD or TOUCH"));
        }
        if payload.len() > u16::MAX as usize { return Err(Error::LimitExceeded("command payload")); }
        Ok(Self { channel, id, timestamp_ms: None, reserved: 0, payload })
    }

    /// Encodes one complete inner message into one outer frame, without USB I/O.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        if self.id >> 16 != self.channel as u32 { return Err(Error::InvalidFrame("message/channel mismatch")); }
        let header = self.channel.header_len();
        if self.payload.len() > Limits::default().max_inner_payload { return Err(Error::LimitExceeded("encoded payload")); }
        let mut inner = Vec::with_capacity(header + self.payload.len());
        if header == 8 {
            if self.timestamp_ms.is_some() || self.payload.len() > u16::MAX as usize {
                return Err(Error::InvalidFrame("invalid command timestamp/length"));
            }
            inner.extend_from_slice(&(self.payload.len() as u16).to_be_bytes());
            inner.extend_from_slice(&self.reserved.to_be_bytes());
        } else {
            if self.reserved != 0 { return Err(Error::InvalidFrame("media has no reserved word")); }
            let time = self.timestamp_ms.ok_or(Error::InvalidFrame("media timestamp missing"))?;
            inner.extend_from_slice(&(self.payload.len() as u32).to_be_bytes());
            inner.extend_from_slice(&time.to_be_bytes());
        }
        inner.extend_from_slice(&self.id.to_be_bytes());
        inner.extend_from_slice(&self.payload);
        let mut wire = Vec::with_capacity(8 + inner.len());
        wire.extend_from_slice(&(self.channel as u32).to_be_bytes());
        wire.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        wire.extend(inner);
        Ok(wire)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_outer_payload: usize,
    pub max_inner_payload: usize,
    pub max_buffered: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self { max_outer_payload: 1024 * 1024 + 12, max_inner_payload: 1024 * 1024, max_buffered: 4 * 1024 * 1024 }
    }
}

pub struct Decoder {
    limits: Limits,
    outer: Vec<u8>,
    channels: [Vec<u8>; 6],
    closed: bool,
}
impl Default for Decoder { fn default() -> Self { Self::new(Limits::default()) } }

impl Decoder {
    /// Invalid custom limits are reported on feed/finish, never silently clamped.
    pub fn new(limits: Limits) -> Self {
        Self { limits, outer: Vec::new(), channels: std::array::from_fn(|_| Vec::new()), closed: false }
    }
    fn validate_limits(&self) -> Result<(), Error> {
        if self.limits.max_outer_payload == 0 || self.limits.max_inner_payload == 0
            || self.limits.max_outer_payload > u32::MAX as usize
            || self.limits.max_outer_payload > usize::MAX - 8
            || self.limits.max_inner_payload > u32::MAX as usize - 12
            || self.limits.max_buffered < 20 {
            Err(Error::InvalidLimits)
        } else { Ok(()) }
    }
    fn buffered(&self) -> usize { self.outer.len() + self.channels.iter().map(Vec::len).sum::<usize>() }
    fn check_add(&self, count: usize) -> Result<(), Error> {
        if count > self.limits.max_buffered.saturating_sub(self.buffered()) {
            Err(Error::LimitExceeded("aggregate buffered bytes"))
        } else { Ok(()) }
    }
    /// Any decoding error is terminal. Returned messages are valid only if the
    /// whole call succeeds; callers must not retry corrupted input or resync.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Message>, Error> {
        if self.closed { return Err(Error::Closed); }
        let result = self.feed_inner(bytes);
        if result.is_err() { self.closed = true; }
        result
    }
    fn feed_inner(&mut self, mut bytes: &[u8]) -> Result<Vec<Message>, Error> {
        self.validate_limits()?;
        // This also bounds the batch of completed messages returned by one call.
        if bytes.len() > self.limits.max_buffered { return Err(Error::LimitExceeded("input batch")); }
        let mut messages = Vec::new();
        while !bytes.is_empty() {
            let target = if self.outer.len() < 8 { 8 } else {
                let length = be32(&self.outer[4..8]) as usize;
                if length == 0 || length > self.limits.max_outer_payload {
                    return Err(Error::LimitExceeded("outer payload length"));
                }
                Channel::from_u32(be32(&self.outer[..4]))?;
                8 + length
            };
            let count = bytes.len().min(target - self.outer.len());
            self.check_add(count)?;
            self.outer.extend_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
            if self.outer.len() >= 8 {
                let channel = Channel::from_u32(be32(&self.outer[..4]))?;
                let length = be32(&self.outer[4..8]) as usize;
                if length == 0 || length > self.limits.max_outer_payload {
                    return Err(Error::LimitExceeded("outer payload length"));
                }
                if self.outer.len() == 8 + length {
                    let frame = std::mem::take(&mut self.outer);
                    self.channel_bytes(channel, &frame[8..], &mut messages)?;
                }
            }
        }
        Ok(messages)
    }
    fn channel_bytes(&mut self, channel: Channel, mut bytes: &[u8], messages: &mut Vec<Message>) -> Result<(), Error> {
        let index = channel.index();
        let header = channel.header_len();
        while !bytes.is_empty() {
            let target = if self.channels[index].len() < header { header } else { self.inner_size(channel)? };
            let count = bytes.len().min(target - self.channels[index].len());
            self.check_add(count)?;
            self.channels[index].extend_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
            if self.channels[index].len() >= header {
                let size = self.inner_size(channel)?;
                if self.channels[index].len() == size {
                    let inner = &self.channels[index];
                    messages.push(Message {
                        channel, id: be32(&inner[header - 4..header]),
                        timestamp_ms: if header == 12 { Some(be32(&inner[4..8])) } else { None },
                        reserved: if header == 8 { u16::from_be_bytes([inner[2], inner[3]]) } else { 0 },
                        payload: inner[header..].to_vec(),
                    });
                    self.channels[index].clear();
                }
            }
        }
        Ok(())
    }
    fn inner_size(&self, channel: Channel) -> Result<usize, Error> {
        let inner = &self.channels[channel.index()];
        let header = channel.header_len();
        let length = if header == 8 { u16::from_be_bytes([inner[0], inner[1]]) as usize } else { be32(&inner[..4]) as usize };
        if length > self.limits.max_inner_payload { return Err(Error::LimitExceeded("inner payload length")); }
        if be32(&inner[header - 4..header]) >> 16 != channel as u32 {
            return Err(Error::InvalidFrame("message/channel mismatch"));
        }
        Ok(header + length)
    }
    /// EOF is successful only at both outer and per-channel inner boundaries.
    /// Successful EOF also closes the decoder, requiring a new decoder on reconnect.
    pub fn finish(&mut self) -> Result<(), Error> {
        if self.closed { return Err(Error::Closed); }
        self.closed = true;
        self.validate_limits()?;
        if !self.outer.is_empty() { return Err(Error::TruncatedOuter { buffered: self.outer.len() }); }
        for (index, bytes) in self.channels.iter().enumerate() {
            if !bytes.is_empty() { return Err(Error::TruncatedInner { channel: Channel::from_u32(index as u32 + 1)?, buffered: bytes.len() }); }
        }
        Ok(())
    }
}

fn be32(bytes: &[u8]) -> u32 { u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoConfig { pub width: u32, pub height: u32, pub fps: u32 }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoState { Uninitialized, Negotiated, AwaitingVideoSource, Paused }
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    ProtocolVersion { major: u32, minor: u32, repeated: bool },
    HuInfoReceived, StatisticReceived, VideoNegotiated(VideoConfig),
    /// A START request was received; no encoder or H.264 has been started.
    AwaitingVideoSource, VideoPaused, VideoReset { generation: u64 },
    FrameRateChanged { fps: u32 }, FeaturesReceived,
    SubscriptionsUnsupported { modules: Vec<u32> },
    SubscriptionControlIgnored { id: u32 },
    ModuleControlIgnored { module: u32, status: u32 },
    MusicControl { status: u32 }, AudioTransport { usb: bool },
    Unhandled { channel: Channel, id: u32, bytes: usize },
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Outcome { pub outgoing: Vec<Message>, pub events: Vec<Event> }

#[derive(Clone, Debug)]
pub struct Session {
    version: Option<(u32, u32)>,
    video: Option<VideoConfig>,
    state: VideoState,
    generation: u64,
    failed: bool,
    source: Option<VideoConfig>,
}
impl Default for Session { fn default() -> Self { Self::new() } }

impl Session {
    pub fn new() -> Self { Self { version: None, video: None, state: VideoState::Uninitialized, generation: 0, failed: false, source: None } }
    pub fn with_video_source(config: VideoConfig) -> Self { Self { source: Some(config), ..Self::new() } }
    pub fn video_state(&self) -> VideoState { self.state }
    pub fn video_config(&self) -> Option<VideoConfig> { self.video }
    pub fn protocol_version(&self) -> Option<(u32, u32)> { self.version }
    pub fn is_failed(&self) -> bool { self.failed }
    /// State changes are transactional. Malformed or unsupported security input
    /// closes the logical session; reconnect must create a fresh Session.
    pub fn handle(&mut self, message: &Message) -> Result<Outcome, Error> {
        if self.failed { return Err(Error::Closed); }
        let mut candidate = self.clone();
        match candidate.handle_inner(message) {
            Ok(outcome) => { *self = candidate; Ok(outcome) },
            Err(error) => { self.failed = true; Err(error) },
        }
    }
    fn handle_inner(&mut self, message: &Message) -> Result<Outcome, Error> {
        use ids::*;
        if message.id >> 16 != message.channel as u32 { return Err(Error::InvalidFrame("message/channel mismatch")); }
        if message.payload.len() > 65535 { return Err(Error::LimitExceeded("session command payload")); }
        let mut out = Outcome::default();
        if message.channel != Channel::Cmd {
            out.events.push(Event::Unhandled { channel: message.channel, id: message.id, bytes: message.payload.len() });
            return Ok(out);
        }
        // Never turn the unconditional peer-acceptance status used by some MDs
        // into a forged answer to a HU phone challenge.
        if matches!(message.id, HU_AUTH_REQUEST | HU_AUTH_RESULT | HU_RSA_KEY) {
            return Err(Error::Unsupported("HU authentication/key exchange"));
        }
        if message.id == HU_PROTOCOL_VERSION {
            let fields = fields(&message.payload)?;
            let (major, minor) = (uint(&fields, 1)?, uint(&fields, 2)?);
            // 4.1 is advertised by the Android HU SDK tested on our lab phone;
            // it uses the same base framing/handshake. Optional features still
            // follow the explicit unsupported/authentication checks below.
            if !matches!((major, minor), (1, 0) | (2, 0) | (2, 1) | (4, 1)) {
                return Err(Error::UnsupportedVersion { major, minor });
            }
            let repeated = self.version == Some((major, minor));
            if self.version.is_some() && !repeated { return Err(Error::InvalidState("protocol changed within session")); }
            self.version = Some((major, minor));
            // v2 HUs may resend VERSION while already negotiating; replying
            // again can restart their negotiation. v1 retains its match ACK.
            if !repeated || major == 1 {
                out.outgoing.push(cmd(VERSION_MATCH, integers(&[(1, 1)]))?);
            }
            if !repeated {
                out.outgoing.push(cmd(FOREGROUND, Vec::new())?);
                out.outgoing.push(cmd(SCREEN_ON, Vec::new())?);
                out.outgoing.push(cmd(MD_INFO, device_info())?);
            }
            out.events.push(Event::ProtocolVersion { major, minor, repeated });
            return Ok(out);
        }
        if self.version.is_none() { return Err(Error::InvalidState("HU_PROTOCOL_VERSION required first")); }
        match message.id {
            HU_INFO => {
                fields(&message.payload)?;
                out.outgoing.push(cmd(MD_INFO, device_info())?);
                // No media/navigation/multitouch capability producer yet.
                out.outgoing.push(cmd(FEATURE_REQUEST, integers(&[(1, 0)]))?);
                out.events.push(Event::HuInfoReceived);
            },
            STATISTIC_INFO => {
                fields(&message.payload)?;
                // Deliberately no MD_AUTHEN_RESULT(true) or CAR_DATA_SUBSCRIBE.
                out.events.push(Event::StatisticReceived);
            },
            VIDEO_INIT => {
                let f = fields(&message.payload)?;
                let (width, height, fps) = (uint(&f, 1)?, uint(&f, 2)?, uint(&f, 3)?);
                if !(16..=1920).contains(&width) || !(16..=1080).contains(&height)
                    || width % 2 != 0 || height % 2 != 0 || fps > 60 {
                    return Err(Error::InvalidVideoConfig);
                }
                // Java HUs use zero to request an unrestricted frame rate.
                let mut config = VideoConfig { width, height, fps: if fps == 0 { 30 } else { fps.min(30) } };
                if let Some(source) = self.source {
                    if (width, height) != (source.width, source.height) {
                        return Err(Error::Unsupported("test video dimensions differ from HU request; regenerate test assets"));
                    }
                    config.fps = config.fps.min(source.fps);
                }
                // Identical retries re-ACK without undoing START/PAUSE state.
                if self.video != Some(config) { self.video = Some(config); self.state = VideoState::Negotiated; }
                out.outgoing.push(cmd(VIDEO_INIT_DONE, integers(&[(1, width as u64), (2, height as u64), (3, config.fps as u64)]))?);
                out.events.push(Event::VideoNegotiated(config));
            },
            VIDEO_START => {
                no_payload(message)?;
                if self.video.is_none() { return Err(Error::InvalidState("VIDEO_START before INIT")); }
                self.state = VideoState::AwaitingVideoSource;
                out.events.push(Event::AwaitingVideoSource);
            },
            VIDEO_PAUSE => {
                no_payload(message)?;
                if self.video.is_none() { return Err(Error::InvalidState("VIDEO_PAUSE before INIT")); }
                self.state = VideoState::Paused;
                out.events.push(Event::VideoPaused);
            },
            VIDEO_RESET => {
                no_payload(message)?;
                self.generation = self.generation.checked_add(1).ok_or(Error::InvalidState("generation overflow"))?;
                // A HU RESET asks for a fresh key frame, preserving the mode
                // and playback state (including PAUSE), not another INIT.
                if self.video.is_none() { return Err(Error::InvalidState("VIDEO_RESET before INIT")); }
                out.events.push(Event::VideoReset { generation: self.generation });
            },
            VIDEO_FPS_CHANGE => {
                let fps = uint(&fields(&message.payload)?, 1)?;
                if !(1..=60).contains(&fps) { return Err(Error::InvalidVideoConfig); }
                let config = self.video.as_mut().ok_or(Error::InvalidState("FPS change before INIT"))?;
                config.fps = fps.min(30).min(self.source.map_or(30, |s| s.fps));
                out.outgoing.push(cmd(VIDEO_FPS_DONE, integers(&[(1, config.fps as u64)]))?);
                out.events.push(Event::FrameRateChanged { fps: config.fps });
            },
            HU_FEATURE_RESPONSE => {
                let f = fields(&message.payload)?;
                let count = uint(&f, 1)? as usize;
                let items = byte_fields(&f, 2)?;
                if count != items.len() || count > 64 { return Err(Error::InvalidProtobuf("feature count")); }
                for item in items {
                    let feature = fields(item)?;
                    let key = single_bytes(&feature, 1)?;
                    let value = uint(&feature, 2)?;
                    if key == b"CONTENT_ENCRYPTION" && value != 0 {
                        return Err(Error::Unsupported("encrypted CarLife media"));
                    }
                    if key == b"AUDIO_TRANSMISSION_MODE" {
                        if value > 1 { return Err(Error::Unsupported("unknown audio transmission mode")); }
                        out.events.push(Event::AudioTransport { usb: value == 0 });
                    }
                }
                out.events.push(Event::FeaturesReceived);
            },
            SUBSCRIBE | SUBSCRIBE_START | SUBSCRIBE_STOP => {
                let f = fields(&message.payload)?;
                let count = uint(&f, 1)? as usize;
                let items = byte_fields(&f, 2)?;
                if count != items.len() || count > 64 { return Err(Error::InvalidProtobuf("subscription count")); }
                let mut modules = Vec::new();
                for item in items {
                    let id = uint(&fields(item)?, 1)?;
                    if modules.contains(&id) { return Err(Error::InvalidProtobuf("duplicate subscription")); }
                    modules.push(id);
                }
                if message.id == SUBSCRIBE {
                    let mut body = integers(&[(1, modules.len() as u64)]);
                    for id in &modules { put_bytes(&mut body, 2, &integers(&[(1, *id as u64), (2, 0)])); }
                    out.outgoing.push(cmd(SUBSCRIBE_DONE, body)?);
                    out.events.push(Event::SubscriptionsUnsupported { modules });
                } else { out.events.push(Event::SubscriptionControlIgnored { id: message.id }); }
            },
            MODULE_CONTROL => {
                let f = fields(&message.payload)?;
                let (module, status) = (uint(&f, 1)?, uint(&f, 2)?);
                if module == 3 { out.events.push(Event::MusicControl { status }); }
                else { out.events.push(Event::ModuleControlIgnored { module, status }); }
            },
            _ => out.events.push(Event::Unhandled { channel: message.channel, id: message.id, bytes: message.payload.len() }),
        }
        Ok(out)
    }
}

fn no_payload(message: &Message) -> Result<(), Error> {
    if message.payload.is_empty() { Ok(()) } else { Err(Error::InvalidProtobuf("unexpected control payload")) }
}
fn cmd(id: u32, payload: Vec<u8>) -> Result<Message, Error> { Message::command(id, payload) }
fn device_info() -> Vec<u8> {
    // Virtual Android wire-compatibility profile: this is the CarLink Linux
    // implementation, not a change to the real OS or a third-party identity.
    // Field 24 is carlifeVersion and 25 carlifeType in the newer HeadSwap
    // schema; older public proto snapshots label field 24 as btname.
    let mut out = Vec::new();
    for (field, value) in [(1, "Android"), (2, "CarLink"), (4, "CarLink"), (13, "CarLink"),
        (14, "CarLink Linux MD"), (15, "CarLink"), (20, "29"),
        (24, option_env!("CARGO_PKG_VERSION").unwrap_or("0.1.0"))] {
        put_bytes(&mut out, field, value.as_bytes());
    }
    out.extend(integers(&[(21, 29), (25, 1)]));
    out
}

#[derive(Clone, Copy)]
enum Value<'a> { Integer(u64), Bytes(&'a [u8]), Fixed }
struct Field<'a> { number: u32, value: Value<'a> }
fn varint(bytes: &[u8], cursor: &mut usize) -> Result<u64, Error> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *bytes.get(*cursor).ok_or(Error::InvalidProtobuf("truncated varint"))?;
        *cursor += 1;
        if shift == 63 && byte > 1 { return Err(Error::InvalidProtobuf("varint overflow")); }
        value |= u64::from(byte & 127) << shift;
        if byte & 128 == 0 { return Ok(value); }
    }
    Err(Error::InvalidProtobuf("varint overflow"))
}
fn fields(bytes: &[u8]) -> Result<Vec<Field<'_>>, Error> {
    if bytes.len() > 65535 { return Err(Error::LimitExceeded("protobuf bytes")); }
    let mut cursor = 0;
    let mut out = Vec::new();
    while cursor < bytes.len() {
        if out.len() == 128 { return Err(Error::LimitExceeded("protobuf fields")); }
        let tag = varint(bytes, &mut cursor)?;
        if tag >> 3 == 0 || tag >> 3 >= 1 << 29 { return Err(Error::InvalidProtobuf("field number")); }
        let value = match tag & 7 {
            0 => Value::Integer(varint(bytes, &mut cursor)?),
            wire @ (1 | 2 | 5) => {
                let length = if wire == 2 {
                    usize::try_from(varint(bytes, &mut cursor)?).map_err(|_| Error::InvalidProtobuf("length overflow"))?
                } else if wire == 1 { 8 } else { 4 };
                if length > bytes.len() - cursor { return Err(Error::InvalidProtobuf("truncated field")); }
                let data = &bytes[cursor..cursor + length]; cursor += length;
                if wire == 2 { Value::Bytes(data) } else { Value::Fixed }
            },
            _ => return Err(Error::InvalidProtobuf("unsupported wire type")),
        };
        out.push(Field { number: (tag >> 3) as u32, value });
    }
    Ok(out)
}
fn single<'a>(fields: &'a [Field<'a>], number: u32) -> Result<Value<'a>, Error> {
    let mut matching = fields.iter().filter(|f| f.number == number);
    let value = matching.next().ok_or(Error::InvalidProtobuf("required field absent"))?.value;
    if matching.next().is_some() { return Err(Error::InvalidProtobuf("duplicate scalar field")); }
    Ok(value)
}
fn uint(fields: &[Field<'_>], number: u32) -> Result<u32, Error> {
    match single(fields, number)? {
        Value::Integer(value) => u32::try_from(value).map_err(|_| Error::InvalidProtobuf("integer out of range")),
        _ => Err(Error::InvalidProtobuf("integer field has wrong wire type")),
    }
}
fn single_bytes<'a>(fields: &'a [Field<'a>], number: u32) -> Result<&'a [u8], Error> {
    match single(fields, number)? { Value::Bytes(bytes) => Ok(bytes), _ => Err(Error::InvalidProtobuf("bytes field has wrong wire type")) }
}
fn byte_fields<'a>(fields: &'a [Field<'a>], number: u32) -> Result<Vec<&'a [u8]>, Error> {
    fields.iter().filter(|f| f.number == number).map(|f| match f.value {
        Value::Bytes(bytes) => Ok(bytes), _ => Err(Error::InvalidProtobuf("repeated message has wrong wire type")),
    }).collect()
}
fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 128 { out.push(value as u8 | 128); value >>= 7; }
    out.push(value as u8);
}
fn integers(pairs: &[(u32, u64)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (field, value) in pairs { put_varint(&mut out, (*field as u64) << 3); put_varint(&mut out, *value); }
    out
}
fn put_bytes(out: &mut Vec<u8>, field: u32, bytes: &[u8]) {
    put_varint(out, ((field as u64) << 3) | 2); put_varint(out, bytes.len() as u64); out.extend_from_slice(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn outer(channel: Channel, bytes: &[u8]) -> Vec<u8> {
        let mut out = (channel as u32).to_be_bytes().to_vec();
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes()); out.extend_from_slice(bytes); out
    }
    fn version() -> Message { cmd(ids::HU_PROTOCOL_VERSION, integers(&[(1, 2), (2, 1)])).unwrap() }
    fn init() -> Message { cmd(ids::VIDEO_INIT, integers(&[(1, 1280), (2, 720), (3, 60)])).unwrap() }

    #[test]
    fn literal_wire_and_every_read_boundary() {
        let message = version();
        let wire = message.encode().unwrap();
        assert_eq!(wire, [0,0,0,1,0,0,0,12,0,4,0,0,0,1,128,1,8,2,16,1]);
        for boundary in 0..=wire.len() {
            let mut d=Decoder::default();
            let mut messages=d.feed(&wire[..boundary]).unwrap();
            messages.extend(d.feed(&wire[boundary..]).unwrap());
            assert_eq!(messages, vec![message.clone()]); d.finish().unwrap();
        }
    }
    #[test]
    fn one_byte_reads_and_concatenation() {
        let mut wire=version().encode().unwrap(); wire.extend(init().encode().unwrap());
        let mut d=Decoder::default(); let mut got=Vec::new();
        for b in wire { got.extend(d.feed(&[b]).unwrap()); }
        assert_eq!(got,vec![version(),init()]); d.finish().unwrap();
    }
    #[test]
    fn arbitrary_inner_splits_and_interleaved_channels() {
        let cmd=version(); let cmd_inner=&cmd.encode().unwrap()[8..];
        let media=Message{channel:Channel::Media,id:0x30006,timestamp_ms:Some(u32::MAX),reserved:0,payload:vec![1,2,3]};
        let media_wire=media.encode().unwrap(); let mut wire=Vec::new();
        wire.extend(outer(Channel::Cmd,&cmd_inner[..3]));
        wire.extend(outer(Channel::Media,&media_wire[8..15]));
        wire.extend(outer(Channel::Cmd,&cmd_inner[3..]));
        wire.extend(outer(Channel::Media,&media_wire[15..]));
        let mut d=Decoder::default(); assert_eq!(d.feed(&wire).unwrap(),vec![cmd,media]); d.finish().unwrap();
    }
    #[test]
    fn multiple_inners_in_one_outer_and_reserved_word_preserved() {
        let mut a=version(); a.reserved=0xbeef;
        let b=cmd(0x68002,vec![8,1]).unwrap();
        let mut inners=a.encode().unwrap()[8..].to_vec(); inners.extend(a.encode().unwrap()[8..].iter());
        let mut wire=outer(Channel::Cmd,&inners); wire.extend(b.encode().unwrap());
        let mut d=Decoder::default(); assert_eq!(d.feed(&wire).unwrap(),vec![a.clone(),a,b]); d.finish().unwrap();
    }
    #[test]
    fn every_truncated_outer_offset_fails_eof() {
        let wire=version().encode().unwrap();
        for length in 1..wire.len() {
            let mut d=Decoder::default(); d.feed(&wire[..length]).unwrap();
            assert!(matches!(d.finish(),Err(Error::TruncatedOuter{..})),"offset {length}");
            assert_eq!(d.feed(&[]),Err(Error::Closed));
        }
    }
    #[test]
    fn truncated_inner_is_distinct_and_error_is_terminal() {
        let mut d=Decoder::default();
        d.feed(&outer(Channel::Cmd,&version().encode().unwrap()[8..11])).unwrap();
        assert_eq!(d.finish(),Err(Error::TruncatedInner{channel:Channel::Cmd,buffered:3}));
        let mut bad=version().encode().unwrap(); bad[13]=2;
        let mut d=Decoder::default(); assert!(matches!(d.feed(&bad),Err(Error::InvalidFrame(_))));
        assert_eq!(d.feed(&version().encode().unwrap()),Err(Error::Closed));
    }
    #[test]
    fn declared_lengths_channels_and_memory_budgets_are_bounded() {
        let mut d=Decoder::default(); assert_eq!(d.feed(&[0,0,0,7,0,0,0,1]),Err(Error::UnknownChannel(7)));
        let mut d=Decoder::default(); assert!(matches!(d.feed(&[0,0,0,1,255,255,255,255]),Err(Error::LimitExceeded(_))));
        let mut d=Decoder::new(Limits{max_outer_payload:64,max_inner_payload:3,max_buffered:64});
        assert!(matches!(d.feed(&version().encode().unwrap()),Err(Error::LimitExceeded(_))));
        let mut d=Decoder::new(Limits{max_outer_payload:64,max_inner_payload:64,max_buffered:20});
        assert!(matches!(d.feed(&[0;21]),Err(Error::LimitExceeded("input batch"))));
        let mut d=Decoder::new(Limits{max_outer_payload:0,..Limits::default()}); assert_eq!(d.feed(&[]),Err(Error::InvalidLimits));
        let mut d=Decoder::new(Limits{max_outer_payload:usize::MAX,..Limits::default()}); assert_eq!(d.feed(&[]),Err(Error::InvalidLimits));
    }
    #[test]
    fn maximum_command_payload_roundtrips() {
        let a=cmd(0x68001,vec![0xaa;65535]).unwrap();
        let mut d=Decoder::default(); assert_eq!(d.feed(&a.encode().unwrap()).unwrap(),vec![a]); d.finish().unwrap();
        assert!(Message::command(0x18001,vec![0;65536]).is_err());
    }
    #[test]
    fn handshake_retry_does_not_restart_video_or_claim_authentication() {
        let mut s=Session::new(); let first=s.handle(&version()).unwrap();
        assert_eq!(first.outgoing.iter().map(|m|m.id).collect::<Vec<_>>(),vec![ids::VERSION_MATCH,ids::FOREGROUND,ids::SCREEN_ON,ids::MD_INFO]);
        s.handle(&init()).unwrap(); s.handle(&cmd(ids::VIDEO_START,vec![]).unwrap()).unwrap();
        let repeat=s.handle(&version()).unwrap(); assert!(repeat.outgoing.is_empty());
        assert_eq!(s.video_state(),VideoState::AwaitingVideoSource);
        let stats=s.handle(&cmd(ids::STATISTIC_INFO,vec![]).unwrap()).unwrap(); assert!(stats.outgoing.is_empty());
    }
    #[test]
    fn md_info_uses_own_named_android_wire_compatibility_profile() {
        let payload=device_info(); let f=fields(&payload).unwrap();
        assert_eq!(single_bytes(&f,1).unwrap(),b"Android");
        assert_eq!(single_bytes(&f,4).unwrap(),b"CarLink");
        assert_eq!(single_bytes(&f,13).unwrap(),b"CarLink");
        assert_eq!(single_bytes(&f,20).unwrap(),b"29");
        assert_eq!(uint(&f,21).unwrap(),29); assert_eq!(uint(&f,25).unwrap(),1);
    }
    #[test]
    fn v1_retry_only_repeats_version_match() {
        let v1=cmd(ids::HU_PROTOCOL_VERSION,integers(&[(1,1),(2,0)])).unwrap();
        let mut s=Session::new(); s.handle(&v1).unwrap();
        let reply=s.handle(&v1).unwrap();
        assert_eq!(reply.outgoing.iter().map(|m|m.id).collect::<Vec<_>>(),vec![ids::VERSION_MATCH]);
    }
    #[test]
    fn video_reset_preserves_mode_and_pause_then_start_resumes() {
        let mut s=Session::new(); s.handle(&version()).unwrap();
        s.handle(&init()).unwrap(); assert_eq!(s.video_config(),Some(VideoConfig{width:1280,height:720,fps:30}));
        let start=s.handle(&cmd(ids::VIDEO_START,vec![]).unwrap()).unwrap();
        assert!(start.outgoing.is_empty()); assert_eq!(start.events,vec![Event::AwaitingVideoSource]);
        s.handle(&cmd(ids::VIDEO_PAUSE,vec![]).unwrap()).unwrap(); assert_eq!(s.video_state(),VideoState::Paused);
        s.handle(&init()).unwrap(); assert_eq!(s.video_state(),VideoState::Paused);
        s.handle(&cmd(ids::VIDEO_RESET,vec![]).unwrap()).unwrap();
        assert_eq!(s.video_config(),Some(VideoConfig{width:1280,height:720,fps:30}));
        assert_eq!(s.video_state(),VideoState::Paused);
        s.handle(&cmd(ids::VIDEO_START,vec![]).unwrap()).unwrap();
        assert_eq!(s.video_state(),VideoState::AwaitingVideoSource);
    }
    #[test]
    fn bad_video_parameters_are_rejected_transactionally() {
        for (w,h,fps) in [(15,720,30),(1281,720,30),(1922,720,30),(1280,1082,30),(1280,720,61)] {
            let mut s=Session::new(); s.handle(&version()).unwrap();
            let msg=cmd(ids::VIDEO_INIT,integers(&[(1,w),(2,h),(3,fps)])).unwrap();
            assert_eq!(s.handle(&msg),Err(Error::InvalidVideoConfig)); assert_eq!(s.video_config(),None); assert!(s.is_failed());
        }
    }
    #[test]
    fn hu_zero_fps_and_source_limits_use_actual_encoder_parameters() {
        let config = VideoConfig { width:1280, height:720, fps:15 };
        let mut s=Session::with_video_source(config); s.handle(&version()).unwrap();
        let zero=cmd(ids::VIDEO_INIT,integers(&[(1,1280),(2,720),(3,0)])).unwrap();
        s.handle(&zero).unwrap(); assert_eq!(s.video_config(),Some(config));
        let mismatch=cmd(ids::VIDEO_INIT,integers(&[(1,800),(2,480),(3,30)])).unwrap();
        assert!(matches!(s.handle(&mismatch),Err(Error::Unsupported(_))));
        assert_eq!(s.video_config(),Some(config));
        let mut normal=Session::new(); normal.handle(&version()).unwrap();
        normal.handle(&zero).unwrap(); assert_eq!(normal.video_config().unwrap().fps,30);
    }
    #[test]
    fn android_hu_4_1_uses_base_handshake_without_duplicate_startup() {
        let request=cmd(ids::HU_PROTOCOL_VERSION,integers(&[(1,4),(2,1)])).unwrap();
        let mut s=Session::new();
        assert_eq!(s.handle(&request).unwrap().outgoing.len(),4);
        assert_eq!(s.protocol_version(),Some((4,1)));
        assert!(s.handle(&request).unwrap().outgoing.is_empty());
        assert!(matches!(s.handle(&cmd(ids::HU_AUTH_REQUEST,vec![]).unwrap()),Err(Error::Unsupported(_))));
    }
    #[test]
    fn protobuf_bad_wire_overflow_missing_duplicate_fields() {
        for body in [vec![0],vec![8,128],vec![10,5,1],vec![8,2],vec![8,2,8,2,16,1],vec![8,255,255,255,255,255,255,255,255,255,2]] {
            let mut s=Session::new(); assert!(s.handle(&cmd(ids::HU_PROTOCOL_VERSION,body).unwrap()).is_err());
        }
        let mut s=Session::new(); assert_eq!(s.handle(&cmd(ids::HU_PROTOCOL_VERSION,integers(&[(1,3),(2,0)])).unwrap()),Err(Error::UnsupportedVersion{major:3,minor:0}));
    }
    #[test]
    fn auth_and_media_encryption_fail_closed() {
        for id in [ids::HU_AUTH_REQUEST,ids::HU_AUTH_RESULT,ids::HU_RSA_KEY] {
            let mut s=Session::new(); assert!(matches!(s.handle(&cmd(id,vec![]).unwrap()),Err(Error::Unsupported(_))));
            assert_eq!(s.handle(&version()),Err(Error::Closed));
        }
        let mut body=integers(&[(1,1)]); let mut feature=Vec::new(); put_bytes(&mut feature,1,b"CONTENT_ENCRYPTION"); feature.extend(integers(&[(2,1)])); put_bytes(&mut body,2,&feature);
        let mut s=Session::new(); s.handle(&version()).unwrap();
        assert_eq!(s.handle(&cmd(ids::HU_FEATURE_RESPONSE,body).unwrap()),Err(Error::Unsupported("encrypted CarLife media")));
    }
    #[test]
    fn subscriptions_report_every_requested_module_unsupported() {
        let mut request=integers(&[(1,3)]); for id in [0,2,99] { put_bytes(&mut request,2,&integers(&[(1,id)])); }
        let mut s=Session::new(); s.handle(&version()).unwrap();
        let reply=s.handle(&cmd(ids::SUBSCRIBE,request).unwrap()).unwrap();
        assert_eq!(reply.outgoing[0].id,ids::SUBSCRIBE_DONE);
        let f=fields(&reply.outgoing[0].payload).unwrap();
        for item in byte_fields(&f,2).unwrap() { assert_eq!(uint(&fields(item).unwrap(),2).unwrap(),0); }
        assert_eq!(reply.events,vec![Event::SubscriptionsUnsupported{modules:vec![0,2,99]}]);
    }
}
