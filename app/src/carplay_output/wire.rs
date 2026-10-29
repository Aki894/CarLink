//! Bounded OXP v0 framing. Media payloads remain binary.
use super::Result;
use serde_json::Value;

pub const MAX_FRAME:usize=16*1024*1024;
pub const MAX_JSON:usize=256*1024;
const HEADER:usize=24;
#[derive(Debug)]
pub enum Message{Json(Value),Binary(Binary)}
#[derive(Debug)]
pub struct Binary{pub kind:u8,pub stream:u8,pub flags:u8,pub sequence:u32,pub pts:i64,pub sender:i64,pub payload:Vec<u8>}
pub struct Decoder{prefix:[u8;4],prefix_len:usize,expected:Option<usize>,body:Vec<u8>}
impl Decoder{
    pub fn new()->Self{Self{prefix:[0;4],prefix_len:0,expected:None,body:Vec::new()}}
    pub fn feed(&mut self,mut bytes:&[u8])->Result<Vec<Message>>{
        let mut messages=Vec::new();
        while !bytes.is_empty(){
            if self.expected.is_none(){
                let n=(4-self.prefix_len).min(bytes.len());
                self.prefix[self.prefix_len..self.prefix_len+n].copy_from_slice(&bytes[..n]);self.prefix_len+=n;bytes=&bytes[n..];
                if self.prefix_len<4{break;}
                let n=u32::from_le_bytes(self.prefix) as usize;
                if !(1..=MAX_FRAME).contains(&n){return Err("invalid OXP frame length".into());}
                self.expected=Some(n);self.prefix_len=0;
            }
            let expected=self.expected.unwrap();
            if self.body.first().or_else(||bytes.first())==Some(&b'{')&&expected>MAX_JSON{return Err("OXP JSON frame too large".into());}
            let n=(expected-self.body.len()).min(bytes.len());
            self.body.extend_from_slice(&bytes[..n]);bytes=&bytes[n..];
            if self.body.first()==Some(&b'{')&&expected>MAX_JSON{return Err("OXP JSON frame too large".into());}
            if self.body.len()<expected{break;}
            let message=if self.body[0]==b'{'{
                let value:Value=serde_json::from_slice(&self.body).map_err(|_|"invalid OXP JSON")?;
                if !value.is_object(){return Err("expected OXP JSON object".into());}Message::Json(value)
            }else{
                if self.body.len()<HEADER{return Err("truncated OXP binary header".into());}
                let b=&self.body;if !valid_kind(b[0])||b[2]>3||b[3]!=0{return Err("invalid OXP binary header".into());}
                Message::Binary(Binary{kind:b[0],stream:b[1],flags:b[2],sequence:u32::from_le_bytes(b[4..8].try_into().unwrap()),
                    pts:i64::from_le_bytes(b[8..16].try_into().unwrap()),sender:i64::from_le_bytes(b[16..24].try_into().unwrap()),payload:b[HEADER..].to_vec()})
            };
            messages.push(message);self.body.clear();self.expected=None;
        }Ok(messages)
    }
    pub fn finish(&self)->Result<()>{if self.prefix_len!=0||self.expected.is_some(){Err("truncated OXP frame at EOF".into())}else{Ok(())}}
}
fn valid_kind(kind:u8)->bool{matches!(kind,1|2|3|16|17)}
pub fn json(value:&Value)->Result<Vec<u8>>{
    if !value.is_object(){return Err("expected OXP JSON object".into());}
    let body=serde_json::to_vec(value).map_err(|_|"cannot encode OXP JSON")?;
    if body.len()>MAX_JSON{return Err("OXP JSON frame too large".into());}
    let mut out=Vec::with_capacity(4+body.len());out.extend((body.len() as u32).to_le_bytes());out.extend(body);Ok(out)
}
pub fn binary(kind:u8,stream:u8,flags:u8,sequence:u32,pts:i64,sender:i64,payload:&[u8])->Result<Vec<u8>>{
    if !valid_kind(kind)||flags>3||payload.len()>MAX_FRAME-HEADER{return Err("invalid OXP binary frame".into());}
    let mut out=Vec::with_capacity(4+HEADER+payload.len());out.extend(((HEADER+payload.len())as u32).to_le_bytes());
    out.extend([kind,stream,flags,0]);out.extend(sequence.to_le_bytes());out.extend(pts.to_le_bytes());out.extend(sender.to_le_bytes());out.extend(payload);Ok(out)
}
pub fn control(value:&Value)->Result<Vec<u8>>{
    let object=value.as_object().ok_or("expected OXP control object")?;
    if object.len()==2&&value["type"]=="keyframe"&&value["screen"].as_u64()==Some(0){return json(value);}
    let now=i64::try_from(super::now_us()).map_err(|_|"OXP control clock overflow")?;
    if value["type"]=="key-press"{
        let code=value["code"].as_u64().filter(|v|*v<=10).ok_or("invalid OXP focus key")? as u16;
        if object.len()!=2{return Err("invalid OXP focus key fields".into());}
        let mut out=Vec::with_capacity(62);
        for down in [1,0]{let mut payload=code.to_le_bytes().to_vec();payload.push(down);out.extend(binary(17,0,0,0,now,now,&payload)?);}return Ok(out);
    }
    if object.len()!=2||value["type"]!="touch"{return Err("expected OXP keyframe or touch control".into());}
    let contacts=value["contacts"].as_array().filter(|c|c.len()<=10).ok_or("invalid OXP contacts")?;
    let mut payload=Vec::with_capacity(1+contacts.len()*6);payload.push(contacts.len()as u8);let mut seen=0u16;
    for c in contacts{
        if c.as_object().is_none_or(|o|o.len()!=4){return Err("invalid OXP contact fields".into());}
        let id=c["id"].as_u64().filter(|v|*v<=9).ok_or("invalid OXP contact id")? as u8;
        if seen&(1u16<<id)!=0{return Err("duplicate OXP contact id".into());}seen|=1u16<<id;
        let down=c["down"].as_bool().ok_or("invalid OXP contact state")?;
        let coordinate=|name:&str|->Result<u16>{let v=c[name].as_f64().filter(|v|v.is_finite()&&(0.0..=1.0).contains(v)).ok_or("invalid OXP contact coordinate")?;Ok((v*65535.0+0.5)as u16)};
        payload.extend([id,u8::from(down)]);payload.extend(coordinate("x")?.to_le_bytes());payload.extend(coordinate("y")?.to_le_bytes());
    }
    binary(16,0,0,0,now,now,&payload)
}

