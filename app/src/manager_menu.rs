// SPDX-License-Identifier: MIT
//! Single-owner live mux. Only the local menu is encoded; projection bytes pass through.
#[cfg(unix)]
mod unix {
use libloading::Library;
use serde_json::{json,Value};
use std::{collections::VecDeque,ffi::{c_char,c_void,CString},fs,io::{Read,Write,ErrorKind},
    os::unix::{ffi::OsStrExt,fs::PermissionsExt,net::{UnixListener,UnixStream}},path::{Path,PathBuf},
    sync::{Arc,Mutex,mpsc::{self,Receiver,SyncSender},atomic::{AtomicBool,Ordering}},thread::{self,JoinHandle},time::{Duration,Instant}};
type Result<T>=std::result::Result<T,String>;
const MAX_FRAME:usize=1024*1024;
const MAX_QUEUE:usize=2*1024*1024;
const MENU_BYTES:usize=512*1024;
#[derive(Clone,PartialEq,Eq)]
struct View {rows:Vec<String>,focus:usize,visible:bool,revision:u64}
impl Default for View {fn default()->Self{Self{rows:vec!["CarLink".into()],focus:0,visible:false,revision:0}}}
/// One producer socket, one output socket, one background IO thread. Menu model
/// is a single latest-value slot; feedback is bounded to 32 events.
pub struct Mux {view:Arc<Mutex<View>>,events:Receiver<Value>,stop:Arc<AtomicBool>,thread:Option<JoinHandle<()>>}
impl Mux {
    pub fn start(source_socket:&Path,live_socket:&Path,library:&Path,font:&Path)->Result<Self>{
        if source_socket==live_socket || !source_socket.is_absolute() || !live_socket.is_absolute(){return Err("invalid menu socket paths".into());}
        // Never unlink an existing path: another owner may be serving it.
        let listener=UnixListener::bind(source_socket).map_err(|_|"cannot bind menu source socket")?;
        if fs::set_permissions(source_socket,fs::Permissions::from_mode(0o600)).is_err() || listener.set_nonblocking(true).is_err(){let _=fs::remove_file(source_socket);return Err("cannot secure menu source socket".into());}
        let view=Arc::new(Mutex::new(View::default()));let stop=Arc::new(AtomicBool::new(false));
        let (send,events)=mpsc::sync_channel(32);let (ready_tx,ready_rx)=mpsc::sync_channel(1);
        let tv=view.clone();let ts=stop.clone();let input=source_socket.to_path_buf();let output=live_socket.to_path_buf();let library=library.to_path_buf();let font=font.to_path_buf();
        let handle=thread::spawn(move||{
            let renderer=Renderer::load(&library,&font);
            match renderer {Ok(renderer)=>{let _=ready_tx.send(Ok(()));Worker::new(listener,output,tv,ts,send,renderer).run();},Err(e)=>{let _=ready_tx.send(Err(e));}}
            let _=fs::remove_file(input);
        });
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(()))=>Ok(Self{view,events,stop,thread:Some(handle)}),
            Ok(Err(e))=>{let _=handle.join();Err(e)},
            Err(_)=>{stop.store(true,Ordering::Relaxed);let _=handle.join();Err("menu renderer initialization timed out".into())}
        }
    }
    pub fn set_menu(&self,rows:Vec<String>,focus:usize,visible:bool)->Result<()>{
        if rows.is_empty() || rows.len()>24 || focus>=rows.len() || rows.iter().any(|s|s.chars().count()>128 || s.chars().any(char::is_control)){return Err("invalid menu model".into());}
        let mut view=self.view.lock().map_err(|_|"menu state unavailable")?;
        if view.rows!=rows || view.focus!=focus || view.visible!=visible {view.revision=view.revision.wrapping_add(1);view.rows=rows;view.focus=focus;view.visible=visible;}Ok(())
    }
    pub fn try_input(&self)->Option<Value>{self.events.try_recv().ok()}
}
impl Drop for Mux {fn drop(&mut self){self.stop.store(true,Ordering::Relaxed);if let Some(t)=self.thread.take(){let _=t.join();}}}
type Create=unsafe extern "C" fn(*const c_char)->*mut c_void;
type Destroy=unsafe extern "C" fn(*mut c_void);
type Render=unsafe extern "C" fn(*mut c_void,*const *const u32,*const u32,u32,u32,*mut u8,u32)->i32;
struct Renderer {context:*mut c_void,destroy:Destroy,render:Render,_library:Library}
impl Renderer {
    fn load(library:&Path,font:&Path)->Result<Self>{
        // Libraries/fonts are pinned installed assets, never network input.
        if !library.is_absolute() || !font.is_absolute(){return Err("menu assets must have absolute paths".into());}
        let path=CString::new(font.as_os_str().as_bytes()).map_err(|_|"invalid font path")?;
        unsafe {let library=Library::new(library).map_err(|_|"cannot load menu renderer")?;
            let create:Create=*library.get(b"carlink_menu_create\0").map_err(|_|"menu renderer ABI missing")?;
            let destroy:Destroy=*library.get(b"carlink_menu_destroy\0").map_err(|_|"menu renderer ABI missing")?;
            let render:Render=*library.get(b"carlink_menu_render\0").map_err(|_|"menu renderer ABI missing")?;
            let context=create(path.as_ptr());if context.is_null(){return Err("menu renderer initialization failed".into());}
            Ok(Self{context,destroy,render,_library:library})}
    }
    fn frame(&mut self,view:&View)->Result<Vec<u8>>{
        let points:Vec<Vec<u32>>=view.rows.iter().map(|s|s.chars().map(u32::from).collect()).collect();
        let ptrs:Vec<*const u32>=points.iter().map(|s|s.as_ptr()).collect();let lengths:Vec<u32>=points.iter().map(|s|s.len() as u32).collect();
        let mut data=vec![0u8;MENU_BYTES];let n=unsafe{(self.render)(self.context,ptrs.as_ptr(),lengths.as_ptr(),points.len() as u32,view.focus as u32,data.as_mut_ptr(),data.len() as u32)};
        if n<=0 || n as usize>data.len(){return Err("menu encoding failed".into());}data.truncate(n as usize);
        if !data.starts_with(&[0,0,0,1]) && !data.starts_with(&[0,0,1]){return Err("invalid menu encoder output".into());}Ok(data)
    }
}
impl Drop for Renderer {fn drop(&mut self){unsafe{(self.destroy)(self.context);}}}
#[derive(Clone)]struct Frame {kind:u8,flags:u8,pts:u64,payload:Vec<u8>}
impl Frame {
    fn new(kind:u8,flags:u8,payload:Vec<u8>)->Self{Self{kind,flags,pts:0,payload}}
    fn wire(&self)->Vec<u8>{let mut w=((self.payload.len()+12)as u32).to_le_bytes().to_vec();w.extend([self.kind,self.flags,0,0]);w.extend(self.pts.to_le_bytes());w.extend(&self.payload);w}
}
fn configure()->Frame{Frame::new(1,0,[800u32,480,30].into_iter().flat_map(u32::to_le_bytes).collect())}
fn feedback(value:Value)->Frame{Frame::new(129,0,serde_json::to_vec(&value).unwrap())}
fn keyframe()->Frame{feedback(json!({"type":"keyframe","screen":0}))}
fn menu_frame(payload:Vec<u8>)->Frame{let mut clock=libc::timespec{tv_sec:0,tv_nsec:0};let ok=unsafe{libc::clock_gettime(libc::CLOCK_MONOTONIC,&mut clock)}==0;
    Frame{kind:2,flags:if ok{3}else{1},pts:if ok{clock.tv_sec as u64*1_000_000+clock.tv_nsec as u64/1000}else{0},payload}}
