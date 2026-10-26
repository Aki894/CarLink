//! Bounded PCM16 microphone encoder and encrypted HiCar RTP packets.
use super::{Result,crypto::Crypto};
use std::{collections::VecDeque,io::{Read,Write,ErrorKind},path::Path,process::{Child,ChildStdin,ChildStdout,Command,Stdio},time::{Duration,Instant}};
use std::os::fd::AsRawFd;

pub struct Uplink{
    child:Child,input:ChildStdin,output:ChildStdout,queue:VecDeque<(Vec<u8>,usize,Instant)>,encoded:Vec<u8>,
    pub generation:u64,pub port:u16,stream:u32,sequence:u16,samples:u64,
}
impl Drop for Uplink{fn drop(&mut self){let _=self.child.kill();let _=self.child.wait();}}
impl Uplink{
    pub fn new(binary:&Path,generation:u64,port:u16,stream:u32)->Result<Self>{
        if generation==0||port==0{return Err("invalid microphone generation or endpoint".into());}
        let mut child=Command::new(binary).args(["-hide_banner","-loglevel","error","-nostdin","-threads","1","-probesize","32","-analyzeduration","0","-f","s16le","-ar","16000","-ac","1","-i","pipe:0","-vn","-threads","1","-c:a","aac","-b:a","32000","-f","adts","-flush_packets","1","pipe:1"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|_|"cannot start microphone AAC encoder")?;
        let input=child.stdin.take().ok_or("missing microphone input")?;let output=child.stdout.take().ok_or("missing microphone output")?;
        for fd in [input.as_raw_fd(),output.as_raw_fd()]{let flags=unsafe{libc::fcntl(fd,libc::F_GETFL)};if flags<0||unsafe{libc::fcntl(fd,libc::F_SETFL,flags|libc::O_NONBLOCK)}<0{let _=child.kill();let _=child.wait();return Err("cannot configure microphone pipes".into());}}
        Ok(Self{child,input,output,queue:VecDeque::new(),encoded:vec![],generation,port,stream,sequence:0,samples:0})
    }
    pub fn feed(&mut self,generation:u64,data:Vec<u8>)->Result<()>{
        if generation!=self.generation{return Ok(());}
        if data.is_empty()||data.len()>16384||data.len()%2!=0{return Err("invalid microphone PCM".into());}
        if self.queue.len()>=6{return Err("microphone encoder backpressure".into());}
        self.queue.push_back((data,0,Instant::now()));Ok(())
    }
    pub fn poll(&mut self,crypto:&Crypto,key:&[u8])->Result<Vec<Vec<u8>>>{
        for _ in 0..6{
            let Some((data,offset,at))=self.queue.front_mut()else{break;};
            if at.elapsed()>Duration::from_millis(160){return Err("microphone PCM expired".into());}
            match self.input.write(&data[*offset..]){Ok(0)=>return Err("microphone encoder closed".into()),Ok(n)=>{*offset+=n;if *offset==data.len(){self.queue.pop_front();}},Err(e)if e.kind()==ErrorKind::WouldBlock=>break,Err(_)=>return Err("microphone encoder input failed".into())}
        }
        let mut bytes=[0u8;4096];for _ in 0..4{match self.output.read(&mut bytes){Ok(0)=>return Err("microphone encoder ended".into()),Ok(n)=>{self.encoded.extend(&bytes[..n]);if self.encoded.len()>32768{return Err("microphone encoded buffer exceeded".into());}},Err(e)if e.kind()==ErrorKind::WouldBlock=>break,Err(_)=>return Err("microphone encoder output failed".into())}}
        let mut packets=vec![];
        while self.encoded.len()>=7{
            if self.encoded[0]!=255||self.encoded[1]&0xf6!=0xf0{return Err("invalid encoded microphone ADTS".into());}
            let n=((self.encoded[3]&3)as usize)<<11|(self.encoded[4]as usize)<<3|(self.encoded[5]as usize)>>5;
            if !(7..=1400).contains(&n){return Err("microphone AAC packet exceeds MTU".into());}if self.encoded.len()<n{break;}
            let aac=self.encoded.drain(..n).collect::<Vec<_>>();super::media::adts(&aac)?;
            let mut rtp=vec![0x80,96];rtp.extend(self.sequence.to_be_bytes());rtp.extend(((self.samples*90000/16000)as u32).to_be_bytes());rtp.extend(self.stream.to_be_bytes());rtp.extend(aac);
            self.sequence=self.sequence.wrapping_add(1);self.samples+=1024;
            let mut iv=[0u8;16];std::fs::File::open("/dev/urandom").and_then(|mut f|f.read_exact(&mut iv)).map_err(|_|"microphone IV unavailable")?;
            let encrypted=crypto.transform(key,&iv,&rtp)?;let mut packet=vec![0xab];packet.extend(((16+encrypted.len())as u16).to_be_bytes());packet.extend(iv);packet.extend(encrypted);packets.push(packet);
        }Ok(packets)
    }
}
