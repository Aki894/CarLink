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
    origin:Option<u64>,samples:u64,submitted_samples:u64,next:Option<Instant>,failed:bool,ogg_sequence:u32,granule:u64,waiting:Option<Instant>,
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
        Ok(Self{child,input,output,format:format.clone(),queue:VecDeque::new(),pcm:vec![],origin:None,samples:0,submitted_samples:0,next:None,failed:false,ogg_sequence:0,granule:0,waiting:None})
    }
    pub fn failed(&self)->bool{self.failed}
    pub fn feed(&mut self,payload:&[u8],pts:u64)->Result<bool>{
        if self.failed{return Ok(false);}
        if payload.is_empty()||payload.len()>65536||self.format.codec=="PCM_S16LE"&&payload.len()%(2*self.format.channels as usize)!=0{return Err("invalid source audio packet".into());}
        if self.queue.len()>=16||self.queue.front().is_some_and(|p|p.queued.elapsed()>BUDGET){self.failed=true;return Ok(false);}
        let samples=packet_samples(&self.format,payload)?;
        let submitted=self.submitted_samples.saturating_add(samples);
        let rate=if self.format.codec=="OPUS"{48000}else{self.format.sample_rate} as u128;
        // Include compressed bytes already handed to FFmpeg and its pipe: a
        // small Rust queue alone does not bound native decoder audio latency.
        if (submitted as u128*44100).saturating_sub(self.samples as u128*rate)>22050*rate{self.failed=true;return Ok(false);}
        let mut data=match self.format.codec.as_str(){"AAC_LC"=>adts(payload,self.format.sample_rate,self.format.channels)?,_=>payload.to_vec()};
        if self.format.codec=="OPUS"{
            let samples=opus_samples(payload)?;data=if self.ogg_sequence==0{opus_headers(self.format.channels)?}else{vec![]};
            self.ogg_sequence=self.ogg_sequence.max(2);self.granule+=samples as u64;data.extend(ogg_page(payload,self.ogg_sequence,self.granule,false)?);self.ogg_sequence=self.ogg_sequence.wrapping_add(1);
        }
        if data.len()>65536{return Err("source audio packet exceeds limit".into());}
        self.submitted_samples=submitted;self.origin.get_or_insert(pts);let now=Instant::now();self.waiting.get_or_insert(now);self.queue.push_back(Packet{data,offset:0,queued:now});Ok(true)
    }
    pub fn poll(&mut self)->Result<Vec<(Vec<u8>,u64)>>{
        if self.failed{return Ok(vec![]);}
        if self.child.try_wait().map_err(|_|"audio decoder status unavailable")?.is_some(){self.failed=true;return Ok(vec![]);}
        if self.queue.front().is_some_and(|p|p.queued.elapsed()>BUDGET)||self.waiting.is_some_and(|t|t.elapsed()>BUDGET){self.failed=true;return Ok(vec![]);}
        // Bound both work per poll and decoded storage. The owner polls all
        // streams, so a busy decoder cannot starve video or the microphone.
        for _ in 0..16{
            let Some(packet)=self.queue.front_mut() else{break};
            match self.input.write(&packet.data[packet.offset..]){
                Ok(0)=>{self.failed=true;break},Ok(n)=>{packet.offset+=n;if packet.offset==packet.data.len(){self.queue.pop_front();}},
                Err(e)if e.kind()==ErrorKind::WouldBlock=>break,Err(e)if e.kind()==ErrorKind::Interrupted=>continue,Err(_)=>{self.failed=true;break},
            }
        }
        for _ in 0..6{
            let available=6*PCM-self.pcm.len();if available==0{break;}
            let mut bytes=[0u8;PCM];let wanted=available.min(PCM);
            match self.output.read(&mut bytes[..wanted]){
                Ok(0)=>{self.failed=true;break},Ok(n)=>{self.pcm.extend(&bytes[..n]);self.waiting=if self.queue.is_empty(){None}else{Some(Instant::now())};},
                Err(e)if e.kind()==ErrorKind::WouldBlock=>break,Err(e)if e.kind()==ErrorKind::Interrupted=>continue,Err(_)=>{self.failed=true;break},
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
        let deadline=Instant::now()+Duration::from_secs(2);let mut blocks=vec![];
        while Instant::now()<deadline&&blocks.len()<5{blocks.extend(decoder.poll().unwrap());assert!(!decoder.failed());std::thread::sleep(Duration::from_millis(2));}
        assert!(blocks.len()>=5);for (i,(pcm,pts)) in blocks.iter().enumerate(){assert_eq!(pcm.len(),PCM);assert_eq!(*pts,100000+i as u64*20000);assert!(pcm.chunks_exact(2).all(|b|b==1234i16.to_le_bytes()));}
        // Six 100 ms packets hit the latency cap before the 16-packet cap.
        let mut bounded=Decoder::new(Path::new(&binary),&format).unwrap();
        for _ in 0..5{assert!(bounded.feed(&vec![0;17640],0).unwrap());}
        assert!(!bounded.feed(&vec![0;17640],0).unwrap());assert!(bounded.failed());
    }
}
