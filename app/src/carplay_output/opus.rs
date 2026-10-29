//! Native Opus encoder; capture remains PCM16 mono and stereo is duplicated.
use super::Result;
use libloading::Library;
use std::{ffi::c_void,path::Path};
type Create=unsafe extern "C" fn(i32,i32,i32,*mut i32)->*mut c_void;
type Destroy=unsafe extern "C" fn(*mut c_void);
type Encode=unsafe extern "C" fn(*mut c_void,*const i16,i32,*mut u8,i32)->i32;
type Control=unsafe extern "C" fn(*mut c_void,i32,...)->i32;
pub struct Encoder{_library:Library,handle:*mut c_void,destroy:Destroy,encode:Encode,channels:u8}
impl Encoder{
    pub fn new(path:Option<&Path>,channels:u8)->Result<Self>{
        if !matches!(channels,1|2){return Err("unsupported Opus channel count".into());}
        let path=path.map(std::path::PathBuf::from).or_else(||std::env::var_os("CARLINK_OPUS_LIBRARY").map(Into::into));
        let library=unsafe{Library::new(path.as_deref().unwrap_or(Path::new("libopus.so.0")))}.map_err(|_|"microphone Opus library unavailable")?;
        unsafe{
            let create:Create=*library.get(b"opus_encoder_create\0").map_err(|_|"Opus create unavailable")?;
            let destroy:Destroy=*library.get(b"opus_encoder_destroy\0").map_err(|_|"Opus destroy unavailable")?;
            let encode:Encode=*library.get(b"opus_encode\0").map_err(|_|"Opus encode unavailable")?;
            let control:Control=*library.get(b"opus_encoder_ctl\0").map_err(|_|"Opus control unavailable")?;
            let mut error=0;let handle=create(48000,channels.into(),2048,&mut error);
            if handle.is_null()||error!=0{if !handle.is_null(){destroy(handle);}return Err("microphone Opus encoder creation failed".into());}
            if control(handle,4002,32000i32*channels as i32)!=0||control(handle,4010,3i32)!=0{destroy(handle);return Err("microphone Opus encoder configuration failed".into());}
            Ok(Self{_library:library,handle,destroy,encode,channels})
        }
    }
    pub fn encode(&mut self,mono:&[u8])->Result<Vec<u8>>{
        if mono.len()!=1920{return Err("Opus microphone requires 960 mono samples".into());}
        let mut samples=Vec::with_capacity(960*self.channels as usize);
        for bytes in mono.chunks_exact(2){let v=i16::from_le_bytes([bytes[0],bytes[1]]);for _ in 0..self.channels{samples.push(v);}}
        let mut out=vec![0;1275];let size=unsafe{(self.encode)(self.handle,samples.as_ptr(),960,out.as_mut_ptr(),1275)};
        if size<=0{return Err("microphone Opus encoding failed".into());}out.truncate(size as usize);Ok(out)
    }
}
impl Drop for Encoder{fn drop(&mut self){unsafe{(self.destroy)(self.handle);}}}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn native_opus_packets_roundtrip_through_ffmpeg(){
        let Some(binary)=std::env::var_os("CARLINK_AUDIO_TEST_BINARY") else{return};
        use std::{io::Write,process::{Command,Stdio}};
        let mono:Vec<u8>=(0..960).flat_map(|i|((i as f64*std::f64::consts::TAU*440./48000.).sin().mul_add(9000.,0.) as i16).to_le_bytes()).collect();
        for channels in [1,2]{
            let mut encoder=Encoder::new(None,channels).unwrap();
            let mut ogg=super::super::audio::opus_headers(channels).unwrap();
            for i in 0..8{let packet=encoder.encode(&mono).unwrap();assert_eq!(super::super::audio::opus_samples(&packet).unwrap(),960);ogg.extend(super::super::audio::ogg_page(&packet,i+2,(i as u64+1)*960,false).unwrap());}
            let mut child=Command::new(&binary).args(["-hide_banner","-loglevel","error","-f","ogg","-i","pipe:0","-f","s16le","-ac","1","-ar","48000","pipe:1"]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
            child.stdin.take().unwrap().write_all(&ogg).unwrap();let result=child.wait_with_output().unwrap();assert!(result.status.success());
            let samples:Vec<i16>=result.stdout.chunks_exact(2).map(|b|i16::from_le_bytes([b[0],b[1]])).collect();assert!(samples.len()>6000);assert!(samples.iter().any(|v|v.unsigned_abs()>5000));
        }
    }
}
