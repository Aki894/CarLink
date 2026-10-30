//! Rust OXP output adapter. Codec computation stays in the pinned native tools.
mod wire;
mod video;
mod link;
mod audio;
mod microphone;
mod opus;

use serde_json::{json,Value};
use std::{collections::{BTreeMap,VecDeque},fs,io::{Read,Write,ErrorKind},os::{fd::AsRawFd,unix::{fs::{DirBuilderExt,MetadataExt,OpenOptionsExt,PermissionsExt},net::{UnixListener,UnixStream}}},path::{Path,PathBuf},sync::atomic::{AtomicBool,Ordering},time::{Duration,Instant}};
type Result<T> = std::result::Result<T,String>;
static STOP:AtomicBool=AtomicBool::new(false);
extern "C" fn stop(_:i32){STOP.store(true,Ordering::Relaxed);}
fn now_us()->u64{let mut t=libc::timespec{tv_sec:0,tv_nsec:0};unsafe{libc::clock_gettime(libc::CLOCK_MONOTONIC,&mut t);}t.tv_sec as u64*1_000_000+t.tv_nsec as u64/1000}
fn diagnostic(event:&str,stream:u8,format:&Value){
    let role=format["role"].as_str().filter(|v|matches!(*v,"MEDIA"|"ALERT"|"VOICE"|"TELEPHONY")).unwrap_or("OTHER");
    let codec=format["codec"].as_str().filter(|v|matches!(*v,"PCM_S16LE"|"AAC_LC"|"AAC_ADTS"|"OPUS")).unwrap_or("OTHER");
    let mut record=json!({"event":event,"stream":stream,"role":role,"codec":codec,"rate":format["sampleRate"].as_u64(),"channels":format["channels"].as_u64()});
    if event=="audio_decoder_failed"{record["failure"]=format["failure"].clone();}
    let line=record.to_string()+"\n";
    eprint!("{line}");
    let path=Path::new("/run/carlink-carplay/audio-diagnostics.jsonl");let truncate=path.symlink_metadata().is_ok_and(|m|m.len()>65536);
    if let Ok(mut file)=fs::OpenOptions::new().write(true).create(true).append(!truncate).truncate(truncate).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(path){let _=file.write_all(line.as_bytes());}
}
struct Config{socket:PathBuf,live:PathBuf,audio:PathBuf,opus:Option<PathBuf>,width:u32,height:u32,source_clock:bool}
impl Config{
    fn parse(args:&[String])->Result<Self>{
        let root=std::env::var_os("CARLINK_CARPLAY_ROOT").map(PathBuf::from).unwrap_or_else(||"/usr/local/lib/carlink/carplay".into());
        let mut c=Self{socket:"/run/carlink-carplay/media.sock".into(),live:std::env::var_os("CARLINK_LIVE_SOCKET").map(PathBuf::from).unwrap_or_else(||"/run/carlink/live.sock".into()),audio:root.join("bin/carlink-audio"),opus:None,width:800,height:480,source_clock:std::env::var("CARLINK_LOW_LATENCY").as_deref()==Ok("1")};
        let mut i=0;while i<args.len(){let name=args[i].as_str();i+=1;
            if !matches!(name,"--unix"|"--live-socket"|"--audio-binary"|"--opus-library"|"--width"|"--height"){continue;}
            let value=args.get(i).ok_or("output option needs a value")?;i+=1;
            match name{"--unix"=>c.socket=value.into(),"--live-socket"=>c.live=value.into(),"--audio-binary"=>c.audio=value.into(),"--opus-library"=>c.opus=Some(value.into()),"--width"=>c.width=value.parse().map_err(|_|"invalid output width")?,"--height"=>c.height=value.parse().map_err(|_|"invalid output height")?,_=>unreachable!()}
        }
        if !c.socket.is_absolute()||!c.live.is_absolute()||!c.audio.is_absolute()||c.socket==c.live{return Err("output paths must be distinct absolute paths".into());}
        if !(16..=1920).contains(&c.width)||!(16..=1080).contains(&c.height)||c.width%2!=0||c.height%2!=0{return Err("unsupported output dimensions".into());}
        Ok(c)
    }
}
struct SocketPath{path:PathBuf,dev:u64,ino:u64}
impl SocketPath{
    fn owned(path:&Path)->Result<Self>{let m=fs::symlink_metadata(path).map_err(|_|"cannot identify output socket")?;Ok(Self{path:path.into(),dev:m.dev(),ino:m.ino()})}
}
impl Drop for SocketPath{fn drop(&mut self){if fs::symlink_metadata(&self.path).is_ok_and(|m|m.dev()==self.dev&&m.ino()==self.ino){let _=fs::remove_file(&self.path);}}}
struct Outgoing{bytes:Vec<u8>,offset:usize,queued:Instant}
struct Output{queue:VecDeque<Outgoing>,bytes:usize}
impl Output{
    fn new()->Self{Self{queue:VecDeque::new(),bytes:0}}
    fn push(&mut self,bytes:Vec<u8>)->Result<()>{
        if self.bytes+bytes.len()>1024*1024||self.queue.len()>=128{return Err("OXP control output exceeded bound".into());}
        self.bytes+=bytes.len();self.queue.push_back(Outgoing{bytes,offset:0,queued:Instant::now()});Ok(())
    }
    fn clear_microphone(&mut self){self.queue.retain(|p|p.offset>0||p.bytes.get(4)!=Some(&3));self.bytes=self.queue.iter().map(|p|p.bytes.len()).sum();}
    fn pump(&mut self,socket:&mut UnixStream)->Result<()>{for _ in 0..16{
        let Some(front)=self.queue.front_mut()else{break;};
        if front.queued.elapsed()>Duration::from_secs(2){return Err("OXP control output stalled".into());}
        match socket.write(&front.bytes[front.offset..]){
            Ok(0)=>return Err("OXP control output closed".into()),Ok(n)=>{front.offset+=n;if front.offset==front.bytes.len(){self.bytes-=front.bytes.len();self.queue.pop_front();}},
            Err(e)if e.kind()==ErrorKind::WouldBlock=>break,Err(e)if e.kind()==ErrorKind::Interrupted=>continue,Err(_)=>return Err("OXP control output failed".into()),
        }
    }Ok(())}
}
struct Adapter{
    config:Config,link:link::Link,microphone:microphone::Microphone,formats:BTreeMap<u8,audio::Format>,decoders:BTreeMap<u8,audio::Decoder>,retry:BTreeMap<u8,Instant>,
    active:bool,hello:bool,parameters:Vec<u8>,video_format:Value,last_video:Option<u32>,capture_offset:Option<i64>,output:Output,
    report_at:Instant,source_frames:u64,source_bytes:u64,source_dropped:u64,source_age_max:f64,
}
impl Adapter{
    fn new(config:Config)->Self{Self{link:link::Link::new(config.live.clone()),microphone:microphone::Microphone::new(config.opus.clone()),config,formats:BTreeMap::new(),decoders:BTreeMap::new(),retry:BTreeMap::new(),active:false,hello:false,parameters:vec![],video_format:Value::Null,last_video:None,capture_offset:None,output:Output::new(),report_at:Instant::now(),source_frames:0,source_bytes:0,source_dropped:0,source_age_max:0.}}
    fn request_microphone(&mut self,values:Vec<Value>)->Result<()>{for value in values{self.link.microphone(&value)?;}Ok(())}
    fn decoder_failed(&self,stream:u8){
        if let (Some(decoder),Some(format))=(self.decoders.get(&stream),self.formats.get(&stream)){
            let role_name=["MEDIA","ALERT","VOICE","TELEPHONY"][format.role as usize];
            diagnostic("audio_decoder_failed",stream,&json!({"role":role_name,
                "codec":format.codec,"sampleRate":format.sample_rate,"channels":format.channels,"failure":decoder.failure()}));
        }
    }

