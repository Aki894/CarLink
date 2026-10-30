//! Generation-fenced CarLife microphone PCM to OXP uplink; no recordings.
use super::{Result,now_us,opus::Encoder};
use serde_json::{Value,json};
use std::{path::PathBuf,time::{Duration,Instant}};
#[derive(Clone)]
struct Demand{stream:u8,rate:u32,channels:u8,opus:bool}
struct Capture{stream:u8,rate:u32}
pub struct Microphone{
    library:Option<PathBuf>,current:Option<Capture>,encoder:Option<Encoder>,generation:u64,
    desired:Option<Demand>,retry_at:Option<Instant>,failures:u8,healthy_packets:u32,
    samples:Vec<i16>,pcm:Vec<u8>,position:u64,sequence:u32,sent_samples:u64,origin:u64,next_pts:u64,
}
impl Microphone{
    pub fn new(opus_library:Option<PathBuf>)->Self{Self{library:opus_library,current:None,encoder:None,generation:now_us().saturating_mul(1000),desired:None,retry_at:None,failures:0,healthy_packets:0,samples:vec![],pcm:vec![],position:0,sequence:0,sent_samples:0,origin:0,next_pts:0}}
    fn request(&self,active:bool)->Value{json!({"generation":self.generation,"active":active,"sample_rate":16000,"channels":1})}
    pub fn configure(&mut self,stream:u8,request:Option<&Value>)->Result<Vec<Value>>{
        let Some(request)=request else{return Ok(self.stop(Some(stream)))};
        let codec=request["codec"].as_str().ok_or("unsupported microphone format")?;
        let rate=request["sampleRate"].as_u64().filter(|r|matches!(r,8000|16000|24000|32000|44100|48000)).ok_or("unsupported microphone rate")? as u32;
        let channels=request["channels"].as_u64().filter(|c|matches!(c,1|2)).ok_or("unsupported microphone channels")? as u8;
        if !request.is_object()||!matches!(codec,"PCM_S16LE"|"OPUS")||request["frameMs"].as_u64()!=Some(20)||codec=="PCM_S16LE"&&channels!=1{return Err("unsupported microphone format".into());}
        // Syntax errors leave the old demand intact. Native construction
        // errors retain the new valid demand for bounded automatic recovery.
        let encoder=if codec=="OPUS"{Encoder::new(self.library.as_deref(),channels).map(Some)}else{Ok(None)};
        let mut commands=self.stop(None);self.desired=Some(Demand{stream,rate,channels,opus:codec=="OPUS"});
        let encoder=match encoder{Ok(v)=>v,Err(_)=>{commands.extend(self.fault());return Ok(commands);}};
        self.generation=self.generation.wrapping_add(1);self.current=Some(Capture{stream,rate:if encoder.is_some(){48000}else{rate}});self.encoder=encoder;
        self.samples.clear();self.pcm.clear();self.position=0;self.sequence=0;self.sent_samples=0;self.origin=now_us();self.next_pts=self.origin;commands.push(self.request(true));Ok(commands)
    }
    fn stop_capture(&mut self)->Vec<Value>{
        let commands=if self.current.is_some(){vec![self.request(false)]}else{vec![]};
        self.current=None;self.encoder=None;self.samples.clear();self.pcm.clear();self.position=0;self.healthy_packets=0;commands
    }
    pub fn stop(&mut self,stream:Option<u8>)->Vec<Value>{
        if stream.is_some_and(|s|self.desired.as_ref().is_none_or(|d|d.stream!=s)){return vec![];}
        self.desired=None;self.retry_at=None;self.failures=0;self.stop_capture()
    }
    /// Preserve the phone's negotiated demand while retiring failed capture.
    pub fn fault(&mut self)->Vec<Value>{
        let commands=self.stop_capture();
        if self.desired.is_some(){self.failures=self.failures.saturating_add(1).min(6);self.retry_at=Some(Instant::now()+Duration::from_millis(250u64<<self.failures.saturating_sub(1)));}
        commands
    }
    pub fn retry(&mut self)->Vec<Value>{
        if !self.retry_at.is_some_and(|at|Instant::now()>=at){return vec![];}
        let Some(demand)=self.desired.clone()else{self.retry_at=None;return vec![];};
        let encoder=if demand.opus{match Encoder::new(self.library.as_deref(),demand.channels){Ok(v)=>Some(v),Err(_)=>{self.fault();return vec![];}}}else{None};
        self.retry_at=None;self.generation=self.generation.wrapping_add(1);self.current=Some(Capture{stream:demand.stream,rate:if encoder.is_some(){48000}else{demand.rate}});self.encoder=encoder;
        self.samples.clear();self.pcm.clear();self.position=0;self.sent_samples=0;self.origin=now_us().max(self.next_pts);vec![self.request(true)]
    }
    pub fn feed(&mut self,generation:u64,pcm:&[u8])->Result<Vec<Vec<u8>>>{
        let Some(current)=&self.current else{return Ok(vec![])};if generation!=self.generation{return Ok(vec![]);}
        if pcm.is_empty()||pcm.len()>16384||pcm.len()%2!=0{return Err("invalid microphone PCM".into());}
        let rate=current.rate;let stream=current.stream;
        if rate==16000{self.pcm.extend(pcm);}else{
            self.samples.extend(pcm.chunks_exact(2).map(|b|i16::from_le_bytes([b[0],b[1]])));
            while self.position/rate as u64+1<(self.samples.len() as u64){
                let index=(self.position/rate as u64) as usize;let fraction=(self.position%rate as u64) as i64;
                let value=(self.samples[index] as i64*(rate as i64-fraction)+self.samples[index+1] as i64*fraction).div_euclid(rate as i64) as i16;
                self.pcm.extend(value.to_le_bytes());self.position+=16000;
            }
            let consumed=(self.position/rate as u64).min(self.samples.len().saturating_sub(1) as u64) as usize;
            self.samples.drain(..consumed);self.position-=consumed as u64*rate as u64;
        }
        let size=rate as usize/50*2;let mut packets=vec![];
        while self.pcm.len()>=size{
            let mut payload:Vec<u8>=self.pcm.drain(..size).collect();if let Some(encoder)=&mut self.encoder{payload=encoder.encode(&payload)?;}
            let pts=self.origin.saturating_add(self.sent_samples*1_000_000/rate as u64);
            packets.push(super::wire::binary(3,stream,0,self.sequence,pts as i64,now_us() as i64,&payload)?);
            self.sequence=self.sequence.wrapping_add(1);self.sent_samples+=size as u64/2;self.next_pts=pts.saturating_add(20_000);self.healthy_packets=self.healthy_packets.saturating_add(1);
            if self.healthy_packets>=50{self.failures=0;}
        }Ok(packets)
    }
}
#[cfg(test)]mod tests{
    use super::*;
    fn format(rate:u32)->Value{json!({"codec":"PCM_S16LE","sampleRate":rate,"channels":1,"frameMs":20})}
    #[test]fn generations_stop_and_partial_blocks_are_fenced(){
        let mut m=Microphone::new(None);let start=m.configure(1,Some(&format(16000))).unwrap();let old=start[0]["generation"].as_u64().unwrap();
        assert!(m.feed(old,&[0;638]).unwrap().is_empty());let commands=m.configure(2,Some(&format(16000))).unwrap();assert_eq!(commands.len(),2);
        assert!(m.feed(old,&[0;640]).unwrap().is_empty());assert!(m.stop(Some(1)).is_empty());let generation=commands[1]["generation"].as_u64().unwrap();
        assert!(m.feed(generation,&[0;2]).unwrap().is_empty());let packets=m.feed(generation,&[0;638]).unwrap();assert_eq!(packets.len(),1);assert_eq!(packets[0].len(),668);assert_eq!(&packets[0][4..6],&[3,2]);
        assert_eq!(m.stop(None).len(),1);assert!(m.feed(generation,&[0;640]).unwrap().is_empty());
    }
    #[test]fn failed_capture_retries_only_current_demand_with_a_new_generation(){
        let mut m=Microphone::new(None);let old=m.configure(3,Some(&format(16000))).unwrap()[0]["generation"].as_u64().unwrap();
        assert_eq!(m.fault()[0]["active"],false);assert!(m.current.is_none());assert!(m.retry().is_empty());
        assert!(m.stop(Some(2)).is_empty());assert!(m.retry_at.is_some());
        m.retry_at=Some(Instant::now());let resumed=m.retry();let generation=resumed[0]["generation"].as_u64().unwrap();assert!(generation>old);
        assert!(m.feed(old,&[0;640]).unwrap().is_empty());assert_eq!(m.feed(generation,&[0;640]).unwrap().len(),1);
        for n in 2u32..=9{
            m.fault();let left=m.retry_at.unwrap().saturating_duration_since(Instant::now());
            let expected=250u64<<((n-1).min(5));assert!(left<=Duration::from_millis(expected)&&left>Duration::from_millis(expected-50));
            m.retry_at=Some(Instant::now());assert_eq!(m.retry().len(),1);
        }
        for _ in 0..50{m.feed(m.generation,&[0;640]).unwrap();}m.fault();assert_eq!(m.failures,1);
        m.stop(Some(3));m.retry_at=Some(Instant::now());assert!(m.retry().is_empty());assert!(m.desired.is_none());
    }
    #[test]fn retry_preserves_sequence_and_never_rewinds_a_burst_capture_clock(){
        let mut m=Microphone::new(None);let generation=m.configure(3,Some(&format(16000))).unwrap()[0]["generation"].as_u64().unwrap();
        let old=m.feed(generation,&[0;12800]).unwrap();assert_eq!(old.len(),20);
        let last=old.last().unwrap();let last_pts=u64::from_le_bytes(last[12..20].try_into().unwrap());
        m.fault();m.retry_at=Some(Instant::now());let next=m.retry()[0]["generation"].as_u64().unwrap();
        let packet=m.feed(next,&[0;640]).unwrap().remove(0);
        assert_eq!(u32::from_le_bytes(packet[8..12].try_into().unwrap()),20);
        assert!(u64::from_le_bytes(packet[12..20].try_into().unwrap())>=last_pts+20_000);
    }
    #[test]fn missing_native_encoder_schedules_recovery_without_rejecting_the_session(){
        let mut m=Microphone::new(Some(PathBuf::from("/nonexistent/carlink-test-opus.so")));
        let request=json!({"codec":"OPUS","sampleRate":48000,"channels":1,"frameMs":20});
        assert!(m.configure(7,Some(&request)).unwrap().is_empty());assert!(m.current.is_none());assert!(m.desired.is_some());assert!(m.retry_at.is_some());
        m.retry_at=Some(Instant::now());assert!(m.retry().is_empty());assert_eq!(m.failures,2);
        m.stop(Some(7));assert!(m.retry_at.is_none());assert!(m.desired.is_none());
    }
    #[test]fn irregular_resampling_matches_python_floor_for_negative_samples(){
        for rate in [8000,24000,32000,44100,48000]{
            let mut m=Microphone::new(None);let generation=m.configure(1,Some(&format(rate))).unwrap()[0]["generation"].as_u64().unwrap();
            let values:Vec<u8>=(0..337).flat_map(|_|(-1234i16).to_le_bytes()).collect();let mut count=0;
            for _ in 0..50{for packet in m.feed(generation,&values).unwrap(){count+=1;assert_eq!(packet.len()-28,rate as usize/50*2);assert!(packet[28..].chunks_exact(2).all(|b|b==(-1234i16).to_le_bytes()));}}
            assert!(count>45);assert!(m.samples.len()<=2);assert!(m.pcm.len()<rate as usize/50*2);
        }
        // Linear interpolation must floor a negative non-integral result.
        let mut m=Microphone::new(None);let generation=m.configure(1,Some(&format(48000))).unwrap()[0]["generation"].as_u64().unwrap();m.feed(generation,&[0,0,255,255]).unwrap();
        assert_eq!(&m.pcm[..6],&[0,0,255,255,255,255]);
    }
    #[test]fn malformed_formats_and_pcm_leave_no_unchecked_input(){
        let mut m=Microphone::new(None);for request in [json!({}),json!({"codec":"PCM_S16LE","sampleRate":16000,"channels":2,"frameMs":20}),json!({"codec":"PCM_S16LE","sampleRate":16000,"channels":1,"frameMs":true})]{assert!(m.configure(1,Some(&request)).is_err());}
        let generation=m.configure(1,Some(&format(16000))).unwrap()[0]["generation"].as_u64().unwrap();for pcm in [vec![],vec![1],vec![0;16386]]{assert!(m.feed(generation,&pcm).is_err());}
    }
    #[test]fn resampling_is_invariant_to_input_boundaries_and_packets_keep_the_sample_clock(){
        let input:Vec<u8>=(0..1600).flat_map(|i|((i%97) as i16-48).to_le_bytes()).collect();
        for rate in [8000,16000,24000,32000,44100,48000]{
            let mut whole=Microphone::new(None);whole.configure(7,Some(&format(rate))).unwrap();let all=whole.feed(whole.generation,&input).unwrap();
            let mut split=Microphone::new(None);split.configure(7,Some(&format(rate))).unwrap();let mut packets=vec![];
            for chunk in input.chunks(148){packets.extend(split.feed(split.generation,chunk).unwrap());}
            assert_eq!(all.len(),packets.len());assert_eq!(whole.pcm,split.pcm);assert_eq!(whole.samples,split.samples);
            let first=u64::from_le_bytes(packets[0][12..20].try_into().unwrap());
            for (i,(a,b)) in all.iter().zip(&packets).enumerate(){assert_eq!(&a[28..],&b[28..]);assert_eq!(u32::from_le_bytes(b[8..12].try_into().unwrap()),i as u32);assert_eq!(u64::from_le_bytes(b[12..20].try_into().unwrap()),first+i as u64*20000);}
        }
    }
}
