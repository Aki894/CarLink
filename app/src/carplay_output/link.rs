//! Bounded, single-owner CarPlay -> CarLife IPC. No USB resources live here.
use super::Result;
use serde_json::{json, Value};
use std::{collections::VecDeque, ffi::c_char, io::{self, ErrorKind, Read, Write},
    os::unix::{ffi::OsStrExt, io::{AsRawFd, FromRawFd}, net::UnixStream},
    path::{Path, PathBuf}, time::{Duration, Instant}};

const MAX_QUEUE: usize = 1024 * 1024;
const MAX_VIDEO: usize = 512 * 1024;
const MAX_CONTROL: usize = 32;
const MEDIA_AGE: Duration = Duration::from_millis(120);
const WRITE_DEADLINE: Duration = Duration::from_millis(200);

pub enum Feedback {
    Control(Value),
    Microphone { generation: u64, pcm: Vec<u8> },
}

struct Pending {
    kind: u8, flags: u8, wire: Vec<u8>, offset: usize,
    queued: Instant, started: Option<Instant>,
}
impl Pending {
    fn new(kind: u8, flags: u8, pts: u64, payload: &[u8]) -> Self {
        let mut wire = Vec::with_capacity(payload.len() + 16);
        wire.extend(((payload.len() + 12) as u32).to_le_bytes());
        wire.extend([kind, flags, 0, 0]);
        wire.extend(pts.to_le_bytes());
        wire.extend(payload);
        Self { kind, flags, wire, offset: 0, queued: Instant::now(), started: None }
    }
}

pub struct Link {
    path: PathBuf, socket: Option<UnixStream>, queue: VecDeque<Pending>, size: usize,
    input: Vec<u8>, events: VecDeque<Feedback>, retry: Instant, closed: bool,
    active: bool, config: Option<Vec<u8>>, microphone: Option<Vec<u8>>, roles: [bool; 4],
    wait_key: bool, last_recovery: Option<Instant>, blocked: Option<Instant>,
    sent: u64, dropped: u64, reconnects: u64, queue_max: f64, send_max: f64,
    video_frames: u64, video_bytes: u64, report_at: Instant,
}