    fn json(&mut self,value:Value)->Result<()>{
        if !self.hello{
            if value["type"]!="hello"||value["version"].as_u64()!=Some(0)||value["role"]!="producer"{return Err("expected OXP v0 producer hello".into());}
            self.hello=true;
            self.output.push(wire::json(&json!({"type":"hello","version":0,"role":"consumer","name":"CarLink live","client":"carlink-live-rust;carlink-host-ui=1"}))?)?;
            self.output.push(wire::json(&json!({"type":"capabilities","latency":"INTERACTIVE","decoders":[{"codecString":"avc1.64001F","hardware":false}],"displays":{"0":{"widthPx":self.config.width,"heightPx":self.config.height,"densityDpi":160,"refreshHz":30,"frameRate":"FPS_30","safeArea":{"left":0,"top":0,"right":0,"bottom":0}}},"touch":10,"keys":(0..11).collect::<Vec<u8>>(),"audio":[],"microphone":true,"prefersH264":true}))?)?;
            return Ok(());
        }
        match value["type"].as_str(){
            Some("host-ui-request") if self.active && value.as_object().is_some_and(|o|o.len()==1)=>{
                self.link.open_menu();
            },
            Some("stats")if self.config.source_clock&&!value["clockOffsetUs"].is_null()=>{
                self.capture_offset=Some(value["clockOffsetUs"].as_str().ok_or("invalid source clock mapping")?.parse().map_err(|_|"invalid source clock mapping")?);
            },
            Some("session")=>{
                let active=value["session"]["phase"]=="ACTIVE";
                if self.active!=active{
                    let stopped=self.microphone.stop(None);self.request_microphone(stopped)?;
                    self.decoders.clear();self.formats.clear();self.retry.clear();self.active=active;self.link.session(active);
                    if active{self.output.push(wire::control(&json!({"type":"keyframe","screen":0}))?)?;}
                    else{self.parameters.clear();self.video_format=Value::Null;self.last_video=None;}
                }
            },
            Some("mic")if self.active=>{
                let stream=value["stream"].as_u64().filter(|s|*s<=255).ok_or("invalid microphone stream")? as u8;
                let request=if value["request"].is_null(){None}else{Some(&value["request"])};
                diagnostic(if request.is_some(){"microphone_request"}else{"microphone_stopped"},stream,&value["request"]);
                let demands=self.microphone.configure(stream,request)?;self.request_microphone(demands)?;
            },
            Some("video-format")if value["screen"].as_u64()==Some(0)=>{
                let format=&value["format"];
                if *format==self.video_format{return Ok(());}
                if format.is_null(){self.parameters.clear();self.video_format=Value::Null;return Ok(());}
                if format["codec"]!="H264"{return Err("output requires H264".into());}
                let width=format["width"].as_u64().and_then(|n|u32::try_from(n).ok()).ok_or("invalid video width")?;
                let height=format["height"].as_u64().and_then(|n|u32::try_from(n).ok()).ok_or("invalid video height")?;
                self.parameters=video::parameters(format["config"].as_str().ok_or("invalid AVC configuration")?)?;
                self.link.configure(width,height,30)?;self.video_format=format.clone();
                self.output.push(wire::control(&json!({"type":"keyframe","screen":0}))?)?;
            },
            Some("audio-format")=>{
                let stream=value["stream"].as_u64().filter(|s|*s<=255).ok_or("invalid audio stream")? as u8;
                diagnostic(if value["format"].is_null(){"audio_stopped"}else{"audio_format"},stream,&value["format"]);
                self.decoders.remove(&stream);self.retry.remove(&stream);
                if let Some(old)=self.formats.remove(&stream){self.link.audio_state(old.role,false);}
                if !value["format"].is_null(){if let Ok(format)=audio::Format::parse(&value["format"]){
                    if self.formats.len()>=8{return Err("too many audio streams".into());}
                    if format.role==3{self.link.audio_state(3,true);}self.formats.insert(stream,format);
                }}
            },
            _=>{},
        }Ok(())
    }
    fn binary(&mut self,message:wire::Binary)->Result<()>{
        if !self.hello{return Err("binary frame before producer hello".into());}
        if !self.active{return Ok(());}
        if message.kind==1&&message.stream==0&&!self.parameters.is_empty(){
            self.source_frames+=1;self.source_bytes+=message.payload.len() as u64;
            if self.last_video.is_some_and(|last|message.sequence!=last.wrapping_add(1))&&message.flags&1==0{self.link.recover_video();}
            self.last_video=Some(message.sequence);
            if self.config.source_clock{
                let age=now_us() as i128-message.sender as i128;self.source_age_max=self.source_age_max.max(age as f64/1000.);
                if !(-50_000..=120_000).contains(&age){self.source_dropped+=1;self.link.recover_video();return Ok(());}
                if self.capture_offset.is_none(){self.capture_offset=message.sender.checked_sub(message.pts);}
            }
            if message.payload.len()>512*1024{self.link.recover_video();return Ok(());}
            let data=video::annexb(&message.payload)?;
            let data=if message.flags&1!=0{[self.parameters.as_slice(),data.as_slice()].concat()}else{data};
            if data.len()>512*1024{self.link.recover_video();return Ok(());}
            let pts=if self.config.source_clock{message.sender}else{message.pts};
            self.link.media(2,data,pts.max(0) as u64,(message.flags&1)|if self.config.source_clock{2}else{0});
        }else if message.kind==2{
            // A single OXP read can contain many compressed packets. Advance
            // nonblocking codec pipes between packets instead of filling the
            // Rust queue before the owner gets its next regular step.
            self.poll_audio()?;
            let Some(format)=self.formats.get(&message.stream).cloned()else{return Ok(());};
            if self.decoders.get(&message.stream).is_some_and(|d|d.failed()){
                self.decoder_failed(message.stream);
                self.decoders.remove(&message.stream);self.retry.insert(message.stream,Instant::now()+Duration::from_millis(500));
            }
            if !self.decoders.contains_key(&message.stream){
                if self.retry.get(&message.stream).is_some_and(|t|Instant::now()<*t){return Ok(());}
                let decoder=audio::Decoder::new(&self.config.audio,&format)?;
                let role_name=["MEDIA","ALERT","VOICE","TELEPHONY"][format.role as usize];
                diagnostic("audio_decoder_started",message.stream,&json!({"role":role_name,"codec":format.codec,"sampleRate":format.sample_rate,"channels":format.channels}));
                self.decoders.insert(message.stream,decoder);self.link.audio_state(format.role,true);
            }
            let pts=if self.config.source_clock{self.capture_offset.and_then(|offset|message.pts.checked_add(offset)).unwrap_or(message.sender)}else{message.pts};
            self.decoders.get_mut(&message.stream).unwrap().feed(&message.payload,pts.max(0) as u64)?;
        }Ok(())
    }
    fn poll_audio(&mut self)->Result<()>{
        let mut failed=vec![];
        for (stream,decoder)in &mut self.decoders{
            let role=self.formats.get(stream).ok_or("audio format disappeared")?.role;
            for (pcm,pts)in decoder.poll()?{self.link.media(3,pcm,pts,role);}
            if decoder.failed(){failed.push((*stream,role));}
        }
        for (stream,role)in failed{
            self.decoder_failed(stream);
            self.decoders.remove(&stream);self.retry.insert(stream,Instant::now()+Duration::from_millis(500));
            if !self.decoders.keys().any(|id|self.formats.get(id).is_some_and(|f|f.role==role)){
                self.link.audio_state(role,false);
                // The OXP format, rather than missing media, owns call lifetime.
                // Reset a failed track while retaining the call/switch guard.
                if role==3&&self.formats.values().any(|f|f.role==3){self.link.audio_state(3,true);}
            }
        }
        Ok(())
    }
    fn step(&mut self)->Result<()>{
        self.poll_audio()?;
        let retry=self.microphone.retry();if !retry.is_empty(){println!("{}",json!({"event":"microphone_recovered"}));self.request_microphone(retry)?;}
        for feedback in self.link.step(){match feedback{
            link::Feedback::Control(value)=>{
                // The source remains active behind the local menu. Returning is
                // a source-mux operation; the existing keyframe request resumes
                // display without a speculative AirPlay mode/audio reset.
                if value["type"] != "return-projection" {
                    if let Ok(bytes)=wire::control(&value){self.output.push(bytes)?;}
                }
            },
            link::Feedback::Microphone{generation,pcm}=>{
                let result=self.microphone.feed(generation,&pcm).and_then(|packets|{for bytes in packets{self.output.push(bytes)?;}Ok(())});
                if result.is_err(){self.output.clear_microphone();let stopped=self.microphone.fault();self.request_microphone(stopped)?;println!("{}",json!({"event":"microphone_error","capture_stopped":true,"retry_scheduled":true}));}
            },
        }}
        if self.report_at.elapsed()>=Duration::from_secs(5){
            let elapsed=self.report_at.elapsed().as_secs_f64().max(0.001);let mut report=self.link.report();
            report["source_video_ms_max"]=self.source_age_max.into();report["source_video_dropped"]=self.source_dropped.into();
            report["source_video_fps"]=(self.source_frames as f64/elapsed).into();report["source_video_mbps"]=(self.source_bytes as f64*8./elapsed/1_000_000.).into();
            println!("{report}");self.source_frames=0;self.source_bytes=0;self.source_age_max=0.;self.report_at=Instant::now();
        }Ok(())
    }
}
impl Drop for Adapter{fn drop(&mut self){let stopped=self.microphone.stop(None);let _=self.request_microphone(stopped);self.decoders.clear();self.link.session(false);self.link.close();}}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn native_audio_ipc_burst_advances_decoder_between_packets(){
        let Some(binary)=std::env::var_os("CARLINK_AUDIO_TEST_BINARY") else{return};
        let packets=audio::test_aac_packets(Path::new(&binary));
        let config=Config{socket:"/nonexistent/oxp".into(),live:"/nonexistent/live".into(),audio:binary.into(),opus:None,width:800,height:480,source_clock:false};
        let mut adapter=Adapter::new(config);adapter.hello=true;adapter.active=true;
        adapter.formats.insert(0,audio::Format{role:0,codec:"AAC_LC".into(),sample_rate:48000,channels:2});
        // 384 ms fits the total latency budget, but exceeds the 16-packet
        // input queue if an IPC batch is fed without polling its codec pipes.
        for sequence in 0..18{
            adapter.binary(wire::Binary{kind:2,stream:0,flags:0,sequence,pts:sequence as i64*1024*1_000_000/48000,sender:0,payload:packets[sequence as usize%packets.len()].clone()}).unwrap();
        }
        assert!(adapter.retry.is_empty(),"a legal IPC burst restarted the codec");
        let decoder=adapter.decoders.get(&0).unwrap();
        assert!(!decoder.failed(),"{}",decoder.failure());
        assert_eq!(decoder.failure()["packets"],18);
    }
}

