//! Bounded nonblocking FFmpeg audio decoder and 20 ms PCM playout clock.
use super::Result;
use serde_json::Value;
use std::{collections::VecDeque,io::{Read,Write,ErrorKind},os::fd::AsRawFd,path::Path,process::{Child,ChildStdin,ChildStdout,Command,Stdio},time::{Duration,Instant}};
const PCM:usize=3528;
const BUDGET:Duration=Duration::from_millis(500);
#[derive(Clone,Debug,PartialEq)]
pub struct Format{pub role:u8,pub codec:String,pub sample_rate:u32,pub channels:u8}
impl Format{
    pub fn parse(value:&Value)->Result<Self>{
        let role=match value["role"].as_str(){Some("MEDIA")=>0,Some("ALERT")=>1,Some("VOICE")=>2,Some("TELEPHONY")=>3,_=>return Err("unsupported audio role".into())};
        let codec=value["codec"].as_str().filter(|s|matches!(*s,"AAC_LC"|"AAC_ADTS"|"PCM_S16LE"|"OPUS")).ok_or("unsupported audio codec")?;
        let sample_rate=value["sampleRate"].as_u64().filter(|r|matches!(r,8000|16000|24000|32000|44100|48000)).ok_or("unsupported audio sample rate")? as u32;
        let channels=value["channels"].as_u64().filter(|c|matches!(c,1|2)).ok_or("unsupported audio channels")? as u8;
        Ok(Self{role,codec:codec.into(),sample_rate,channels})
    }
}
pub fn adts(payload:&[u8],rate:u32,channels:u8)->Result<Vec<u8>>{
    let rates=[96000,88200,64000,48000,44100,32000,24000,22050,16000,12000,11025,8000];
    let index=rates.iter().position(|r|*r==rate).ok_or("unsupported AAC sample rate")?;
    if payload.is_empty()||payload.len()>8184||!matches!(channels,1|2){return Err("unsupported AAC access unit".into());}
    let size=payload.len()+7;let mut out=vec![255,241,64|((index as u8)<<2)|(channels>>2),((channels&3)<<6)|((size>>11) as u8),(size>>3) as u8,((size&7) as u8)<<5|31,252];out.extend(payload);Ok(out)
}
pub fn opus_samples(payload:&[u8])->Result<u32>{
    let toc=*payload.first().ok_or("empty Opus packet")?;
    let duration=if toc&128!=0{120<<((toc>>3)&3)}else if toc&96==96{if toc&8!=0{960}else{480}}else{[480,960,1920,2880][((toc>>3)&3) as usize]};
    let count=match toc&3{0=>1,1|2=>2,_=>payload.get(1).copied().unwrap_or(0)&63} as u32;
    let samples=duration*count;if samples==0||samples>5760{return Err("invalid Opus duration".into());}Ok(samples)
}
pub fn ogg_page(payload:&[u8],sequence:u32,granule:u64,bos:bool)->Result<Vec<u8>>{
    if payload.is_empty()||payload.len()>=65025{return Err("invalid Ogg packet length".into());}
    let mut page=b"OggS".to_vec();page.extend([0,if bos{2}else{0}]);page.extend(granule.to_le_bytes());page.extend(1u32.to_le_bytes());page.extend(sequence.to_le_bytes());page.extend([0;4]);
    let segments=payload.len()/255+1;page.push(segments as u8);page.extend(std::iter::repeat_n(255,segments-1));page.push((payload.len()%255) as u8);page.extend(payload);
    let mut crc=0u32;for byte in &page{crc^=(*byte as u32)<<24;for _ in 0..8{crc=(crc<<1)^if crc&0x80000000!=0{0x04c11db7}else{0};}}
    page[22..26].copy_from_slice(&crc.to_le_bytes());Ok(page)
}
pub fn opus_headers(channels:u8)->Result<Vec<u8>>{
    let mut head=b"OpusHead".to_vec();head.extend([1,channels]);head.extend(0u16.to_le_bytes());head.extend(48000u32.to_le_bytes());head.extend([0,0,0]);
    let mut tags=b"OpusTags".to_vec();tags.extend(7u32.to_le_bytes());tags.extend(b"CarLink");tags.extend(0u32.to_le_bytes());
    let mut out=ogg_page(&head,0,0,true)?;out.extend(ogg_page(&tags,1,0,false)?);Ok(out)
}
fn packet_samples(format:&Format,payload:&[u8])->Result<u64>{
    match format.codec.as_str(){
        "PCM_S16LE"=>Ok((payload.len()/(2*format.channels as usize)) as u64),
        "AAC_LC"=>Ok(1024),
        "OPUS"=>Ok(opus_samples(payload)? as u64),
        "AAC_ADTS"=>{
            let mut offset=0;let mut samples=0;
            while offset<payload.len(){
                let header=payload.get(offset..offset+7).ok_or("truncated ADTS packet")?;
                let size=((header[3] as usize&3)<<11)|((header[4] as usize)<<3)|((header[5] as usize)>>5);
                let minimum=if header[1]&1==0{9}else{7};
                let rates=[96000,88200,64000,48000,44100,32000,24000,22050,16000,12000,11025,8000,7350];
                let channels=(header[2]&1)<<2|header[3]>>6;
                if header[0]!=255||header[1]&0xf6!=0xf0||size<=minimum||size>payload.len()-offset||rates.get(((header[2]>>2)&15) as usize)!=Some(&format.sample_rate)||channels!=format.channels{return Err("invalid ADTS packet".into());}
                samples+=1024*((header[6]&3) as u64+1);offset+=size;
            }Ok(samples)
        },_=>Err("unsupported audio codec".into()),
    }
}
struct Packet{data:Vec<u8>,offset:usize,queued:Instant}
pub struct Decoder{
    child:Child,input:ChildStdin,output:ChildStdout,format:Format,queue:VecDeque<Packet>,pcm:Vec<u8>,
    origin:Option<u64>,samples:u64,submitted_samples:u64,next:Option<Instant>,failed:bool,ogg_sequence:u32,granule:u64,waiting:Option<Instant>,failure:Option<&'static str>,packets:u64,decoded_bytes:u64,last_pts:u64,
}
fn nonblocking(fd:i32)->Result<()>{unsafe{let flags=libc::fcntl(fd,libc::F_GETFL);if flags<0||libc::fcntl(fd,libc::F_SETFL,flags|libc::O_NONBLOCK)<0{return Err("audio pipe nonblocking setup failed".into());}}Ok(())}
impl Decoder{
    pub fn new(binary:&Path,format:&Format)->Result<Self>{
        // Validate even when the caller builds Format directly.
        let role=["MEDIA","ALERT","VOICE","TELEPHONY"].get(format.role as usize).ok_or("unsupported audio role")?;
        Format::parse(&serde_json::json!({"role":role,"codec":format.codec,"sampleRate":format.sample_rate,"channels":format.channels}))?;
        let mut command=Command::new(binary);command.args(["-hide_banner","-loglevel","error","-nostdin","-threads","1","-probesize","32","-analyzeduration","0"]);
        match format.codec.as_str(){"AAC_LC"|"AAC_ADTS"=>{command.args(["-f","aac"]);},"PCM_S16LE"=>{command.args(["-f","s16le","-ar",&format.sample_rate.to_string(),"-ac",&format.channels.to_string()]);},"OPUS"=>{command.args(["-f","ogg"]);},_=>unreachable!()}
        let mut child=command.args(["-i","pipe:0","-vn","-threads","1","-ac","2","-ar","44100","-c:a","pcm_s16le","-f","s16le","-flush_packets","1","pipe:1"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|_|"audio decoder could not start")?;
        let input=child.stdin.take().unwrap();let output=child.stdout.take().unwrap();
        if let Err(error)=nonblocking(input.as_raw_fd()).and_then(|_|nonblocking(output.as_raw_fd())){let _=child.kill();let _=child.wait();return Err(error);}
        Ok(Self{child,input,output,format:format.clone(),queue:VecDeque::new(),pcm:vec![],origin:None,samples:0,submitted_samples:0,next:None,failed:false,ogg_sequence:0,granule:0,waiting:None,failure:None,packets:0,decoded_bytes:0,last_pts:0})
    }
    pub fn failed(&self)->bool{self.failed}
    fn fail(&mut self,reason:&'static str){self.failed=true;self.failure.get_or_insert(reason);}
    pub fn failure(&self)->Value{
        let rate=if self.format.codec=="OPUS"{48000}else{self.format.sample_rate} as f64;
        serde_json::json!({"reason":self.failure.unwrap_or("unknown"),"packets":self.packets,
            "submitted_ms":self.submitted_samples as f64*1000./rate,"played_ms":self.samples as f64*1000./44100.,
            "decoded_ms":self.decoded_bytes as f64*1000./176400.,
            "source_pts_span_ms":self.last_pts.saturating_sub(self.origin.unwrap_or(self.last_pts)) as f64/1000.,
            "queue_packets":self.queue.len(),"pcm_bytes":self.pcm.len(),
            "waiting_ms":self.waiting.map(|t|t.elapsed().as_secs_f64()*1000.),
            "queue_age_ms":self.queue.front().map(|p|p.queued.elapsed().as_secs_f64()*1000.),
            "playout_late_ms":self.next.map(|t|Instant::now().saturating_duration_since(t).as_secs_f64()*1000.)})
    }
    pub fn feed(&mut self,payload:&[u8],pts:u64)->Result<bool>{
        if self.failed{return Ok(false);}
        if payload.is_empty()||payload.len()>65536||self.format.codec=="PCM_S16LE"&&payload.len()%(2*self.format.channels as usize)!=0{return Err("invalid source audio packet".into());}
        if self.queue.len()>=16{self.fail("input_queue_full");return Ok(false);}
        if self.queue.front().is_some_and(|p|p.queued.elapsed()>BUDGET){self.fail("input_queue_expired");return Ok(false);}
        let samples=packet_samples(&self.format,payload)?;
        let submitted=self.submitted_samples.saturating_add(samples);
        let rate=if self.format.codec=="OPUS"{48000}else{self.format.sample_rate} as u128;
        // Include compressed bytes already handed to FFmpeg and its pipe: a
        // small Rust queue alone does not bound native decoder audio latency.
        if (submitted as u128*44100).saturating_sub(self.samples as u128*rate)>22050*rate{self.fail("sample_budget_exceeded");return Ok(false);}
        let mut data=match self.format.codec.as_str(){"AAC_LC"=>adts(payload,self.format.sample_rate,self.format.channels)?,_=>payload.to_vec()};
        if self.format.codec=="OPUS"{
            let samples=opus_samples(payload)?;data=if self.ogg_sequence==0{opus_headers(self.format.channels)?}else{vec![]};
            self.ogg_sequence=self.ogg_sequence.max(2);self.granule+=samples as u64;data.extend(ogg_page(payload,self.ogg_sequence,self.granule,false)?);self.ogg_sequence=self.ogg_sequence.wrapping_add(1);
        }
        if data.len()>65536{return Err("source audio packet exceeds limit".into());}
        self.submitted_samples=submitted;self.packets+=1;self.last_pts=pts;self.origin.get_or_insert(pts);let now=Instant::now();self.waiting.get_or_insert(now);self.queue.push_back(Packet{data,offset:0,queued:now});Ok(true)
    }
    pub fn poll(&mut self)->Result<Vec<(Vec<u8>,u64)>>{
        if self.failed{return Ok(vec![]);}
        if self.child.try_wait().map_err(|_|"audio decoder status unavailable")?.is_some(){self.fail("native_exited");return Ok(vec![]);}
        if self.queue.front().is_some_and(|p|p.queued.elapsed()>BUDGET){self.fail("input_queue_expired");return Ok(vec![]);}
        if self.waiting.is_some_and(|t|t.elapsed()>BUDGET){self.fail("native_output_timeout");return Ok(vec![]);}
        // Bound both work per poll and decoded storage. The owner polls all
        // streams, so a busy decoder cannot starve video or the microphone.
        for _ in 0..16{
            let Some(packet)=self.queue.front_mut() else{break};
            match self.input.write(&packet.data[packet.offset..]){
                Ok(0)=>{self.fail("native_input_closed");break},Ok(n)=>{packet.offset+=n;if packet.offset==packet.data.len(){self.queue.pop_front();}},
                Err(e)if e.kind()==ErrorKind::WouldBlock=>break,Err(e)if e.kind()==ErrorKind::Interrupted=>continue,Err(_)=>{self.fail("native_input_error");break},
            }
        }
        for _ in 0..6{
            let available=6*PCM-self.pcm.len();if available==0{break;}
            let mut bytes=[0u8;PCM];let wanted=available.min(PCM);
            match self.output.read(&mut bytes[..wanted]){
                Ok(0)=>{self.fail("native_output_closed");break},Ok(n)=>{self.decoded_bytes+=n as u64;self.pcm.extend(&bytes[..n]);self.waiting=if self.queue.is_empty(){None}else{Some(Instant::now())};},
                Err(e)if e.kind()==ErrorKind::WouldBlock=>break,Err(e)if e.kind()==ErrorKind::Interrupted=>continue,Err(_)=>{self.fail("native_output_error");break},
            }
        }
        if self.pcm.len()<PCM{return Ok(vec![]);}
        let now=Instant::now();let due=*self.next.get_or_insert(now);
        if now<due{return Ok(vec![]);}
        // One PCM block per poll, paced at 20 ms. A late owner tick rebases
        // rather than dumping old blocks into the USB output queue.
        self.next=Some(if now.saturating_duration_since(due)>Duration::from_millis(300){now+Duration::from_millis(20)}else{due+Duration::from_millis(20)});
        let pts=self.origin.unwrap_or(0).saturating_add(self.samples*1_000_000/44100);self.samples+=882;
        Ok(vec![(self.pcm.drain(..PCM).collect(),pts)])
    }
}
impl Drop for Decoder{fn drop(&mut self){let _=self.child.kill();let _=self.child.wait();}}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn formats_aac_and_opus_framing_are_strict(){
        let format=serde_json::json!({"role":"TELEPHONY","codec":"PCM_S16LE","sampleRate":16000,"channels":1});assert_eq!(Format::parse(&format).unwrap().role,3);
        for invalid in [serde_json::json!({}),serde_json::json!({"role":"MEDIA","codec":"AAC_LC","sampleRate":true,"channels":1})]{assert!(Format::parse(&invalid).is_err());}
        assert_eq!(&adts(b"abc",48000,2).unwrap()[..3],&[255,241,76]);assert!(adts(&[],48000,2).is_err());assert!(adts(&[0;8185],48000,2).is_err());
        assert_eq!(opus_samples(&[248,255,254]).unwrap(),960);assert!(opus_samples(&[3]).is_err());
        let aac=Format{role:0,codec:"AAC_ADTS".into(),sample_rate:48000,channels:2};
        let packet=adts(b"payload",48000,2).unwrap();assert_eq!(packet_samples(&aac,&packet).unwrap(),1024);
        assert_eq!(packet_samples(&aac,&[packet.as_slice(),packet.as_slice()].concat()).unwrap(),2048);assert!(packet_samples(&aac,&packet[..packet.len()-1]).is_err());
        let page=ogg_page(b"OpusHead",0,0,true).unwrap();assert_eq!(&page[..6],b"OggS\0\x02");assert_eq!(page[26],1);assert_ne!(&page[22..26],&[0;4]);
    }
    #[test]fn native_pcm_decoder_paces_and_reaps(){
        let Some(binary)=std::env::var_os("CARLINK_AUDIO_TEST_BINARY") else{return};
        let format=Format{role:0,codec:"PCM_S16LE".into(),sample_rate:44100,channels:2};
        let mut decoder=Decoder::new(Path::new(&binary),&format).unwrap();let source:Vec<u8>=(0..882*2).flat_map(|_|1234i16.to_le_bytes()).collect();
        for _ in 0..8{assert!(decoder.feed(&source,100000).unwrap());}
        // FFmpeg's live raw-PCM reader retains part of a short burst until
        // more input arrives. Keep stdin open and supply a real 20 ms stream;
        // requiring a finite 160 ms burst to drain would test EOF semantics.
        let started=Instant::now();let deadline=started+Duration::from_secs(8);let mut blocks=vec![];let mut input_due=started+Duration::from_millis(20);let mut fed=8u64;
        while Instant::now()<deadline{
            let now=Instant::now();while now>=input_due{assert!(decoder.feed(&source,100000+fed*20000).unwrap(),"{}",decoder.failure());fed+=1;input_due+=Duration::from_millis(20);}
            blocks.extend(decoder.poll().unwrap());assert!(!decoder.failed(),"{}",decoder.failure());std::thread::sleep(Duration::from_millis(2));
        }
        assert!(blocks.len()>=350,"sustained live PCM output stalled: blocks={}, diagnostics={}",blocks.len(),decoder.failure());
        for (i,(pcm,pts)) in blocks.iter().enumerate(){assert_eq!(pcm.len(),PCM);assert_eq!(*pts,100000+i as u64*20000);assert!(pcm.chunks_exact(2).all(|b|b==1234i16.to_le_bytes()));}
        // Six 100 ms packets hit the latency cap before the 16-packet cap.
        let mut bounded=Decoder::new(Path::new(&binary),&format).unwrap();
        for _ in 0..5{assert!(bounded.feed(&vec![0;17640],0).unwrap());}
        assert!(!bounded.feed(&vec![0;17640],0).unwrap());assert!(bounded.failed());
        assert_eq!(bounded.failure()["reason"],"sample_budget_exceeded");
    }
    #[test]fn native_aac_decoder_sustains_eight_seconds_of_realtime_music(){
        let Some(binary)=std::env::var_os("CARLINK_AUDIO_TEST_BINARY") else{return};
        // Generate valid, compact AAC with the same pinned native tool used on
        // the board; no external fixture or encoder dependency is required.
        let pcm:Vec<u8>=(0..12000).flat_map(|i|{
            let sample=((i as f64*std::f64::consts::TAU*440./48000.).sin()*8000.) as i16;
            [sample.to_le_bytes(),sample.to_le_bytes()].concat()
        }).collect();
        let mut encoder=Command::new(&binary).args(["-hide_banner","-loglevel","error","-f","s16le","-ar","48000","-ac","2","-i","pipe:0","-c:a","aac","-b:a","64000","-f","adts","pipe:1"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
        encoder.stdin.take().unwrap().write_all(&pcm).unwrap();let encoded=encoder.wait_with_output().unwrap();assert!(encoded.status.success());
        let mut packets=vec![];let mut offset=0;
        while offset<encoded.stdout.len(){
            let header=&encoded.stdout[offset..offset+7];
            let size=((header[3] as usize&3)<<11)|((header[4] as usize)<<3)|((header[5] as usize)>>5);
            assert!(size>7&&offset+size<=encoded.stdout.len());
            packets.push(encoded.stdout[offset+7..offset+size].to_vec());offset+=size;
        }
        assert!(packets.len()>=10);
        let format=Format{role:0,codec:"AAC_LC".into(),sample_rate:48000,channels:2};
        let mut decoder=Decoder::new(Path::new(&binary),&format).unwrap();let started=Instant::now();let mut fed=0u64;let mut blocks=0u64;let mut first=None;let mut audible=false;
        while started.elapsed()<Duration::from_secs(8){
            // Three-packet bursts exercise real radio/IPC delivery without
            // relaxing the total 500 ms native+Rust+playout sample budget.
            let elapsed=started.elapsed().as_micros() as u64;
            while fed/3*64000<=elapsed{
                assert!(decoder.feed(&packets[fed as usize%packets.len()],100000+fed*1024*1_000_000/48000).unwrap(),"{}",decoder.failure());fed+=1;
            }
            for (pcm,pts)in decoder.poll().unwrap(){
                first.get_or_insert(started.elapsed());assert_eq!(pts,100000+blocks*20000);assert_eq!(pcm.len(),PCM);
                audible|=pcm.chunks_exact(2).any(|b|i16::from_le_bytes([b[0],b[1]]).unsigned_abs()>1000);blocks+=1;
            }
            assert!(!decoder.failed(),"{}",decoder.failure());std::thread::sleep(Duration::from_millis(2));
        }
        assert!(first.unwrap()<BUDGET);assert!(blocks>=350&&audible,"sustained AAC failed: blocks={blocks}, diagnostics={}",decoder.failure());
    }

}
