// SPDX-License-Identifier: MIT
//! Bounded local live-media ingress. Never records media or parses private identities.
use crate::carlife::{Channel, Message, VideoConfig};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::io::{self, Read, Write};
use std::time::{Duration,Instant};
#[cfg(unix)]
use std::os::unix::{net::{UnixListener, UnixStream}, fs::{PermissionsExt, FileTypeExt}};

const MAX_FRAME: usize = 1024 * 1024;
const MAX_QUEUED: usize = 1024 * 1024;
pub const CONFIG: VideoConfig = VideoConfig { width: 800, height: 480, fps: 30 };
struct Frame { kind: u8, flags: u8, pts: u64, payload: Vec<u8>, arrived: Instant }

fn monotonic_us()->Option<u64>{
    #[cfg(target_os="linux")] {
        #[repr(C)]struct Time {sec:std::os::raw::c_long,nsec:std::os::raw::c_long}
        unsafe extern "C" {fn clock_gettime(clock:std::os::raw::c_int,time:*mut Time)->std::os::raw::c_int;}
        let mut time=Time{sec:0,nsec:0};
        if unsafe{clock_gettime(1,&mut time)}==0 && time.sec>=0{return Some(time.sec as u64*1_000_000+time.nsec as u64/1000);}
    }
    None
}
fn decode(body: &[u8]) -> Result<Frame, String> {
    if body.len() < 12 || body.len() > MAX_FRAME || body[2..4] != [0,0] { return Err("invalid live frame header".into()); }
    let f = Frame { kind: body[0], flags: body[1], pts: u64::from_le_bytes(body[4..12].try_into().unwrap()), payload: body[12..].to_vec(), arrived:Instant::now() };
    let valid = match f.kind {
        1 => f.flags == 0 && f.payload.len() == 12,
        2 => f.flags <= 3 && !f.payload.is_empty() && f.payload.len() <= 512*1024 && (f.payload.starts_with(&[0,0,0,1]) || f.payload.starts_with(&[0,0,1])),
        3 => f.flags <= 3 && !f.payload.is_empty() && f.payload.len() <= 64*1024 && f.payload.len()%4 == 0,
        4 => f.flags <= 1 && f.payload.is_empty(),
        5 => f.flags <= 3 && f.payload.len() == 1 && f.payload[0] <= 1,
        6 => f.flags==0 && f.payload.is_empty(),
        7 => f.flags==0 && f.payload.len()<=1024 && serde_json::from_slice::<serde_json::Value>(&f.payload).is_ok(),
        _ => false,
    };
    if !valid { return Err("invalid live media kind, flags or payload".into()); }
    Ok(f)
}
fn wire(json: &str) -> Vec<u8> {
    let mut data = ((12+json.len()) as u32).to_le_bytes().to_vec();
    data.extend_from_slice(&[129,0,0,0]); data.extend_from_slice(&0u64.to_le_bytes()); data.extend_from_slice(json.as_bytes()); data
}

