// SPDX-License-Identifier: MIT
//! Lexus Remote Touch to CarPlay focus keys. Protocol/gesture behavior follows
//! the project's CarProjection CarPlayPadInput and TouchPadTapTracker references.
use crate::carlife::{Channel,Message,input_integers};

#[derive(Default)]
pub struct Input {
    down_at: Option<u64>,
    last_tap: Option<u64>,
    last_double: Option<u64>,
    second: bool,
    mechanical: bool,
    moved: bool,
    travel: f64,
    x: i64,
    y: i64,
}
impl Input {
    pub fn handle(&mut self,message:&Message,now:u64)->Result<Vec<u16>,String> {
        let pad=(0x1005a..=0x1005e).contains(&message.id) && matches!(message.channel,Channel::Cmd|Channel::Touch);
        let key=message.id==0x68008 && message.channel==Channel::Touch;
        if !pad && !key{return Ok(Vec::new());}
        // Focus changes terminate an old gesture. Pinch has no corresponding
        // control in the receiver's current OXP key vocabulary.
        if message.id==0x1005e {*self=Self::default();return Ok(Vec::new());}
        if message.id==0x1005d{return Ok(Vec::new());}
        let values=input_integers(&message.payload).map_err(|e|e.to_string())?;
        let get=|n:usize|values[n].ok_or_else(||"required input field absent".to_string());
        let mut out=Vec::new();
        match message.id {
            0x1005a=>{
                get(1)?;
                self.second=self.last_tap.is_some_and(|t|now.saturating_sub(t)<=450);
                self.last_tap=None;self.down_at=Some(now);self.mechanical=false;
                self.moved=false;self.travel=0.;self.x=0;self.y=0;
            },
            0x1005b=>{
                get(1)?;let dx=get(2)?;let dy=get(3)?;
                if dx.unsigned_abs()>4096 || dy.unsigned_abs()>4096{return Err("touchpad delta out of range".into());}
                if !self.down_at.is_some_and(|t|now.saturating_sub(t)<=5000){return Ok(out);}
                self.travel+=(dx as f64).hypot(dy as f64);self.moved|=self.travel>=6.;
                if self.moved {
                    self.x=(self.x+dx as i64).clamp(-192,192);self.y=(self.y+dy as i64).clamp(-192,192);
                    let horizontal=self.x.abs()>=self.y.abs();let axis=if horizontal{self.x}else{self.y};
                    let steps=(axis.abs()/48).min(4);let sign=axis.signum();
                    if horizontal{self.x-=sign*steps*48;}else{self.y-=sign*steps*48;}
                    let code=match (horizontal,sign>0){(true,false)=>3,(true,true)=>4,(false,false)=>5,(false,true)=>6};
                    for _ in 0..steps{out.push(code);}
                }
            },
            0x1005c=>{
                get(1)?;
                let tap=self.down_at.is_some_and(|t|now.saturating_sub(t)<=500) && !self.moved && !self.mechanical;
                if tap && self.second{out.push(2);self.last_double=Some(now);}
                self.last_tap=if tap && !self.second{Some(now)}else{None};
                self.down_at=None;self.x=0;self.y=0;self.second=false;self.moved=false;self.mechanical=false;
            },
            0x68008=>{
                match get(1)? {
                    0x14=>{
                        self.mechanical=true;self.last_tap=None;self.second=false;
                        if self.last_double.is_none_or(|t|now.saturating_sub(t)>150){out.push(2);}
                    },
                    1=>out.push(0),0xe=>out.push(1),6=>out.push(4),7=>out.push(3),
                    0xf=>out.push(9),0x10=>out.push(8),_=>{},
                }
            },_=>{},
        }
        Ok(out)
    }
}

#[cfg(test)]mod tests {
 use super::*;
 fn event(id:u32,values:&[(u8,i32)])->Message {
  let mut payload=Vec::new();
  for (field,n) in values {payload.push(field<<3);let mut v=*n as i64 as u64;while v>=128{payload.push(v as u8|128);v>>=7;}payload.push(v as u8);}
  Message{channel:Channel::Touch,id,timestamp_ms:None,reserved:0,payload}
 }
 #[test]fn signed_motion_focus_steps_and_limits(){
  let mut p=Input::default();
  assert!(p.handle(&event(0x1005b,&[(1,0),(2,-48),(3,0)]),0).unwrap().is_empty());
  p.handle(&event(0x1005a,&[(1,0)]),0).unwrap();
  assert_eq!(p.handle(&event(0x1005b,&[(1,1),(2,-48),(3,0)]),1).unwrap(),vec![3]);
  assert_eq!(p.handle(&event(0x1005b,&[(1,2),(2,0),(3,96)]),2).unwrap(),vec![6,6]);
  assert!(p.handle(&event(0x1005b,&[(1,3),(2,i32::MIN),(3,0)]),3).is_err());
  assert_eq!(p.handle(&event(0x1005b,&[(1,4),(2,4096),(3,0)]),4).unwrap().len(),4);
 }
 #[test]fn double_tap_and_mechanical_dedup(){
  let mut p=Input::default();
  for (id,t) in [(0x1005a,0),(0x1005c,50),(0x1005a,100)]{assert!(p.handle(&event(id,&[(1,t as i32)]),t).unwrap().is_empty());}
  assert_eq!(p.handle(&event(0x1005c,&[(1,150)]),150).unwrap(),vec![2]);
  assert!(p.handle(&event(0x68008,&[(1,0x14)]),200).unwrap().is_empty());
  assert_eq!(p.handle(&event(0x68008,&[(1,0x14)]),350).unwrap(),vec![2]);
  assert_eq!(p.handle(&event(0x68008,&[(1,0xe)]),351).unwrap(),vec![1]);
  assert_eq!(p.handle(&event(0x68008,&[(1,0x0f)]),351).unwrap(),vec![9]);
  assert_eq!(p.handle(&event(0x68008,&[(1,0x10)]),351).unwrap(),vec![8]);
  p.handle(&event(0x1005e,&[]),352).unwrap();assert!(p.down_at.is_none());
 }
 #[test]fn legacy_touch_channel_survives_wire_decoder_and_session(){
  let m=event(0x1005b,&[(1,20),(2,-48),(3,0)]);let wire=m.encode().unwrap();
  let mut decoder=crate::carlife::Decoder::new(crate::carlife::Limits::default());
  let decoded=decoder.feed(&wire).unwrap();assert_eq!(decoded,vec![m.clone()]);
  crate::carlife::Session::new().handle(&decoded[0]).unwrap();
  let mut bad=m;bad.id=0x10059;assert!(bad.encode().is_err());
 }
}