struct Pending {data:Vec<u8>,offset:usize,kind:u8}
struct Pipe {socket:UnixStream,input:Vec<u8>,output:VecDeque<Pending>,queued:usize}
impl Pipe {
    fn new(socket:UnixStream)->Result<Self>{socket.set_nonblocking(true).map_err(|_|"mux socket setup failed")?;Ok(Self{socket,input:vec![],output:VecDeque::new(),queued:0})}
    fn push(&mut self,frame:Frame)->Result<()>{let data=frame.wire();if self.queued+data.len()>MAX_QUEUE || self.output.len()>=96{return Err("mux peer is too slow".into());}self.queued+=data.len();self.output.push_back(Pending{data,offset:0,kind:frame.kind});Ok(())}
    fn discard_media(&mut self){self.output.retain(|p|p.offset>0);self.queued=self.output.iter().map(|p|p.data.len()-p.offset).sum();}
    fn discard_video(&mut self){self.output.retain(|p|p.kind!=2 || p.offset>0);self.queued=self.output.iter().map(|p|p.data.len()-p.offset).sum();}
    fn pump(&mut self,reverse:bool)->Result<Vec<Frame>>{
        let mut result=vec![];
        for _ in 0..8 {
            let mut bytes=[0u8;16384];match self.socket.read(&mut bytes){
                Ok(0)=>return Err("mux peer disconnected".into()),Ok(n)=>{self.input.extend(&bytes[..n]);if self.input.len()>MAX_FRAME+4{return Err("mux receive bound exceeded".into());}},
                Err(e) if e.kind()==ErrorKind::WouldBlock=>break,Err(e) if e.kind()==ErrorKind::Interrupted=>continue,Err(_)=>return Err("mux read failed".into())}
            while self.input.len()>=4 {
                let n=u32::from_le_bytes(self.input[..4].try_into().unwrap()) as usize;
                if !(12..=MAX_FRAME).contains(&n){return Err("invalid mux frame length".into());}
                if self.input.len()<n+4{break;}
                let body=&self.input[4..n+4];let f=Frame{kind:body[0],flags:body[1],pts:u64::from_le_bytes(body[4..12].try_into().unwrap()),payload:body[12..].to_vec()};
                if body[2..4]!=[0,0] || !valid(&f,reverse){return Err("invalid mux frame".into());}result.push(f);self.input.drain(..n+4);
                if result.len()>=64{break;}
            }
            if result.len()>=64{break;}
        }
        for _ in 0..16 {
            let Some(p)=self.output.front_mut()else{break};match self.socket.write(&p.data[p.offset..]){
                Ok(0)=>return Err("mux write disconnected".into()),Ok(n)=>{p.offset+=n;self.queued-=n;if p.offset==p.data.len(){self.output.pop_front();}},
                Err(e) if e.kind()==ErrorKind::WouldBlock=>break,Err(e) if e.kind()==ErrorKind::Interrupted=>continue,Err(_)=>return Err("mux write failed".into())}
        }Ok(result)
    }
}
fn valid(f:&Frame,reverse:bool)->bool{
    if reverse{return f.kind==129 && f.flags==0 && f.payload.len()<=16384 && serde_json::from_slice::<Value>(&f.payload).is_ok();}
    match f.kind {1=>f.flags==0&&f.payload==configure().payload,2=>f.flags<=3&&!f.payload.is_empty()&&f.payload.len()<=MENU_BYTES&&(f.payload.starts_with(&[0,0,1])||f.payload.starts_with(&[0,0,0,1])),
        3=>f.flags<=3&&!f.payload.is_empty()&&f.payload.len()<=65536&&f.payload.len()%4==0,4=>f.flags<=1&&f.payload.is_empty(),5=>f.flags<=1&&f.payload.len()==1&&f.payload[0]<=1,6=>f.flags==0&&f.payload.is_empty(),_=>false}
}
struct Worker {listener:UnixListener,output:PathBuf,view:Arc<Mutex<View>>,stop:Arc<AtomicBool>,events:SyncSender<Value>,renderer:Renderer,
    source:Option<Pipe>,hu:Option<Pipe>,snapshot:View,source_active:bool,waiting_key:bool,retry:Instant,painted:Option<Instant>,touch:Option<Value>,menu_touch:Option<(usize,f64,f64,bool)>}