pub fn run(args:&[String])->Result<()>{
    if args.iter().any(|v|v=="--help"||v=="-h"){println!("Usage: carlink carplay-output [--unix PATH] [--live-socket PATH] [--audio-binary PATH] [--opus-library PATH] [--width 800] [--height 480]");return Ok(());}
    let config=Config::parse(args)?;STOP.store(false,Ordering::Relaxed);
    unsafe{libc::signal(libc::SIGTERM,stop as *const () as libc::sighandler_t);libc::signal(libc::SIGINT,stop as *const () as libc::sighandler_t);}
    let parent=config.socket.parent().ok_or("invalid OXP socket parent")?;fs::DirBuilder::new().recursive(true).mode(0o700).create(parent).map_err(|_|"cannot create OXP socket directory")?;
    let listener=UnixListener::bind(&config.socket).map_err(|_|"cannot bind OXP output socket")?;
    let _path=SocketPath::owned(&config.socket)?;fs::set_permissions(&config.socket,fs::Permissions::from_mode(0o600)).map_err(|_|"cannot secure OXP output socket")?;
    listener.set_nonblocking(true).map_err(|_|"cannot configure OXP listener")?;
    let mut stream=loop{
        if STOP.load(Ordering::Relaxed){return Ok(());}
        match listener.accept(){Ok((s,_))=>break s,Err(e)if e.kind()==ErrorKind::WouldBlock=>std::thread::sleep(Duration::from_millis(5)),Err(e)if e.kind()==ErrorKind::Interrupted=>continue,Err(_)=>return Err("OXP accept failed".into())}
    };
    stream.set_nonblocking(true).map_err(|_|"cannot configure OXP socket")?;
    let mut adapter=Adapter::new(config);let mut decoder=wire::Decoder::new();let started=Instant::now();let mut partial:Option<Instant>=None;
    while !STOP.load(Ordering::Relaxed){
        for _ in 0..16{
            let mut bytes=[0u8;16384];match stream.read(&mut bytes){
                Ok(0)=>{decoder.finish()?;return Err("OXP producer disconnected".into());},
                Ok(n)=>{
                    let messages=decoder.feed(&bytes[..n])?;let complete=!messages.is_empty();
                    if decoder.finish().is_err(){if partial.is_none()||complete{partial=Some(Instant::now());}}else{partial=None;}
                    for message in messages{match message{wire::Message::Json(v)=>adapter.json(v)?,wire::Message::Binary(v)=>adapter.binary(v)?,}}
                },
                Err(e)if e.kind()==ErrorKind::WouldBlock=>break,Err(e)if e.kind()==ErrorKind::Interrupted=>continue,Err(_)=>return Err("OXP input failed".into()),
            }
        }
        if !adapter.hello&&started.elapsed()>Duration::from_secs(5)||partial.is_some_and(|t|t.elapsed()>Duration::from_secs(5)){return Err("OXP input deadline exceeded".into());}
        adapter.step()?;adapter.output.pump(&mut stream)?;
        let events=libc::POLLIN|if adapter.output.queue.is_empty(){0}else{libc::POLLOUT};let mut fd=libc::pollfd{fd:stream.as_raw_fd(),events,revents:0};
        unsafe{libc::poll(&mut fd,1,2);}
    }Ok(())
}