impl Link {
    pub fn new(path: PathBuf) -> Self {
        Self { path, socket: None, queue: VecDeque::new(), size: 0, input: vec![],
            events: VecDeque::new(), retry: Instant::now(), closed: false, active: false,
            config: None, microphone: None, roles: [false; 4], wait_key: true,
            last_recovery: None, blocked: None, sent: 0, dropped: 0, reconnects: 0,
            queue_max: 0., send_max: 0., video_frames: 0, video_bytes: 0, report_at: Instant::now() }
    }
    pub fn connected(&self) -> bool { self.socket.is_some() }
    fn recount(&mut self) { self.size = self.queue.iter().map(|p| p.wire.len()).sum(); }
    fn emit(&mut self, event: Feedback) {
        // Only locally generated keyframe/release events use this queue. A
        // bounded caller that does not service feedback must not grow memory.
        if self.events.len() < 64 { self.events.push_back(event); }
    }
    fn clear_complete(&mut self) {
        self.queue.retain(|p| p.offset > 0);
        self.recount(); self.wait_key = true;
    }
    fn put(&mut self, kind: u8, flags: u8, pts: u64, payload: &[u8]) {
        if self.queue.len() >= MAX_CONTROL || self.size + payload.len() + 16 > MAX_QUEUE {
            // Failure closes this source IPC only. Cached demand is replayed
            // after reconnect; CarLife independently owns the physical USB.
            self.disconnect();
            return;
        }
        let frame = Pending::new(kind, flags, pts, payload);
        self.size += frame.wire.len(); self.queue.push_back(frame);
    }
    pub fn session(&mut self, active: bool) {
        self.active = active;
        if !active { self.microphone = None; self.roles = [false; 4]; }
        self.clear_complete();
        self.put(4, u8::from(active), 0, &[]);
        if let Some(config) = self.config.clone() { self.put(1, 0, 0, &config); }
    }
    pub fn configure(&mut self, width: u32, height: u32, fps: u32) -> Result<()> {
        if !(16..=1920).contains(&width) || !(16..=1080).contains(&height)
            || width % 2 != 0 || height % 2 != 0 || !(1..=60).contains(&fps) {
            return Err("unsupported live dimensions".into());
        }
        let config = [width, height, fps].into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>();
        if self.config.as_ref() == Some(&config) {
            self.recover_video();
            return Ok(());
        }
        self.config = Some(config.clone());
        // A format update is a video fence, not an audio/microphone epoch.
        // Put complete demand transitions after the new configuration: an
        // initial microphone request before kind 1 would otherwise be ignored
        // by the CarLife source as unconfigured. An already-started frame must
        // still finish before any replacement envelope is written.
        let mut controls = VecDeque::new();
        self.queue.retain_mut(|p| {
            if p.offset == 0 && matches!(p.kind, 5 | 7) {
                controls.push_back(Pending::new(p.kind, p.flags, 0, &p.wire[16..]));
                false
            } else { p.offset > 0 }
        });
        self.recount(); self.wait_key = true;
        self.put(4, u8::from(self.active), 0, &[]); self.put(1, 0, 0, &config);
        for control in controls { self.put(control.kind, control.flags, 0, &control.wire[16..]); }
        if let Some(microphone) = self.microphone.clone() {
            if !self.queue.iter().any(|p| p.kind == 7 && p.offset == 0 && p.wire[16..] == microphone) {
                self.put(7, 0, 0, &microphone);
            }
        }
        Ok(())
    }
    pub fn audio_state(&mut self, role: u8, active: bool) {
        if role > 3 { return; }
        self.roles[role as usize] = active;
        self.queue.retain(|p| p.offset > 0 || p.kind != 3
            || p.flags != role && !(role == 3 && active && p.flags < 3));
        self.recount(); self.put(5, role, 0, &[u8::from(active)]);
        if role == 3 { println!("{}", json!({"event":"call_state","active":active})); }
    }
    pub fn microphone(&mut self, value: &Value) -> Result<()> {
        if value["generation"].as_u64().is_none_or(|n| n == 0)
            || value["active"].as_bool().is_none()
            || value["sample_rate"] != 16000 || value["channels"] != 1 {
            return Err("invalid live microphone demand".into());
        }
        let bytes = serde_json::to_vec(value).map_err(|_| "microphone serialization failed")?;
        if bytes.len() > 1024 { return Err("microphone demand exceeds bound".into()); }
        self.microphone = (value["active"] == true).then(|| bytes.clone());
        self.put(7, 0, 0, &bytes); Ok(())
    }
    pub fn recover_video(&mut self) {
        self.queue.retain(|p| p.offset > 0 || !matches!(p.kind, 2 | 6)); self.recount();
        self.wait_key = true; self.put(6, 0, 0, &[]);
        if self.last_recovery.is_none_or(|at| at.elapsed() >= Duration::from_millis(250)) {
            self.last_recovery = Some(Instant::now());
            self.emit(Feedback::Control(json!({"type":"keyframe","screen":0})));
        }
    }
    pub fn media(&mut self, kind: u8, payload: Vec<u8>, pts: u64, flags: u8) -> bool {
        let valid = match kind {
            2 => flags <= 3 && !payload.is_empty() && payload.len() <= MAX_VIDEO,
            3 => flags <= 3 && !payload.is_empty() && payload.len() <= 65536 && payload.len() % 4 == 0,
            _ => false,
        };
        if !valid || !self.connected() || !self.active || self.config.is_none()
            || kind == 2 && self.wait_key && flags & 1 == 0
            || kind == 3 && self.roles[3] && flags < 3 {
            self.dropped += 1; return false;
        }
        let limit = if kind == 2 { 4 } else { 6 };
        if self.queue.len() >= MAX_CONTROL || self.size + payload.len() + 16 > MAX_QUEUE
            || self.queue.iter().filter(|p| p.kind == kind).count() >= limit {
            self.dropped += 1; if kind == 2 { self.recover_video(); } return false;
        }
        if kind == 2 { self.wait_key = false; self.video_frames += 1; self.video_bytes += payload.len() as u64; }
        self.put(kind, flags, pts, &payload); true
    }
    fn disconnect(&mut self) {
        let had_socket = self.socket.take().is_some();
        self.queue.clear(); self.size = 0; self.input.clear(); self.wait_key = true;
        self.blocked = None; self.retry = Instant::now() + Duration::from_millis(250);
        if had_socket { self.emit(Feedback::Control(json!({"type":"touch","contacts":[]}))); }
    }
    fn connect(&mut self) {
        if self.closed || self.socket.is_some() || Instant::now() < self.retry { return; }
        self.retry = Instant::now() + Duration::from_millis(250);
        let Ok(socket) = connect_output(&self.path) else { return; };
        self.socket = Some(socket); self.queue.clear(); self.size = 0; self.input.clear();
        self.wait_key = true; self.blocked = None; self.reconnects += 1;
        self.put(4, u8::from(self.active), 0, &[]);
        if let Some(config) = self.config.clone() { self.put(1, 0, 0, &config); }
        for role in 0..4 { if self.roles[role] { self.put(5, role as u8, 0, &[1]); } }
        if let Some(microphone) = self.microphone.clone() { self.put(7, 0, 0, &microphone); }
        self.emit(Feedback::Control(json!({"type":"keyframe","screen":0})));
    }
    fn write(&mut self) -> io::Result<()> {
        for _ in 0..32 {
            let Some(front) = self.queue.front() else { self.blocked = None; break; };
            if front.offset == 0 && matches!(front.kind, 2 | 3) && front.queued.elapsed() > MEDIA_AGE {
                let old = self.queue.pop_front().unwrap(); self.size -= old.wire.len(); self.dropped += 1;
                if old.kind == 2 { self.recover_video(); } continue;
            }
            if front.started.is_some_and(|at| at.elapsed() > WRITE_DEADLINE)
                || self.blocked.is_some_and(|at| at.elapsed() > WRITE_DEADLINE) {
                return Err(io::Error::new(ErrorKind::TimedOut, "live write stalled"));
            }
            let Some(socket) = self.socket.as_mut() else { break; };
            let front = self.queue.front_mut().unwrap();
            match socket.write(&front.wire[front.offset..]) {
                Ok(0) => return Err(io::Error::new(ErrorKind::WriteZero, "live output closed")),
                Ok(n) => {
                    self.blocked = None; front.started.get_or_insert_with(Instant::now); front.offset += n;
                    if front.offset == front.wire.len() {
                        let done = self.queue.pop_front().unwrap(); self.size -= done.wire.len(); self.sent += 1;
                        self.queue_max = self.queue_max.max(done.queued.elapsed().as_secs_f64() * 1000.);
                        self.send_max = self.send_max.max(done.started.unwrap().elapsed().as_secs_f64() * 1000.);
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    self.blocked.get_or_insert_with(Instant::now); break;
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        } Ok(())
    }
    fn parse_input(&mut self, out: &mut Vec<Feedback>) -> Result<()> {
        while self.input.len() >= 4 {
            let size = u32::from_le_bytes(self.input[..4].try_into().unwrap()) as usize;
            if !(12..=16404).contains(&size) { return Err("invalid live feedback length".into()); }
            if self.input.len() < size + 4 { break; }
            let kind = self.input[4];
            if !matches!(kind, 129 | 130) || self.input[5..8] != [0, 0, 0] {
                return Err("invalid live feedback header".into());
            }
            let payload = &self.input[16..size + 4];
            if kind == 130 {
                if payload.len() < 10 || payload.len() % 2 != 0 { return Err("invalid microphone feedback".into()); }
                let generation = u64::from_le_bytes(payload[..8].try_into().unwrap());
                if generation == 0 { return Err("invalid microphone generation".into()); }
                out.push(Feedback::Microphone { generation, pcm: payload[8..].to_vec() });
            } else {
                if size > 4096 { return Err("oversized live control JSON".into()); }
                let value: Value = serde_json::from_slice(payload).map_err(|_| "invalid live control JSON")?;
                if !value.is_object() { return Err("live control must be object".into()); }
                out.push(Feedback::Control(value));
            }
            self.input.drain(..size + 4);
        } Ok(())
    }
    fn read(&mut self, out: &mut Vec<Feedback>) -> Result<()> {
        let mut bytes = [0u8; 4096];
        for _ in 0..16 {
            let Some(socket) = self.socket.as_mut() else { break; };
            match socket.read(&mut bytes) {
                Ok(0) => return Err("live feedback closed".into()),
                Ok(n) => {
                    self.input.extend_from_slice(&bytes[..n]);
                    if self.input.len() > 32768 { return Err("live input exceeds bound".into()); }
                    self.parse_input(out)?;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(_) => return Err("live feedback read failed".into()),
            }
        } Ok(())
    }
    pub fn step(&mut self) -> Vec<Feedback> {
        self.connect();
        let mut output = Vec::new();
        if self.socket.is_some() && (self.write().is_err() || self.read(&mut output).is_err()) { self.disconnect(); }
        output.extend(self.events.drain(..)); output
    }
    pub fn report(&mut self) -> Value {
        let seconds = self.report_at.elapsed().as_secs_f64().max(0.001);
        let report = json!({"event":"forward_latency","sent":self.sent,"dropped":self.dropped,
            "reconnects":self.reconnects,"pending":self.queue.len(),"pending_bytes":self.size,
            "queue_ms_max":self.queue_max,"send_ms_max":self.send_max,
            "source_video_fps":self.video_frames as f64 / seconds,
            "source_video_mbps":self.video_bytes as f64 * 8. / seconds / 1_000_000.});
        self.queue_max = 0.; self.send_max = 0.; self.video_frames = 0; self.video_bytes = 0;
        self.report_at = Instant::now(); report
    }
    pub fn close(&mut self) {
        if self.closed { return; }
        self.session(false);
        let until = Instant::now() + Duration::from_millis(100);
        while self.socket.is_some() && !self.queue.is_empty() && Instant::now() < until {
            if self.write().is_err() { break; }
            std::thread::sleep(Duration::from_millis(1));
        }
        self.closed = true; self.socket = None; self.queue.clear(); self.size = 0;
    }
}
impl Drop for Link { fn drop(&mut self) { self.close(); } }

fn connect_output(path: &Path) -> io::Result<UnixStream> {
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= address.sun_path.len() { return Err(io::Error::new(ErrorKind::InvalidInput, "live socket path too long")); }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (dst, src) in address.sun_path.iter_mut().zip(bytes) { *dst = *src as c_char; }
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 { return Err(io::Error::last_os_error()); }
    let socket = unsafe { UnixStream::from_raw_fd(fd) };
    let size = 32768i32;
    if unsafe { libc::setsockopt(socket.as_raw_fd(), libc::SOL_SOCKET, libc::SO_SNDBUF,
        &size as *const _ as *const _, std::mem::size_of_val(&size) as libc::socklen_t) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::connect(socket.as_raw_fd(), &address as *const _ as *const libc::sockaddr,
        std::mem::size_of_val(&address) as libc::socklen_t) } != 0 { return Err(io::Error::last_os_error()); }
    Ok(socket)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::unix::net::UnixListener, sync::atomic::{AtomicU64, Ordering}};
    fn path() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!("carlink-output-link-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)))
    }
    fn pair() -> (Link, UnixStream) {
        let (client, peer) = UnixStream::pair().unwrap(); client.set_nonblocking(true).unwrap();
        let mut link = Link::new(PathBuf::from("/not-used")); link.socket = Some(client);
        link.active = true; link.config = Some(vec![0; 12]); link.wait_key = false;
        (link, peer)
    }
    fn feedback(kind: u8, payload: &[u8]) -> Vec<u8> { Pending::new(kind, 0, 0, payload).wire }
    fn messages(bytes: &[u8]) -> Vec<(u8, u8, Vec<u8>)> {
        let mut out = vec![]; let mut at = 0;
        while at < bytes.len() {
            let size = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
            out.push((bytes[at + 4], bytes[at + 5], bytes[at + 16..at + size + 4].to_vec())); at += size + 4;
        } out
    }
    #[test] fn partial_video_survives_recovery_and_audio_transition() {
        let (mut link, mut peer) = pair();
        let video = vec![7; 65536]; assert!(link.media(2, video.clone(), 1, 1));
        let initial = link.queue.front().unwrap().wire[..8].to_vec();
        link.socket.as_mut().unwrap().write_all(&initial).unwrap();
        link.queue.front_mut().unwrap().offset = 8;
        link.queue.front_mut().unwrap().started = Some(Instant::now());
        link.recover_video(); link.audio_state(3, true);
        assert_eq!(link.queue.front().unwrap().offset, 8);
        peer.set_nonblocking(true).unwrap(); let mut got = Vec::new();
        let expected = link.size; let until = Instant::now() + Duration::from_secs(1);
        while got.len() < expected && Instant::now() < until {
            link.step(); let mut buffer = [0; 16384];
            loop { match peer.read(&mut buffer) { Ok(n) if n > 0 => got.extend_from_slice(&buffer[..n]), _ => break } }
        }
        let frames = messages(&got); assert_eq!(frames[0], (2, 1, video));
        assert!(frames.iter().any(|f| f.0 == 6)); assert!(frames.iter().any(|f| f.0 == 5 && f.1 == 3));
    }
    #[test] fn feedback_fragmentation_microphone_and_invalid_header_are_isolated() {
        let (mut link, mut peer) = pair();
        let mut wire = feedback(129, br#"{"type":"keyframe","screen":0}"#);
        wire.extend(feedback(130, &[17u64.to_le_bytes().as_slice(), &[1, 2, 3, 4]].concat()));
        peer.write_all(&wire[..7]).unwrap(); assert!(link.step().is_empty());
        peer.write_all(&wire[7..]).unwrap(); let events = link.step();
        assert!(events.iter().any(|e| matches!(e, Feedback::Control(v) if v["type"]=="keyframe")));
        assert!(events.iter().any(|e| matches!(e, Feedback::Microphone{generation:17,pcm} if pcm==&vec![1,2,3,4])));
        let mut bad = feedback(129, b"{}"); bad[6] = 1; peer.write_all(&bad).unwrap();
        let events = link.step(); assert!(!link.connected());
        assert!(events.iter().any(|e| matches!(e, Feedback::Control(v) if v["type"]=="touch")));
    }
    #[test] fn reconnect_replays_active_configuration_and_microphone_demand() {
        let path = path(); let listener = UnixListener::bind(&path).unwrap();
        let mut link = Link::new(path.clone()); link.session(true); link.configure(800, 480, 30).unwrap();
        link.audio_state(3, true); link.microphone(&json!({"generation":9,"active":true,"sample_rate":16000,"channels":1})).unwrap();
        for _ in 0..2 {
            link.retry = Instant::now(); let events = link.step();
            assert!(events.iter().any(|e| matches!(e, Feedback::Control(v) if v["type"]=="keyframe")));
            let (mut peer, _) = listener.accept().unwrap(); peer.set_nonblocking(true).unwrap();
            let mut got = vec![]; let mut buffer = [0; 4096];
            loop { match peer.read(&mut buffer) { Ok(n) if n > 0 => got.extend_from_slice(&buffer[..n]), _ => break } }
            let frames = messages(&got);
            assert_eq!(frames[0].0, 4); assert_eq!(frames[0].1, 1); assert_eq!(frames[1].0, 1);
            assert!(frames.iter().any(|f| f.0 == 5 && f.1 == 3 && f.2 == vec![1]));
            assert!(frames.iter().any(|f| f.0 == 7 && serde_json::from_slice::<Value>(&f.2).unwrap()["generation"]==9));
            drop(peer); link.step(); assert!(!link.connected());
        }
        drop(link); drop(listener); std::fs::remove_file(path).unwrap();
    }
    #[test] fn bounded_controls_replay_latest_demand_and_media_expires_whole() {
        let (mut link, _peer) = pair();
        assert!(link.media(3, vec![0; 3528], 0, 0)); link.queue.front_mut().unwrap().queued -= Duration::from_secs(1);
        link.step(); assert_eq!(link.dropped, 1); assert!(link.queue.is_empty());
        for i in 0..100 { link.audio_state(3, i % 2 == 0); }
        assert!(link.queue.len() <= MAX_CONTROL); assert!(link.size <= MAX_QUEUE); assert!(!link.roles[3]);
    }
    #[test] fn video_configuration_keeps_pending_call_stop_and_microphone_demand() {
        let (mut link, _peer) = pair(); link.config = None;
        link.audio_state(3, true); link.audio_state(3, false);
        link.microphone(&json!({"generation":29,"active":true,"sample_rate":16000,"channels":1})).unwrap();
        link.configure(800, 480, 30).unwrap();
        let frames = link.queue.iter().map(|p| (p.kind, p.flags, p.wire[16..].to_vec())).collect::<Vec<_>>();
        assert_eq!(frames[0].0, 4); assert_eq!(frames[1].0, 1);
        let transitions = frames.iter().filter(|p| p.0 == 5).map(|p| p.2.clone()).collect::<Vec<_>>();
        assert_eq!(transitions, vec![vec![1], vec![0]]);
        assert_eq!(frames.iter().filter(|p| p.0 == 7).count(), 1);
        assert_eq!(serde_json::from_slice::<Value>(&frames.last().unwrap().2).unwrap()["generation"], 29);
    }
    #[test] fn partial_microphone_demand_is_replayed_after_first_configuration() {
        let (mut link, _peer) = pair(); link.config = None;
        link.microphone(&json!({"generation":31,"active":true,"sample_rate":16000,"channels":1})).unwrap();
        link.queue.front_mut().unwrap().offset = 8;
        link.configure(800, 480, 30).unwrap();
        let frames = link.queue.iter().map(|p| (p.kind, p.offset)).collect::<Vec<_>>();
        assert_eq!(frames, vec![(7,8),(4,0),(1,0),(7,0)]);
    }
    #[test] fn call_takeover_keeps_partial_pcm_but_reserves_the_remaining_budget() {
        let (mut link, _peer) = pair();
        assert!(link.media(3, vec![1; 3528], 0, 0));
        link.queue.front_mut().unwrap().offset = 8;
        for role in [0,1,2,0,1] { assert!(link.media(3, vec![2; 3528], 0, role)); }
        link.audio_state(3, true);
        assert_eq!(link.queue.iter().filter(|p| p.kind == 3).count(), 1);
        assert_eq!(link.queue.front().unwrap().offset, 8);
        for role in 0..3 { assert!(!link.media(3, vec![3; 3528], 1, role)); }
        assert!(link.media(3, vec![4; 3528], 1, 3));
        link.audio_state(3, false);
        assert!(!link.queue.iter().any(|p| p.kind == 3 && p.flags == 3));
        assert!(link.media(3, vec![5; 3528], 2, 0));
    }
    #[test] fn unchanged_geometry_only_fences_video_and_keeps_call_controls() {
        let (mut link, _peer) = pair(); link.configure(800, 480, 30).unwrap();
        link.queue.clear(); link.size = 0;
        link.audio_state(3, true);
        link.microphone(&json!({"generation":33,"active":true,"sample_rate":16000,"channels":1})).unwrap();
        link.configure(800, 480, 30).unwrap();
        let kinds=link.queue.iter().map(|p|p.kind).collect::<Vec<_>>();
        assert_eq!(kinds, vec![5,7,6]); assert!(link.roles[3]); assert!(link.microphone.is_some());
        assert!(link.queue.iter().any(|p|p.kind==5&&p.flags==3&&p.wire[16..]==[1]));
    }
}