impl Worker {
    fn new(listener:UnixListener,output:PathBuf,view:Arc<Mutex<View>>,stop:Arc<AtomicBool>,events:SyncSender<Value>,renderer:Renderer)->Self{
        Self{listener,output,view,stop,events,renderer,source:None,hu:None,snapshot:View::default(),source_active:false,waiting_key:true,retry:Instant::now(),painted:None,touch:None,menu_touch:None}
    }
    fn output(&mut self,f:Frame){if self.hu.as_mut().is_some_and(|p|p.push(f).is_err()){self.hu=None;self.painted=None;self.waiting_key=true;}}
    fn source(&mut self,f:Frame){if self.source.as_mut().is_some_and(|p|p.push(f).is_err()){self.source=None;self.source_active=false;}}
    fn state(&mut self){self.output(configure());self.output(Frame::new(4,u8::from(self.snapshot.visible||self.source_active),vec![]));self.waiting_key=true;self.source(keyframe());self.painted=None;}
    fn incoming(&mut self,f:Frame){match f.kind {
        1=>{if !self.snapshot.visible {self.output(f);}},
        4=>{self.source_active=f.flags==1;self.waiting_key=true;if !self.snapshot.visible {self.output(f);}},
        6=>{self.waiting_key=true;if !self.snapshot.visible {self.output(f);}},
        2=>{if !self.snapshot.visible && self.source_active {
            if self.waiting_key && f.flags&1==0{return;}
            if self.hu.as_ref().is_some_and(|h|h.queued+f.payload.len()+16>MAX_QUEUE/2){
                if let Some(h)=self.hu.as_mut(){h.discard_video();}self.waiting_key=true;self.source(keyframe());if f.flags&1==0{return;}
            }
            self.waiting_key=false;self.output(f);
        }},
        _=>self.output(f),
    }}
    fn input(&mut self,f:Frame){
        let Ok(v)=serde_json::from_slice::<Value>(&f.payload)else{return;};
        if self.snapshot.visible {
            if v["type"]=="keyframe" {self.painted=None;}else if v["type"]=="touch" {self.menu_touch(v);}else if matches!(v["type"].as_str(),Some("touch"|"key-press"|"key"|"focus")){
                // Overflow cancels the gesture rather than retaining a stale press.
                if self.events.try_send(v).is_err(){let _=self.events.try_send(json!({"type":"cancel"}));}
            }
        }else{if v["type"]=="touch"{self.touch=Some(v);}self.source(f);}
    }
    fn menu_touch(&mut self,v:Value){
        let Some(c)=v["contacts"].as_array().and_then(|c|c.first())else{self.menu_touch=None;return};
        let (Some(x),Some(y),Some(down))=(c["x"].as_f64(),c["y"].as_f64(),c["down"].as_bool())else{self.menu_touch=None;return};
        if !x.is_finite() || !y.is_finite() || !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y){self.menu_touch=None;return;}
        let row=touch_row(x,y,self.snapshot.focus,self.snapshot.rows.len());
        if down {
            if let Some((_,px,py,moved))=self.menu_touch.as_mut(){if (x-*px).abs()*800.0>12.0 || (y-*py).abs()*480.0>12.0{*moved=true;}}
            else if let Some(row)=row {self.menu_touch=Some((row,x,y,false));}
        }else if let Some((pressed,_,_,moved))=self.menu_touch.take(){
            if let Some(row)=row {let op=if moved{"menu-focus"}else{"menu-select"};if moved || row==pressed {let _=self.events.try_send(json!({"type":op,"index":row}));}}
        }
    }
    fn update(&mut self){
        let view=match self.view.lock(){Ok(v)=>v.clone(),Err(_)=>return};
        if view.revision==self.snapshot.revision{return;}
        let changed=view.visible!=self.snapshot.visible;self.snapshot=view;
        if changed {
            self.menu_touch=None;
            if let Some(h)=self.hu.as_mut(){h.discard_video();}
            if self.snapshot.visible {
                if let Some(mut v)=self.touch.take(){if let Some(contacts)=v["contacts"].as_array_mut(){for c in contacts{c["down"]=Value::Bool(false);}}self.source(feedback(v));}
            }
            self.output(Frame::new(6,0,vec![]));self.output(Frame::new(4,u8::from(self.snapshot.visible||self.source_active),vec![]));
            self.waiting_key=true;self.source(keyframe());
        }
        self.painted=None;
    }
    fn run(&mut self){
        while !self.stop.load(Ordering::Relaxed){
            self.update();
            if self.source.is_none(){match self.listener.accept(){Ok((s,_))=>{self.source=Pipe::new(s).ok();self.source_active=false;self.waiting_key=true;self.source(keyframe());
                if !self.snapshot.visible{self.output(configure());self.output(Frame::new(6,0,vec![]));self.output(Frame::new(4,0,vec![]));}},Err(e) if e.kind()==ErrorKind::WouldBlock=>{},Err(_)=>break}}
            if self.hu.is_none() && Instant::now()>=self.retry {self.retry=Instant::now()+Duration::from_millis(200);if let Ok(s)=UnixStream::connect(&self.output){self.hu=Pipe::new(s).ok();if self.hu.is_some(){self.state();}}}
            if let Some(p)=self.source.as_mut(){match p.pump(false){Ok(frames)=>{for f in frames{self.incoming(f);}},Err(_)=>{
                self.source=None;self.source_active=false;self.waiting_key=true;
                if let Some(h)=self.hu.as_mut(){h.discard_media();}self.painted=None;
                self.output(Frame::new(5,0,vec![0]));self.output(Frame::new(5,1,vec![0]));
                if !self.snapshot.visible{self.output(Frame::new(4,0,vec![]));}
            }}}
            if let Some(p)=self.hu.as_mut(){match p.pump(true){Ok(frames)=>{for f in frames{self.input(f);}},Err(_)=>{self.hu=None;self.painted=None;self.waiting_key=true;}}}
            // Menu heartbeat is an IDR once per second; repaint only on changes
            // otherwise. This keeps HU recovery independent of a source phone.
            if self.hu.is_some() && self.snapshot.visible && self.painted.is_none_or(|t|t.elapsed()>=Duration::from_secs(1)) {
                match self.renderer.frame(&self.snapshot){Ok(data)=>{self.output(menu_frame(data));self.painted=Some(Instant::now());},Err(_)=>{let _=self.events.try_send(json!({"type":"menu-error"}));self.painted=Some(Instant::now());}}
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}
fn touch_row(x:f64,y:f64,focus:usize,count:usize)->Option<usize>{
    let (x,y)=(x*800.0,y*480.0);
    if !(24.0..776.0).contains(&x) || !(76.0..426.0).contains(&y){return None;}
    let slot=((y-76.0)/50.0)as usize;if (y-76.0)%50.0>=46.0{return None;}
    let first=focus.saturating_sub(6);let index=first+slot;(index<count).then_some(index)
}
#[cfg(test)]mod tests {
    use super::*;
    #[test]fn touch_maps_visible_scroll_rows(){assert_eq!(touch_row(0.5,0.2,0,3),Some(0));assert_eq!(touch_row(0.5,0.2,8,15),Some(2));assert_eq!(touch_row(0.01,0.2,0,3),None);assert_eq!(touch_row(0.5,0.95,0,3),None);}
    #[test]fn rejects_bad_media(){assert!(valid(&configure(),false));assert!(!valid(&Frame::new(3,0,vec![1,2,3]),false));assert!(!valid(&Frame::new(129,0,b"{}".to_vec()),false));assert!(valid(&feedback(json!({"type":"keyframe"})),true));assert!(!valid(&Frame::new(2,1,vec![1,2,3]),false));}
    #[test]fn partial_video_is_never_removed(){let(a,_)=UnixStream::pair().unwrap();let mut p=Pipe::new(a).unwrap();p.push(Frame::new(2,1,vec![0,0,0,1,5])).unwrap();p.output.front_mut().unwrap().offset=2;p.queued-=2;p.push(Frame::new(2,1,vec![0,0,0,1,5])).unwrap();p.push(Frame::new(3,0,vec![0;4])).unwrap();p.discard_video();assert_eq!(p.output.len(),2);assert_eq!(p.output.front().unwrap().offset,2);assert_eq!(p.queued,p.output.iter().map(|p|p.data.len()-p.offset).sum::<usize>());}
    fn recv(s:&mut UnixStream)->Frame {let mut h=[0u8;4];s.read_exact(&mut h).unwrap();let n=u32::from_le_bytes(h)as usize;assert!((12..=MAX_FRAME).contains(&n));let mut b=vec![0;n];s.read_exact(&mut b).unwrap();Frame{kind:b[0],flags:b[1],pts:u64::from_le_bytes(b[4..12].try_into().unwrap()),payload:b[12..].to_vec()}}
    fn until(s:&mut UnixStream,kind:u8)->Frame {for _ in 0..40{let f=recv(s);if f.kind==kind{return f;}}panic!("expected frame kind {kind}");}
    #[test]fn mux_real_native_contract(){
        let Ok(lib)=std::env::var("CARLINK_MENU_LIBRARY")else{return};let font=std::env::var("CARLINK_MENU_FONT").unwrap();
        let root=std::env::temp_dir().join(format!("carlink-menu-ci-{}",std::process::id()));fs::create_dir(&root).unwrap();
        let source=root.join("source");let live=root.join("live");
        let mux=Mux::start(&source,&live,Path::new(&lib),Path::new(&font)).unwrap();
        mux.set_menu(vec!["杩炴帴 iPhone".into(),"杩炴帴鍗庝负".into()],1,true).unwrap();
        // Discovery/input socket exists before HU; the late HU can recover.
        let mut upstream=UnixStream::connect(&source).unwrap();upstream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        upstream.write_all(&configure().wire()).unwrap();upstream.write_all(&Frame::new(4,1,vec![]).wire()).unwrap();
        let server=UnixListener::bind(&live).unwrap();let(mut hu,_)=server.accept().unwrap();hu.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let menu=until(&mut hu,2);assert_eq!(menu.flags&1,1);assert!(menu.payload.len()>200);
        upstream.write_all(&Frame::new(3,0,vec![7;256]).wire()).unwrap();assert_eq!(until(&mut hu,3).payload,vec![7;256]);
        hu.write_all(&feedback(json!({"type":"key-press","code":6})).wire()).unwrap();
        let input_deadline=Instant::now()+Duration::from_secs(3);let input=loop{if let Some(v)=mux.try_input(){break v;}assert!(Instant::now()<input_deadline);thread::sleep(Duration::from_millis(10));};assert_eq!(input["code"],6);
        mux.set_menu(vec!["杩炴帴 iPhone".into()],0,false).unwrap();thread::sleep(Duration::from_millis(100));
        let key=vec![0,0,0,1,0x65,0xff,0xab,0xcd];upstream.write_all(&Frame::new(2,1,key.clone()).wire()).unwrap();
        let deadline=Instant::now()+Duration::from_secs(5);loop{let f=recv(&mut hu);if f.kind==2 && f.payload==key{break;}assert!(Instant::now()<deadline);}
        hu.write_all(&feedback(json!({"type":"touch","contacts":[{"id":0,"x":0.5,"y":0.4,"down":true}]})).wire()).unwrap();
        loop{let f=recv(&mut upstream);let v:Value=serde_json::from_slice(&f.payload).unwrap();if v["type"]=="touch"{break;}}
        mux.set_menu(vec!["CarLink".into()],0,true).unwrap();
        loop{let f=recv(&mut upstream);let v:Value=serde_json::from_slice(&f.payload).unwrap();if v["type"]=="touch"{assert_eq!(v["contacts"][0]["down"],false);break;}}
        drop(hu);drop(server);thread::sleep(Duration::from_millis(100));fs::remove_file(&live).unwrap();
        let server=UnixListener::bind(&live).unwrap();let(mut hu,_)=server.accept().unwrap();hu.set_read_timeout(Some(Duration::from_secs(5))).unwrap();assert_eq!(until(&mut hu,2).flags&1,1);
        drop(mux);assert!(!source.exists());drop(server);drop(hu);drop(upstream);fs::remove_file(live).unwrap();fs::remove_dir(root).unwrap();
    }
}
}
#[cfg(unix)]pub use unix::Mux;
