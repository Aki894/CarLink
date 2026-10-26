//! CarLife microphone ownership. PCM stays in bounded binary IPC, never logs/JSON.
use crate::carlife::{Channel, Message};
use std::collections::VecDeque;

#[derive(Default)]
pub struct Voice {
    pub capture: Option<u64>,
    pub call: bool,
    speech: bool,
    announced: bool,
    pending: VecDeque<Message>,
}
impl Voice {
    pub fn congested(&self)->bool{self.pending.len()>=24}
    fn cmd(&mut self,id:u32,payload:Vec<u8>) {
        self.pending.push_back(Message::command(id,payload).expect("fixed voice command"));
    }
    fn status(&mut self) {
        let active=self.capture.is_some()||self.call||self.speech;
        if active!=self.announced {
            self.announced=active;
            self.cmd(0x10026,vec![8,1,18,4,8,4,16,active as u8]);
        }
    }
    pub fn microphone(&mut self,generation:u64,active:bool) {
        if active&&self.capture==Some(generation)||!active&&self.capture!=Some(generation){return;}
        if self.capture.take().is_some(){self.cmd(0x10023,vec![]);}
        if active {self.capture=Some(generation);self.status();self.cmd(0x10024,vec![]);}
        else {self.status();}
    }
    pub fn call(&mut self,active:bool){self.call=active;self.status();}
    pub fn speech(&mut self,active:bool){self.speech=active;self.status();}
    pub fn stop(&mut self){
        if self.capture.take().is_some(){self.cmd(0x10023,vec![]);}
        self.call=false;self.speech=false;self.status();
    }
    pub fn pop(&mut self)->Option<Message>{self.pending.pop_front()}
    pub fn pcm(&self,message:&Message)->Option<Vec<u8>>{
        let generation=self.capture?;
        if message.channel!=Channel::Vr||message.id!=0x58001||message.payload.is_empty()
            ||message.payload.len()>16384||message.payload.len()%2!=0{return None;}
        let mut body=Vec::with_capacity(message.payload.len()+20);
        body.extend([130,0,0,0]);body.extend((message.timestamp_ms.unwrap_or(0) as u64*1000).to_le_bytes());
        body.extend(generation.to_le_bytes());body.extend(&message.payload);
        let mut wire=(body.len() as u32).to_le_bytes().to_vec();wire.extend(body);Some(wire)
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn ownership_and_stop_order(){
        let mut v=Voice::default();v.microphone(1,true);
        assert_eq!(v.pop().unwrap().id,0x10026);assert_eq!(v.pop().unwrap().id,0x10024);
        v.microphone(1,true);assert!(v.pop().is_none());v.call(true);
        v.microphone(1,false);assert_eq!(v.pop().unwrap().id,0x10023);assert!(v.pop().is_none());
        v.call(false);assert_eq!(v.pop().unwrap().payload.last(),Some(&0));
        v.microphone(2,true);while v.pop().is_some(){}v.microphone(1,false);assert_eq!(v.capture,Some(2));
        v.stop();assert_eq!(v.pop().unwrap().id,0x10023);assert_eq!(v.pop().unwrap().payload.last(),Some(&0));
    }
    #[test] fn microphone_binary_gate(){
        let mut v=Voice::default();let mut m=Message{channel:Channel::Vr,id:0x58001,timestamp_ms:Some(50),reserved:0,payload:vec![3;1024]};
        assert!(v.pcm(&m).is_none());v.microphone(71,true);let wire=v.pcm(&m).unwrap();
        assert_eq!(&wire[16..24],&71u64.to_le_bytes());assert_eq!(&wire[24..],&m.payload);
        m.payload.push(1);assert!(v.pcm(&m).is_none());v.stop();assert!(v.pcm(&m).is_none());
    }
}