pub struct LiveSource {
    #[cfg(unix)] listener: UnixListener,
    #[cfg(unix)] peer: Option<UnixStream>,
    path: PathBuf,
    input: Vec<u8>, output: VecDeque<Vec<u8>>, output_offset: usize,
    queue: VecDeque<Frame>, queued: usize,
    active: bool, configured: bool, playing: bool, audio_ready:bool, waiting_key: bool,
    audio_active: [bool; 4], audio_pending: VecDeque<Message>, voice:crate::voice::Voice,
    base_pts: Option<u64>, origin_ms: u32, last_ms: u32, stamps: [u32;5], generation: u64,
    report_at:Instant, video_dropped:u64, age_max_us:u64, audio_generation:u64, keyframe_at:Option<Instant>,call_media:bool,
}
impl LiveSource {
    pub fn bind(path: &Path) -> Result<Self,String> {
        #[cfg(not(unix))] { let _=path; return Err("live socket requires Linux/Unix".into()); }
        #[cfg(unix)] {
            if path.exists() { return Err("live socket already exists; refuse replacing another process endpoint".into()); }
            let listener=UnixListener::bind(path).map_err(|e|format!("live bind: {e}"))?;
            std::fs::set_permissions(path,std::fs::Permissions::from_mode(0o600)).map_err(|e|e.to_string())?;
            listener.set_nonblocking(true).map_err(|e|e.to_string())?;
            Ok(Self {listener,peer:None,path:path.into(),input:Vec::new(),output:VecDeque::new(),output_offset:0,queue:VecDeque::new(),queued:0,
                active:false,configured:false,playing:false,audio_ready:false,waiting_key:true,audio_active:[false;4],audio_pending:VecDeque::new(),voice:crate::voice::Voice::default(),base_pts:None,origin_ms:0,last_ms:0,stamps:[0;5],generation:0,report_at:Instant::now(),video_dropped:0,age_max_us:0,audio_generation:0,keyframe_at:None,call_media:std::env::var("CARLINK_CALL_AUDIO_LANE").as_deref()==Ok("media")})
        }
    }
    pub fn generation(&self)->[u64;2] { [self.generation,self.audio_generation] }
    pub fn accepts_input(&self)->bool {self.active && self.playing}
    pub fn microphone(&mut self,message:&Message){
        if !self.active || !self.configured || !self.audio_ready{return;}
        if let Some(wire)=self.voice.pcm(message){
            if self.output.len()>=32{self.disconnect();return;}
            self.output.push_back(wire);
        }
    }
    pub fn clock_floor(&mut self, ms:u32) {if self.base_pts.is_none(){self.origin_ms=self.last_ms.max(ms);}}
    pub fn heartbeat(&mut self, ms:u32)->u32 {self.stamps[0]=self.stamps[0].max(ms);self.last_ms=self.last_ms.max(ms);self.stamps[0]}
    pub fn feedback(&mut self,json:&str) {
        if json=="{\"type\":\"keyframe\",\"screen\":0}" {
            if self.keyframe_at.is_some_and(|at|at.elapsed()<Duration::from_millis(250)){return;}
            self.keyframe_at=Some(Instant::now());
        }
        // A slow consumer must not grow controls without bound. Dropping the
        // connection resets touch state at the receiver on reconnect.
        if self.output.len()>=32 { self.disconnect(); return; }
        self.output.push_back(wire(json));
    }
    pub fn reset(&mut self) {
        self.voice.stop();
        self.queue.clear(); self.queued=0; self.audio_pending.clear(); self.base_pts=None; self.origin_ms=self.last_ms; self.waiting_key=true; self.generation=self.generation.wrapping_add(1);
        self.audio_generation=self.audio_generation.wrapping_add(1);
        self.stop_audio(); self.feedback("{\"type\":\"keyframe\",\"screen\":0}");
    }
    pub fn recover_video(&mut self) {
        // A missed video AU invalidates dependent video only. Keep the audio
        // player alive instead of repeatedly stopping/reinitializing its track.
        self.queue.retain(|f|f.kind!=2);
        self.queued=self.queue.iter().map(|f|f.payload.len()).sum();
        self.waiting_key=true;self.generation=self.generation.wrapping_add(1);
        self.feedback("{\"type\":\"keyframe\",\"screen\":0}");
    }
    fn stop_role(&mut self, role:u8) {
        if self.audio_active[role as usize] {
            self.audio_active[role as usize]=false;
            let (channel,_,_,stop)=self.audio_lane(role);
            self.audio_pending.push_back(packet(channel,stop,self.last_ms,Vec::new()));
        }
        if role==2{self.voice.speech(false);}if role==3{self.voice.call(false);}
    }
    fn audio_lane(&self,role:u8)->(Channel,u32,u32,u32){audio_lane(if role==3&&self.call_media{0}else{role})}
    fn stop_audio(&mut self) { for role in 0..4{self.stop_role(role);} }
    pub fn set_playing(&mut self,value:bool) {
        if value{self.audio_ready=true;}
        if self.playing!=value {
            self.playing=value;
            self.recover_video();
            // Video backgrounding stops ordinary playback, not an ongoing call.
            if !value{self.set_audio_allowed(false);}
        }
    }
    pub fn transport_lost(&mut self) {
        self.playing=false;self.audio_ready=false;
        // Closing the source socket makes its consumer replay microphone demand
        // after a fresh USB transport, instead of retaining an orphaned capture.
        self.disconnect();
    }
    pub fn set_audio_allowed(&mut self,value:bool) { if !value {
        self.queue.retain(|f|f.kind!=3||f.flags>=2);self.queued=self.queue.iter().map(|f|f.payload.len()).sum();
        self.stop_role(0);self.stop_role(1);
    } }
    fn disconnect(&mut self) {
        #[cfg(unix)] { self.peer=None; }
        self.active=false; self.configured=false; self.input.clear(); self.output.clear(); self.output_offset=0;
        self.reset(); self.output.clear();
    }
    fn accept_frame(&mut self,f:Frame)->Result<(),String> {
        if matches!(f.kind,5|7)&&self.voice.congested(){self.disconnect();return Err("voice controls exceeded bounded queue".into());}
        match f.kind {
            1 => { let v:Vec<u32>=f.payload.chunks_exact(4).map(|v|u32::from_le_bytes(v.try_into().unwrap())).collect();
                if v != [CONFIG.width,CONFIG.height,CONFIG.fps] {return Err("live source must match advertised 800x480x30; restart with matching receiver mode".into());}
                if !self.configured {self.reset();self.configured=true;}
            },
            4 => { let active=f.flags==1; if active!=self.active {self.active=active;self.reset();} },
            5 => {
                if self.voice.call&&f.flags==2{return Ok(());}
                if f.flags==3&&f.payload[0]==1&&!self.voice.call{
                    self.audio_generation=self.audio_generation.wrapping_add(1);
                    self.queue.retain(|q|q.kind!=3||q.flags==3);self.queued=self.queue.iter().map(|q|q.payload.len()).sum();self.stop_role(0);self.stop_role(1);self.stop_role(2);
                }
                if f.flags==2{self.voice.speech(f.payload[0]==1);}if f.flags==3{self.voice.call(f.payload[0]==1);}
                if f.payload[0]==0 {
                    if f.flags==3{self.audio_generation=self.audio_generation.wrapping_add(1);}
                    let role=f.flags;
                    self.queue.retain(|q|q.kind!=3 || q.flags!=role);
                    self.queued=self.queue.iter().map(|q|q.payload.len()).sum();
                    let channel=self.audio_lane(role).0;
                    if !self.voice.call||channel!=self.audio_lane(3).0{self.audio_pending.retain(|m|m.channel!=channel);}
                    self.stop_role(role);
                }
            },
            6 => self.recover_video(),
            7 => {
                let v:serde_json::Value=serde_json::from_slice(&f.payload).map_err(|_|"invalid microphone request")?;
                let generation=v["generation"].as_u64().filter(|v|*v>0).ok_or("invalid microphone generation")?;
                let active=v["active"].as_bool().ok_or("invalid microphone state")?;
                if v["sample_rate"]!=16000||v["channels"]!=1{return Err("microphone requires 16000 Hz mono PCM16".into());}
                if !self.active||!self.configured{return Ok(());}
                self.voice.microphone(generation,active);
            },
            _ => {
                if !self.active || !self.configured {return Ok(());}
                if !self.playing && !(f.kind==3&&f.flags>=2&&self.audio_ready){return Ok(());}
                if f.kind==3&&self.voice.call&&f.flags<3{return Ok(());}
                let lane_count=self.queue.iter().filter(|q|q.kind==f.kind && (f.kind==2 || q.flags==f.flags)).count();
                if self.queued+f.payload.len()>MAX_QUEUED || lane_count>=if f.kind==2{4}else{6} {
                    if f.kind==3{return Ok(());}
                    self.recover_video();
                    if self.queued+f.payload.len()>MAX_QUEUED || self.queue.len()>=32{return Ok(());}
                }
                if f.kind==2 && self.waiting_key {if f.flags&1==0{return Ok(());}self.waiting_key=false;}
                self.queued+=f.payload.len();self.queue.push_back(f);
            }
        }
        Ok(())
    }
    pub fn poll_io(&mut self)->Result<(),String> {
        #[cfg(unix)] {
            if self.peer.is_none() { match self.listener.accept() { Ok((peer,_))=>{peer.set_nonblocking(true).map_err(|e|e.to_string())?;self.peer=Some(peer);self.feedback("{\"type\":\"keyframe\",\"screen\":0}");}, Err(e) if e.kind()==io::ErrorKind::WouldBlock=>return Ok(()), Err(e)=>return Err(e.to_string()) } }
            // Budget protects command handling from a continuously writing producer.
            for _ in 0..16 {
                let mut bytes=[0u8;16384];
                match self.peer.as_mut().unwrap().read(&mut bytes) {
                    Ok(0)=>{self.disconnect();return Ok(());},
                    Ok(n)=>{self.input.extend_from_slice(&bytes[..n]);if self.input.len()>MAX_FRAME+4 {self.disconnect();return Err("live receive buffer exceeded".into());}},
                    Err(e) if e.kind()==io::ErrorKind::WouldBlock=>break,
                    Err(e) if e.kind()==io::ErrorKind::Interrupted=>continue,
                    Err(_)=>{self.disconnect();return Ok(());}
                }
                while self.input.len()>=4 {
                    let n=u32::from_le_bytes(self.input[..4].try_into().unwrap()) as usize;
                    if !(12..=MAX_FRAME).contains(&n){self.disconnect();return Err("invalid live frame length".into());}
                    if self.input.len()<n+4{break;}
                    let frame=decode(&self.input[4..n+4])?;self.input.drain(..n+4);self.accept_frame(frame)?;
                }
            }
            for _ in 0..16 {
                let Some(data)=self.output.front() else{break;};
                match self.peer.as_mut().unwrap().write(&data[self.output_offset..]) {
                    Ok(0)=>{self.disconnect();break;}, Ok(n)=>{self.output_offset+=n;if self.output_offset==data.len(){self.output.pop_front();self.output_offset=0;}},
                    Err(e) if e.kind()==io::ErrorKind::WouldBlock=>break,
                    Err(e) if e.kind()==io::ErrorKind::Interrupted=>continue,
                    Err(_)=>{self.disconnect();break;}
                }
            }
        }
        Ok(())
    }
    pub fn next(&mut self,audio_allowed:bool)->Option<Message> {
        if self.waiting_key && self.active && self.playing {self.feedback("{\"type\":\"keyframe\",\"screen\":0}");}
        if self.report_at.elapsed()>=Duration::from_secs(5){
            crate::runtime::log("live_latency",&format!("pending={} bytes={} age_max_ms={} video_dropped={}",self.queue.len(),self.queued,self.age_max_us/1000,self.video_dropped));
            println!("{}",serde_json::json!({"event":"voice_stats","microphone_active":self.voice.capture.is_some(),"microphone_packets":self.voice.packets,"microphone_bytes":self.voice.bytes,"call_active":self.voice.call}));
            self.report_at=Instant::now();self.age_max_us=0;
        }
        if let Some(message)=self.voice.pop(){return Some(message);}
        if let Some(message)=self.audio_pending.pop_front(){return Some(message);}
        let f=self.queue.pop_front()?;self.queued-=f.payload.len();
        let age=if f.kind==2 && f.flags&2!=0 {monotonic_us().map(|now|now.saturating_sub(f.pts)).unwrap_or(0)}else{f.arrived.elapsed().as_micros() as u64};
        self.age_max_us=self.age_max_us.max(age);
        if age>120_000 {
            if f.kind==2{self.video_dropped+=1;self.recover_video();}
            return None;
        }
        let base=*self.base_pts.get_or_insert(f.pts);
        let ms=(f.pts.saturating_sub(base)/1000).min(u32::MAX as u64) as u32;
        let lane=if f.kind==2{0}else{match self.audio_lane(f.flags).0{Channel::Media=>1,Channel::Tts=>2,_=>3}};
        let timestamp=self.stamps[lane].max(self.origin_ms.saturating_add(ms));
        self.stamps[lane]=timestamp;self.last_ms=self.last_ms.max(timestamp);
        if f.kind==2{return Some(packet(Channel::Video,0x20001,timestamp,f.payload));}
        if !audio_allowed&&f.flags<2{return None;}
        let (channel,init_id,data_id,_)=self.audio_lane(f.flags);
        let data=packet(channel,data_id,timestamp,f.payload);
        if !self.audio_active[f.flags as usize] {
            self.audio_active[f.flags as usize]=true;
            if f.flags==3{self.stop_role(0);self.voice.call(true);}
            if f.flags==2{self.voice.speech(true);}
            let init=packet(channel,init_id,timestamp,vec![8,0xc4,0xd8,2,16,2,24,16]);
            self.audio_pending.push_back(init);self.audio_pending.push_back(data);return self.audio_pending.pop_front();
        }
        Some(data)
    }
}
fn audio_lane(role:u8)->(Channel,u32,u32,u32){match role {
    0=>(Channel::Media,0x30001,0x30006,0x30002),
    1=>(Channel::Tts,0x40001,0x40003,0x40002),
    _=>(Channel::Vr,0x50002,0x50003,0x50004),
}}
fn packet(channel:Channel,id:u32,ms:u32,payload:Vec<u8>)->Message{Message{channel,id,timestamp_ms:Some(ms),reserved:0,payload}}
impl Drop for LiveSource { fn drop(&mut self){ #[cfg(unix)] if std::fs::symlink_metadata(&self.path).map(|m|m.file_type().is_socket()).unwrap_or(false){let _=std::fs::remove_file(&self.path);} } }

/// Official CarLife single-point protobuf facts: action 0x68001 has
/// action/x/y fields 1/2/3, separate down/up/move 0x68002..4 use x/y 1/2.
pub fn touch(message:&Message,config:VideoConfig)->Result<Option<String>,String>{
    if message.channel!=Channel::Touch || !(0x68001..=0x68004).contains(&message.id){return Ok(None);}
    let mut values=[None;4];let mut pos=0;
    while pos<message.payload.len(){
        let tag=varint(&message.payload,&mut pos)?;
        let field=tag>>3;
        if field==0 || field>0x1fffffff{return Err("invalid touch field".into());}
        if field<values.len() as u64 {
            if tag&7!=0{return Err("touch requires integer fields".into());}
            // Protobuf singular fields use the last value. Newer head units
            // append metadata fields that do not affect the contact position.
            values[field as usize]=Some(varint(&message.payload,&mut pos)?);
        }else{
            let size=match tag&7 {
                0=>{varint(&message.payload,&mut pos)?;0},
                1=>8,
                2=>usize::try_from(varint(&message.payload,&mut pos)?).map_err(|_|"touch extension too large")?,
                5=>4,
                _=>return Err("unsupported touch extension wire type".into()),
            };
            pos=pos.checked_add(size).filter(|end|*end<=message.payload.len()).ok_or("truncated touch extension")?;
        }
    }
    let (down,x,y)=if message.id==0x68001{let action=values[1].ok_or("touch action missing")?;if action>2{return Ok(None);} (action!=1,values[2],values[3])}else{(message.id!=0x68003,values[1],values[2])};
    let (x,y)=(x.ok_or("touch x missing")?,y.ok_or("touch y missing")?);
    if x>=config.width as u64 || y>=config.height as u64{return Err("touch outside negotiated display".into());}
    Ok(Some(format!("{{\"type\":\"touch\",\"contacts\":[{{\"id\":0,\"x\":{},\"y\":{},\"down\":{}}}]}}",x as f64/config.width as f64,y as f64/config.height as f64,down)))
}
fn varint(bytes:&[u8],pos:&mut usize)->Result<u64,String>{let mut out=0u64;for shift in (0..70).step_by(7){let b=*bytes.get(*pos).ok_or("truncated touch varint")?;*pos+=1;if shift==63 && b>1{return Err("touch varint overflow".into());}out|=((b&127)as u64)<<shift;if b&128==0{return Ok(out);}}Err("touch varint overflow".into())}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn protocol_rejects_bad_frames(){assert!(decode(&[0;11]).is_err());let mut b=vec![3,0,0,0];b.extend(0u64.to_le_bytes());b.extend([1,2,3]);assert!(decode(&b).is_err());}
 #[test]fn touch_normalizes_negotiated_dimensions(){let m=Message::command(0x68002,vec![8,0x90,3,16,0xf0,1]).unwrap();let s=touch(&m,CONFIG).unwrap().unwrap();assert!(s.contains("\"x\":0.5"));assert!(s.contains("\"y\":0.5"));assert!(s.contains("\"down\":true"));}
 #[test]fn touch_accepts_protobuf_extensions_and_last_singular_value(){
  let m=Message::command(0x68001,vec![8,0,16,1,16,0x90,3,24,0xf0,1,32,123,42,2,9,8,53,0,0,0,0,57,0,0,0,0,0,0,0,0]).unwrap();
  let s=touch(&m,CONFIG).unwrap().unwrap();assert!(s.contains("\"x\":0.5"));assert!(s.contains("\"y\":0.5"));
  for payload in [vec![8,0,16,1,24,1,42,3,1],vec![8,0,16,1,24,1,32,128],vec![8,0,16,1,24,1,0]]{
   assert!(touch(&Message::command(0x68001,payload).unwrap(),CONFIG).is_err());
  }
 }
 #[cfg(unix)]fn source()->LiveSource{let p=std::env::temp_dir().join(format!("carlink-live-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));let mut s=LiveSource::bind(&p).unwrap();s.active=true;s.configured=true;s.playing=true;s.audio_ready=true;s}
 #[test]#[cfg(unix)]fn video_pause_and_recovery_preserve_duplex_call(){
  let mut s=source();s.call_media=false;
  s.voice.microphone(91,true);s.voice.call(true);while s.voice.pop().is_some(){}
  s.accept_frame(Frame{kind:3,flags:3,pts:1000,payload:vec![1;4],arrived:Instant::now()}).unwrap();
  assert_eq!(s.next(true).unwrap().id,0x50002);assert_eq!(s.next(true).unwrap().id,0x50003);
  let epoch=s.generation()[1];
  for playing in [false,true,false] {
   s.set_playing(playing);s.recover_video();
   assert!(s.voice.call);assert_eq!(s.voice.capture,Some(91));assert_eq!(s.generation()[1],epoch);
   s.accept_frame(Frame{kind:3,flags:3,pts:2000,payload:vec![2;4],arrived:Instant::now()}).unwrap();
   assert_eq!(s.next(false).unwrap().id,0x50003);
   let microphone=Message{channel:Channel::Vr,id:0x58001,timestamp_ms:Some(2),reserved:0,payload:vec![3;640]};
   s.microphone(&microphone);
   assert_eq!(s.output.back().unwrap()[4],130);
   assert!(s.voice.pop().is_none());
  }
  s.transport_lost();assert!(!s.audio_ready);assert!(!s.voice.call);assert_eq!(s.voice.capture,None);
  assert!(!s.active);assert!(!s.configured);assert_eq!(s.next(false).unwrap().id,0x10023);
 }
 #[test]#[cfg(unix)]fn call_pcm_waits_for_initial_video_start_and_source_session(){
  let mut s=source();s.playing=false;s.audio_ready=false;s.voice.microphone(2,true);while s.voice.pop().is_some(){}
  s.accept_frame(Frame{kind:3,flags:3,pts:0,payload:vec![1;4],arrived:Instant::now()}).unwrap();assert!(s.queue.is_empty());
  let pcm=Message{channel:Channel::Vr,id:0x58001,timestamp_ms:Some(0),reserved:0,payload:vec![0;640]};
  s.output.clear();s.microphone(&pcm);assert!(s.output.is_empty());
  s.set_playing(true);s.microphone(&pcm);assert_eq!(s.output.back().unwrap()[4],130);
  s.accept_frame(Frame{kind:4,flags:0,pts:0,payload:vec![],arrived:Instant::now()}).unwrap();
  s.output.clear();s.microphone(&pcm);assert!(s.output.is_empty());assert_eq!(s.voice.capture,None);
 }
 #[test]#[cfg(unix)]fn microphone_lifecycle_and_call_audio_ignore_music_mute(){
  let mut s=source();let request=serde_json::json!({"generation":51,"active":true,"sample_rate":16000,"channels":1});
  s.accept_frame(Frame{kind:7,flags:0,pts:0,payload:request.to_string().into_bytes(),arrived:Instant::now()}).unwrap();
  assert_eq!(s.next(true).unwrap().id,0x10026);assert_eq!(s.next(true).unwrap().id,0x10024);
  s.accept_frame(Frame{kind:5,flags:3,pts:0,payload:vec![1],arrived:Instant::now()}).unwrap();
  s.accept_frame(Frame{kind:3,flags:3,pts:0,payload:vec![1;3528],arrived:Instant::now()}).unwrap();
  s.set_audio_allowed(false);assert_eq!(s.next(false).unwrap().id,0x50002);assert_eq!(s.next(false).unwrap().id,0x50003);
  s.disconnect();assert_eq!(s.voice.capture,None);assert_eq!(s.next(false).unwrap().id,0x10023);
 }
 #[test]#[cfg(unix)]fn pause_overload_and_reconnect_wait_for_keyframe(){let mut s=source();let frame=|key|Frame{arrived:Instant::now(),kind:2,flags:key,pts:1000,payload:vec![0,0,0,1,0x65]};s.accept_frame(frame(0)).unwrap();assert!(s.next(true).is_none());s.accept_frame(frame(1)).unwrap();assert!(s.next(true).is_some());s.set_playing(false);s.accept_frame(frame(1)).unwrap();assert!(s.queue.is_empty());s.set_playing(true);s.accept_frame(frame(0)).unwrap();assert!(s.queue.is_empty());s.queued=MAX_QUEUED;s.accept_frame(frame(0)).unwrap();assert!(s.waiting_key);s.disconnect();assert!(!s.configured);assert!(!s.active);}
 #[test]#[cfg(unix)]fn monotonic_timestamp_and_audio_init(){let mut s=source();s.waiting_key=false;for pts in [100000,90000]{s.accept_frame(Frame{arrived:Instant::now(),kind:3,flags:0,pts,payload:vec![0;4]}).unwrap();}assert_eq!(s.next(true).unwrap().id,0x30001);assert_eq!(s.next(true).unwrap().id,0x30006);assert_eq!(s.next(true).unwrap().timestamp_ms,Some(0));}
 #[test]#[cfg(unix)]fn call_switch_fences_queued_audio_and_preserves_physical_clock(){
  for media in [false,true]{
  let mut s=source();s.call_media=media;let role=if media{0}else{2};let data=if media{0x30006}else{0x50003};
  s.accept_frame(Frame{kind:3,flags:role,pts:1000000,payload:vec![0;4],arrived:Instant::now()}).unwrap();
  for _ in 0..8{if s.next(true).is_some_and(|m|m.id==data){break;}}
  s.accept_frame(Frame{kind:3,flags:role,pts:2000000,payload:vec![0;4],arrived:Instant::now()}).unwrap();
  assert_eq!(s.next(true).unwrap().timestamp_ms,Some(1000));
  let epoch=s.generation()[1];
  s.accept_frame(Frame{kind:5,flags:3,pts:0,payload:vec![1],arrived:Instant::now()}).unwrap();
  assert_ne!(s.generation()[1],epoch);
  s.accept_frame(Frame{kind:3,flags:3,pts:1100000,payload:vec![0;4],arrived:Instant::now()}).unwrap();
  let mut call_ms=None;
  for _ in 0..8 {if let Some(m)=s.next(true){if m.id==data{call_ms=m.timestamp_ms;break;}}}
  assert_eq!(call_ms,Some(1000));
  let epoch=s.generation()[1];
  s.accept_frame(Frame{kind:5,flags:3,pts:0,payload:vec![0],arrived:Instant::now()}).unwrap();
  assert_ne!(s.generation()[1],epoch);
  }
 }
 #[test]#[cfg(unix)]fn audio_clock_is_not_clamped_to_video_arrival() {
  let mut s=source();s.waiting_key=false;
  for pts in [1000000,2000000] {s.accept_frame(Frame{arrived:Instant::now(),kind:2,flags:1,pts,payload:vec![0,0,0,1,0x65]}).unwrap();}
  s.accept_frame(Frame{arrived:Instant::now(),kind:3,flags:0,pts:1500000,payload:vec![0;4]}).unwrap();
  assert_eq!(s.next(true).unwrap().timestamp_ms,Some(0));
  assert_eq!(s.next(true).unwrap().timestamp_ms,Some(1000));
  assert_eq!(s.next(true).unwrap().timestamp_ms,Some(500));
 }
 #[test]#[cfg(unix)]fn video_recovery_and_audio_overflow_keep_audio_track_alive(){
  let mut s=source();s.waiting_key=false;s.audio_active[0]=true;
  s.recover_video();assert!(s.audio_active[0]);assert!(s.audio_pending.is_empty());
  // Audio must continue while video is waiting for its recovery keyframe.
  assert!(s.waiting_key);
  for _ in 0..6{s.accept_frame(Frame{arrived:Instant::now(),kind:3,flags:0,pts:1000,payload:vec![0;4]}).unwrap();}
  s.accept_frame(Frame{arrived:Instant::now(),kind:3,flags:0,pts:2000,payload:vec![0;4]}).unwrap();
  assert!(s.audio_active[0]);assert!(s.audio_pending.is_empty());assert_eq!(s.queue.len(),6);
  assert_eq!(s.next(true).unwrap().id,0x30006);assert!(s.waiting_key);
 }
 #[test]#[cfg(unix)]fn stale_video_is_not_replayed_and_audio_epoch_survives(){
  let mut s=source();s.waiting_key=false;s.audio_active[0]=true;
  let audio_epoch=s.generation()[1];
  s.accept_frame(Frame{arrived:Instant::now()-Duration::from_millis(200),kind:2,flags:1,pts:1000,payload:vec![0,0,0,1,0x65]}).unwrap();
  assert!(s.next(true).is_none());assert!(s.waiting_key);assert!(s.audio_active[0]);assert_eq!(s.generation()[1],audio_epoch);
  if let Some(now)=monotonic_us(){
   s.accept_frame(Frame{arrived:Instant::now(),kind:2,flags:3,pts:now.saturating_sub(200_000),payload:vec![0,0,0,1,0x65]}).unwrap();
   assert!(s.next(true).is_none());assert_eq!(s.video_dropped,2);
  }
  assert!(s.audio_pending.is_empty());
 }

}