#[cfg(test)]mod tests{
    use super::*;use serde_json::json;
    #[test]fn partial_and_concatenated_frames_preserve_signed_clocks_and_binary_payload(){
        let mut wire=json(&json!({"type":"hello","version":0})).unwrap();
        wire.extend(binary(1,7,3,u32::MAX,-9,i64::MAX,&[0,255,b'{']).unwrap());
        for split in 0..=wire.len(){let mut d=Decoder::new();let mut values=d.feed(&wire[..split]).unwrap();values.extend(d.feed(&wire[split..]).unwrap());d.finish().unwrap();
            assert_eq!(values.len(),2);assert!(matches!(&values[0],Message::Json(v)if v["type"]=="hello"));
            match &values[1]{Message::Binary(v)=>{assert_eq!((v.kind,v.stream,v.flags,v.sequence,v.pts,v.sender),(1,7,3,u32::MAX,-9,i64::MAX));assert_eq!(v.payload,[0,255,b'{']);},_=>panic!()}
        }
        let mut d=Decoder::new();let mut count=0;for b in &wire{count+=d.feed(&[*b]).unwrap().len();}assert_eq!(count,2);d.finish().unwrap();
    }
    #[test]fn pinned_upstream_video_vector_is_byte_exact(){
        // OpenXPlay-Stream spec/vectors/wire/video.hex (OXP v0).
        let expected=[30,0,0,0,1,0,3,0,7,0,0,0,64,226,1,0,0,0,0,0,71,148,3,0,0,0,0,0,0,0,0,2,101,0];
        assert_eq!(binary(1,0,3,7,123456,234567,&[0,0,0,2,101,0]).unwrap(),expected);
        let mut d=Decoder::new();assert!(matches!(&d.feed(&expected).unwrap()[0],Message::Binary(v)if v.sequence==7&&v.pts==123456&&v.sender==234567));d.finish().unwrap();
    }
    #[test]fn malformed_lengths_headers_json_and_eof_are_rejected(){
        for size in [0u32,(MAX_FRAME+1)as u32]{assert!(Decoder::new().feed(&size.to_le_bytes()).is_err());}
        let mut d=Decoder::new();assert!(d.feed(&((MAX_JSON+1)as u32).to_le_bytes()).unwrap().is_empty());assert!(d.feed(b"{").is_err());
        for body in [b"{bad}".to_vec(),vec![1;23],vec![0;24]]{let mut bytes=(body.len()as u32).to_le_bytes().to_vec();bytes.extend(body);assert!(Decoder::new().feed(&bytes).is_err());}
        for index in [4usize,6,7]{let mut b=binary(1,0,0,0,0,0,&[]).unwrap();b[index]=255;assert!(Decoder::new().feed(&b).is_err());}
        let b=binary(2,0,0,0,0,0,b"abc").unwrap();for n in 1..b.len(){let mut d=Decoder::new();d.feed(&b[..n]).unwrap();assert!(d.finish().is_err());}
    }
    #[test]fn controls_match_python_vectors_and_keys_always_release(){
        let frames=Decoder::new().feed(&control(&json!({"type":"key-press","code":10})).unwrap()).unwrap();assert_eq!(frames.len(),2);
        for (f,down)in frames.iter().zip([1u8,0]){match f{Message::Binary(v)=>{assert_eq!(v.kind,17);assert_eq!(v.payload,[10,0,down]);assert_eq!(v.pts,v.sender);},_=>panic!()}}
        let wire=control(&json!({"type":"touch","contacts":[{"id":0,"x":0.5,"y":1,"down":true}]})).unwrap();
        match &Decoder::new().feed(&wire).unwrap()[0]{Message::Binary(v)=>assert_eq!(v.payload,[1,0,1,0,128,255,255]),_=>panic!()}
        assert!(matches!(&Decoder::new().feed(&control(&json!({"type":"keyframe","screen":0})).unwrap()).unwrap()[0],Message::Json(_)));
        for v in [json!({"type":"key-press","code":true}),json!({"type":"key-press","code":11}),json!({"type":"touch","contacts":[{"id":0,"x":0,"y":0,"down":true},{"id":0,"x":0,"y":0,"down":false}]}),json!({"type":"touch","contacts":[{"id":0,"x":-0.1,"y":0,"down":true}]}),json!({"type":"keyframe","screen":0,"extra":1})]{assert!(control(&v).is_err());}
    }
}
