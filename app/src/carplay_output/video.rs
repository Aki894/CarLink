//! AVC configuration and four-byte length-prefixed H.264 access units.
use super::Result;

pub fn annexb(data:&[u8])->Result<Vec<u8>>{
    let mut out=Vec::with_capacity(data.len());let mut pos=0;
    while pos<data.len(){
        if data.len()-pos<4{return Err("truncated H.264 NAL length".into());}
        let size=u32::from_be_bytes(data[pos..pos+4].try_into().unwrap())as usize;pos+=4;
        if size==0||size>data.len()-pos{return Err("invalid H.264 NAL length".into());}
        out.extend([0,0,0,1]);out.extend(&data[pos..pos+size]);pos+=size;
    }
    if out.is_empty(){return Err("empty H.264 access unit".into());}Ok(out)
}
pub fn parameters(config_base64:&str)->Result<Vec<u8>>{avc_parameters(&base64(config_base64)?)}
fn avc_parameters(data:&[u8])->Result<Vec<u8>>{
    if data.len()<7||data[0]!=1||data[4]&3!=3{return Err("expected AVC configuration with four-byte NAL lengths".into());}
    let mut pos=6;let mut out=Vec::new();let sps=data[5]&31;
    if sps==0{return Err("missing AVC SPS".into());}
    for group in 0..2{
        let count=if group==0{sps}else{let count=*data.get(pos).ok_or("missing AVC PPS count")?;pos+=1;if count==0{return Err("missing AVC PPS".into());}count};
        for _ in 0..count{
            if data.len()-pos<2{return Err("truncated AVC parameter length".into());}
            let size=u16::from_be_bytes(data[pos..pos+2].try_into().unwrap())as usize;pos+=2;
            if size==0||size>data.len()-pos{return Err("invalid AVC parameter length".into());}
            out.extend([0,0,0,1]);out.extend(&data[pos..pos+size]);pos+=size;
        }
    }Ok(out)
}
fn base64(text:&str)->Result<Vec<u8>>{
    let text=text.as_bytes();if text.len()>256*1024||text.len()%4!=0{return Err("invalid AVC base64 length".into());}
    let decode=|b|->Result<u8>{match b{b'A'..=b'Z'=>Ok(b-b'A'),b'a'..=b'z'=>Ok(b-b'a'+26),b'0'..=b'9'=>Ok(b-b'0'+52),b'+'=>Ok(62),b'/'=>Ok(63),_=>Err("invalid AVC base64 character".into())}};
    let mut out=Vec::with_capacity(text.len()/4*3);
    for (i,q)in text.chunks_exact(4).enumerate(){
        let a=decode(q[0])?;let b=decode(q[1])?;let last=i+1==text.len()/4;
        out.push(a<<2|b>>4);
        if q[2]==b'='{if !last||q[3]!=b'='{return Err("invalid AVC base64 padding".into());}continue;}
        let c=decode(q[2])?;out.push(b<<4|c>>2);
        if q[3]==b'='{if !last{return Err("invalid AVC base64 padding".into());}continue;}
        let d=decode(q[3])?;out.push(c<<6|d);
    }Ok(out)
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn avcc_vectors_match_python_and_extensions_remain_compatible(){
        // Golden bytes: 0142c01effe1000267800100026880.
        let raw=[1,66,192,30,255,225,0,2,103,128,1,0,2,104,128];
        let expected=[0,0,0,1,103,128,0,0,0,1,104,128];
        assert_eq!(avc_parameters(&raw).unwrap(),expected);
        assert_eq!(parameters("AULAHv/hAAJngAEAAmiA").unwrap(),expected);
        let mut extended=raw.to_vec();extended.extend([0xff,0,0]);assert_eq!(avc_parameters(&extended).unwrap(),expected);
        for n in 0..raw.len(){assert!(avc_parameters(&raw[..n]).is_err());}
        let mut missing=raw;missing[5]=224;assert!(avc_parameters(&missing).is_err());
        let mut missing=raw;missing[10]=0;assert!(avc_parameters(&missing).is_err());
    }
    #[test]fn annexb_keeps_binary_nals_and_rejects_truncation(){
        assert_eq!(annexb(&[0,0,0,2,101,128,0,0,0,1,9]).unwrap(),[0,0,0,1,101,128,0,0,0,1,9]);
        for bytes in [vec![],vec![0],vec![0,0,0,0],vec![0,0,0,3,101]]{assert!(annexb(&bytes).is_err());}
    }
    #[test]fn base64_accepts_standard_alphabet_and_rejects_malformed_padding(){
        assert_eq!(base64("AA==").unwrap(),[0]);assert_eq!(base64("AP8=").unwrap(),[0,255]);assert_eq!(base64("+/8A").unwrap(),[251,255,0]);
        for text in ["A","A===","=AAA","AA=A","AA==AAAA","AA\n=","????"]{assert!(base64(text).is_err());}
    }
}
