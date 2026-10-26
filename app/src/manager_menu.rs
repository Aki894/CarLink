// SPDX-License-Identifier: MIT
//! Single-owner live mux. Only the local menu is encoded; projection bytes pass through.
#[cfg(unix)]
mod unix {
use libloading::Library;
use serde_json::{json,Value};
use std::{collections::VecDeque,ffi::{c_char,c_void,CString},fs,io::{Read,Write,ErrorKind},
    os::unix::{ffi::OsStrExt,fs::PermissionsExt,io::{AsRawFd,FromRawFd},net::{UnixListener,UnixStream}},path::{Path,PathBuf},
    sync::{Arc,Mutex,Condvar,mpsc::{self,Receiver,SyncSender},atomic::{AtomicBool,Ordering}},thread::{self,JoinHandle},time::{Duration,Instant}};
type Result<T>=std::result::Result<T,String>;
const MAX_FRAME:usize=1024*1024;
const MAX_QUEUE:usize=2*1024*1024;
const MENU_BYTES:usize=512*1024;
#[derive(Clone,PartialEq,Eq)]
struct View {rows:Vec<String>,focus:usize,visible:bool,revision:u64}
impl Default for View {fn default()->Self{Self{rows:vec!["CarLink".into()],focus:0,visible:false,revision:0}}}
/// One producer socket and one output socket. Native rendering is confined
/// to its own thread; latest-view/result slots keep native work off media IO.
pub struct Mux {view:Arc<Mutex<View>>,wake:Arc<Condvar>,events:Receiver<Value>,stop:Arc<AtomicBool>,thread:Option<JoinHandle<()>>,renderer:Option<JoinHandle<()>>}
type RenderSlot=Arc<Mutex<Option<(u64,Result<Vec<u8>>)>>>;
fn request_stop(view:&Mutex<View>,wake:&Condvar,stop:&AtomicBool){
    // Share the waiter's mutex so stopping cannot race between its predicate
    // check and Condvar::wait, leaving a lost notification.
    let _view=view.lock().unwrap_or_else(|e|e.into_inner());stop.store(true,Ordering::Release);wake.notify_all();
}
fn bounded_join(handle:Option<JoinHandle<()>>){
    if let Some(handle)=handle{let deadline=Instant::now()+Duration::from_secs(1);
        while !handle.is_finished()&&Instant::now()<deadline{thread::sleep(Duration::from_millis(5));}
        if handle.is_finished(){let _=handle.join();}
        // A malfunctioning native call is detached with its library/context
        // still owned by that thread. It cannot hold the IO thread or sockets.
    }
}
impl Mux {
    pub fn start(source_socket:&Path,live_socket:&Path,library:&Path,font:&Path)->Result<Self>{
        let delay=Duration::ZERO;
        #[cfg(test)]let delay=std::env::var("CARLINK_MENU_TEST_RENDER_DELAY_MS").ok().and_then(|s|s.parse::<u64>().ok()).map(|ms|Duration::from_millis(ms.min(2000))).unwrap_or(delay);
        Self::start_inner(source_socket,live_socket,library,font,delay)
    }
    fn start_inner(source_socket:&Path,live_socket:&Path,library:&Path,font:&Path,delay:Duration)->Result<Self>{
        if source_socket==live_socket || !source_socket.is_absolute() || !live_socket.is_absolute(){return Err("invalid menu socket paths".into());}
        // Never unlink an existing path: another owner may be serving it.
        let listener=UnixListener::bind(source_socket).map_err(|_|"cannot bind menu source socket")?;
        if fs::set_permissions(source_socket,fs::Permissions::from_mode(0o600)).is_err() || listener.set_nonblocking(true).is_err(){let _=fs::remove_file(source_socket);return Err("cannot secure menu source socket".into());}
        let view=Arc::new(Mutex::new(View::default()));let stop=Arc::new(AtomicBool::new(false));let wake=Arc::new(Condvar::new());let rendered=Arc::new(Mutex::new(None));
        let (send,events)=mpsc::sync_channel(32);let (ready_tx,ready_rx)=mpsc::sync_channel(1);
        let rv=view.clone();let rs=stop.clone();let rw=wake.clone();let result=rendered.clone();let library=library.to_path_buf();let font=font.to_path_buf();
        let renderer=thread::spawn(move||render_loop(library,font,rv,rw,rs,result,ready_tx,delay));
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(()))=>{},
            result=>{request_stop(&view,&wake,&stop);bounded_join(Some(renderer));drop(listener);let _=fs::remove_file(source_socket);
                return Err(match result{Ok(Err(e))=>e,_=>"menu renderer initialization timed out".into()});}
        }
        let tv=view.clone();let ts=stop.clone();let tw=wake.clone();let input=source_socket.to_path_buf();let output=live_socket.to_path_buf();
        let handle=thread::spawn(move||{
            let exit_view=tv.clone();Worker::new(listener,output,tv,ts.clone(),send,rendered).run();
            request_stop(&exit_view,&tw,&ts);let _=fs::remove_file(input);
        });
        Ok(Self{view,wake,events,stop,thread:Some(handle),renderer:Some(renderer)})
    }
    pub fn set_menu(&self,rows:Vec<String>,focus:usize,visible:bool)->Result<()>{
        if rows.is_empty() || rows.len()>24 || focus>=rows.len() || rows.iter().any(|s|s.chars().count()>128 || s.chars().any(char::is_control)){return Err("invalid menu model".into());}
        let mut view=self.view.lock().map_err(|_|"menu state unavailable")?;
        if view.rows!=rows || view.focus!=focus || view.visible!=visible {view.revision=view.revision.wrapping_add(1);view.rows=rows;view.focus=focus;view.visible=visible;self.wake.notify_one();}Ok(())
    }
    pub fn try_input(&self)->Option<Value>{self.events.try_recv().ok()}
}
impl Drop for Mux {fn drop(&mut self){request_stop(&self.view,&self.wake,&self.stop);bounded_join(self.thread.take());bounded_join(self.renderer.take());}}
fn render_loop(library:PathBuf,font:PathBuf,view:Arc<Mutex<View>>,wake:Arc<Condvar>,stop:Arc<AtomicBool>,output:RenderSlot,ready:SyncSender<Result<()>>,delay:Duration){
    // Load/create, render and destroy all native pointers in this thread only.
    let mut renderer=match Renderer::load(&library,&font){Ok(r)=>r,Err(e)=>{let _=ready.send(Err(e));return;}};
    if ready.send(Ok(())).is_err(){return;}
    let mut processed=None;let mut cache:Option<(Vec<String>,usize,Vec<u8>)>=None;
    loop{
        let next={let mut current=view.lock().unwrap_or_else(|e|e.into_inner());loop{
            if stop.load(Ordering::Acquire){return;}
            if current.visible&&processed!=Some(current.revision){break current.clone();}
            current=wake.wait(current).unwrap_or_else(|e|e.into_inner());
        }};
        processed=Some(next.revision);
        let reused=cache.as_ref().and_then(|(rows,focus,data)|if rows==&next.rows&&*focus==next.focus{Some(data.clone())}else{None});
        let result=if let Some(data)=reused{Ok(data)}else{
            let until=Instant::now()+delay;
            while Instant::now()<until&&!stop.load(Ordering::Acquire){thread::sleep(until.saturating_duration_since(Instant::now()).min(Duration::from_millis(5)));}
            if stop.load(Ordering::Acquire){return;}
            let fresh={let current=view.lock().unwrap_or_else(|e|e.into_inner());current.visible&&current.revision==next.revision};
            if !fresh{continue;}
            renderer.frame(&next)
        };
        if let Ok(data)=&result{cache=Some((next.rows.clone(),next.focus,data.clone()));}
        let current=view.lock().unwrap_or_else(|e|e.into_inner());
        if !stop.load(Ordering::Acquire)&&current.visible&&current.revision==next.revision{*output.lock().unwrap_or_else(|e|e.into_inner())=Some((next.revision,result));}
    }
}

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
struct Pending {data:Vec<u8>,offset:usize,kind:u8,queued_at:Instant}
struct Pipe {socket:UnixStream,input:Vec<u8>,output:VecDeque<Pending>,queued:usize}
impl Pipe {
    fn new(socket:UnixStream)->Result<Self>{socket.set_nonblocking(true).map_err(|_|"mux socket setup failed")?;
        let bytes:libc::c_int=32768;let size=std::mem::size_of_val(&bytes)as libc::socklen_t;
        if unsafe{libc::setsockopt(socket.as_raw_fd(),libc::SOL_SOCKET,libc::SO_SNDBUF,&bytes as *const _ as *const c_void,size)}!=0{return Err("mux send buffer setup failed".into());}
        Ok(Self{socket,input:vec![],output:VecDeque::new(),queued:0})}
    fn push(&mut self,frame:Frame)->Result<()>{let data=frame.wire();if self.queued+data.len()>MAX_QUEUE || self.output.len()>=96{return Err("mux peer is too slow".into());}self.queued+=data.len();self.output.push_back(Pending{data,offset:0,kind:frame.kind,queued_at:Instant::now()});Ok(())}
    fn discard_media(&mut self){self.output.retain(|p|p.offset>0);self.queued=self.output.iter().map(|p|p.data.len()-p.offset).sum();}
    fn discard_video(&mut self){self.output.retain(|p|p.kind!=2 || p.offset>0);self.queued=self.output.iter().map(|p|p.data.len()-p.offset).sum();}
    fn video_backlog(&self)->bool{self.output.iter().filter(|p|p.kind==2).count()>=4 || self.output.iter().any(|p|p.kind==2&&p.offset==0&&p.queued_at.elapsed()>Duration::from_millis(80))}
    fn trim_audio(&mut self){
        let mut keep=0;let mut drop=Vec::new();
        for (i,p) in self.output.iter().enumerate().rev(){if p.kind==3&&p.offset==0{keep+=1;if keep>=6 || p.queued_at.elapsed()>Duration::from_millis(80){drop.push(i);}}}
        for i in drop {self.output.remove(i);}self.queued=self.output.iter().map(|p|p.data.len()-p.offset).sum();
    }
    fn extract(&mut self,reverse:bool,result:&mut Vec<Frame>)->Result<()>{
        while self.input.len()>=4 && result.len()<64 {
            let n=u32::from_le_bytes(self.input[..4].try_into().unwrap())as usize;
            if !(12..=MAX_FRAME).contains(&n){return Err("invalid mux frame length".into());}
            if self.input.len()<n+4{break;}
            let body=&self.input[4..n+4];let f=Frame{kind:body[0],flags:body[1],pts:u64::from_le_bytes(body[4..12].try_into().unwrap()),payload:body[12..].to_vec()};
            if body[2..4]!=[0,0] || !valid(&f,reverse){return Err("invalid mux frame".into());}result.push(f);self.input.drain(..n+4);
        }Ok(())
    }
    fn pump(&mut self,reverse:bool)->Result<Vec<Frame>>{
        let mut result=vec![];
        for round in 0..=8 {
            // Parse leftovers before reading; a prior 64-frame batch may have
            // left complete frames even when the socket currently WouldBlock.
            self.extract(reverse,&mut result)?;if result.len()>=64 || round==8{break;}
            let mut bytes=[0u8;16384];match self.socket.read(&mut bytes){
                Ok(0)=>return Err("mux peer disconnected".into()),Ok(n)=>{self.input.extend(&bytes[..n]);if self.input.len()>MAX_FRAME+4{return Err("mux receive bound exceeded".into());}},
                Err(e) if e.kind()==ErrorKind::WouldBlock=>break,Err(e) if e.kind()==ErrorKind::Interrupted=>continue,Err(_)=>return Err("mux read failed".into())}
        }
        for _ in 0..16 {
            let Some(p)=self.output.front_mut()else{break};
            // Never cut a written frame. A peer stalled in the middle of one
            // is disconnected, so recovery starts with a new framed stream.
            if p.queued_at.elapsed()>Duration::from_millis(300){return Err("mux write stalled".into());}
            match self.socket.write(&p.data[p.offset..]){
                Ok(0)=>return Err("mux write disconnected".into()),Ok(n)=>{p.offset+=n;self.queued-=n;if p.offset==p.data.len(){self.output.pop_front();}},
                Err(e) if e.kind()==ErrorKind::WouldBlock=>break,Err(e) if e.kind()==ErrorKind::Interrupted=>continue,Err(_)=>return Err("mux write failed".into())}
        }Ok(result)
    }
}
fn valid(f:&Frame,reverse:bool)->bool{
    if reverse{return f.flags==0 && (f.kind==129 && f.payload.len()<=16384 && serde_json::from_slice::<Value>(&f.payload).is_ok()
        ||f.kind==130&&(10..=16392).contains(&f.payload.len())&&f.payload.len()%2==0);}
    match f.kind {1=>f.flags==0&&f.payload==configure().payload,2=>f.flags<=3&&!f.payload.is_empty()&&f.payload.len()<=MENU_BYTES&&(f.payload.starts_with(&[0,0,1])||f.payload.starts_with(&[0,0,0,1])),
        3=>f.flags<=3&&!f.payload.is_empty()&&f.payload.len()<=65536&&f.payload.len()%4==0,4=>f.flags<=1&&f.payload.is_empty(),5=>f.flags<=3&&f.payload.len()==1&&f.payload[0]<=1,6=>f.flags==0&&f.payload.is_empty(),
        7=>f.flags==0&&f.payload.len()<=1024&&serde_json::from_slice::<Value>(&f.payload).is_ok(),_=>false}
}
fn connect_output(path:&Path)->std::io::Result<UnixStream>{
    // AF_UNIX connect can otherwise block indefinitely when a stale listener's
    // accept queue is full. Keep retry and shutdown budgets fully bounded.
    let bytes=path.as_os_str().as_bytes();let mut address:libc::sockaddr_un=unsafe{std::mem::zeroed()};
    if bytes.len()>=address.sun_path.len(){return Err(std::io::Error::new(ErrorKind::InvalidInput,"mux output path too long"));}
    address.sun_family=libc::AF_UNIX as libc::sa_family_t;
    for (dst,src) in address.sun_path.iter_mut().zip(bytes){*dst=*src as c_char;}
    let fd=unsafe{libc::socket(libc::AF_UNIX,libc::SOCK_STREAM|libc::SOCK_NONBLOCK|libc::SOCK_CLOEXEC,0)};
    if fd<0{return Err(std::io::Error::last_os_error());}
    if unsafe{libc::connect(fd,&address as *const _ as *const libc::sockaddr,std::mem::size_of::<libc::sockaddr_un>()as libc::socklen_t)}!=0{
        let error=std::io::Error::last_os_error();unsafe{libc::close(fd);}return Err(error);
    }
    Ok(unsafe{UnixStream::from_raw_fd(fd)})
}
struct Worker {listener:UnixListener,output:PathBuf,view:Arc<Mutex<View>>,stop:Arc<AtomicBool>,events:SyncSender<Value>,rendered:RenderSlot,cached:Option<(u64,Vec<u8>)>,
    source:Option<Pipe>,hu:Option<Pipe>,snapshot:View,source_active:bool,source_key:Option<Frame>,waiting_key:bool,retry:Instant,painted:Option<Instant>,presented:Option<View>,touch:Option<Value>,menu_touch:Option<(usize,f64,f64,bool)>,last_key:Option<Instant>,back_frames:Vec<Frame>,back_last:Option<Instant>,back_first:Option<Instant>}
impl Worker {
    fn new(listener:UnixListener,output:PathBuf,view:Arc<Mutex<View>>,stop:Arc<AtomicBool>,events:SyncSender<Value>,rendered:RenderSlot)->Self{
        Self{listener,output,view,stop,events,rendered,cached:None,source:None,hu:None,snapshot:View::default(),source_active:false,source_key:None,waiting_key:true,retry:Instant::now(),painted:None,presented:None,touch:None,menu_touch:None,last_key:None,back_frames:vec![],back_last:None,back_first:None}
    }
    fn output(&mut self,f:Frame){if self.hu.as_mut().is_some_and(|p|p.push(f).is_err()){self.hu=None;self.painted=None;self.presented=None;self.waiting_key=true;}}
    fn source(&mut self,f:Frame){if self.source.as_mut().is_some_and(|p|p.push(f).is_err()){self.lose_source();}}
    fn lose_source(&mut self){self.back_frames.clear();self.back_last=None;self.back_first=None;let _=self.events.try_send(json!({"type":"source-state","active":false}));self.source=None;self.source_active=false;self.source_key=None;self.waiting_key=true;self.touch=None;self.last_key=None;self.stop_source_media();}
    fn stop_source_media(&mut self){
        if let Some(h)=self.hu.as_mut(){h.discard_media();}self.painted=None;
        for role in 0..4{self.output(Frame::new(5,role,vec![0]));}
        if !self.snapshot.visible{self.output(Frame::new(4,0,vec![]));}
    }
    fn request_key(&mut self){if self.last_key.is_none_or(|t|t.elapsed()>=Duration::from_millis(250)){self.last_key=Some(Instant::now());self.source(keyframe());}}
    fn replay_source_key(&mut self){
        // Only reuse a frame if it is still the latest source picture. Any P
        // frame invalidates this slot, so no missing prediction chain replays.
        if self.source_active&&!self.snapshot.visible{if let Some(key)=&self.source_key{let frame=menu_frame(key.payload.clone());self.output(frame);self.waiting_key=false;}}
    }
    fn state(&mut self){self.output(configure());self.output(Frame::new(4,u8::from(self.snapshot.visible||self.source_active),vec![]));self.waiting_key=true;self.request_key();self.replay_source_key();self.painted=None;}
    fn incoming(&mut self,f:Frame){match f.kind {
        1=>{if !self.snapshot.visible {self.output(f);}},
        4=>{self.source_active=f.flags==1;self.source_key=None;let _=self.events.try_send(json!({"type":"source-state","active":self.source_active}));self.waiting_key=true;if !self.source_active{self.stop_source_media();}else if !self.snapshot.visible {self.output(f);}},
        6=>{self.waiting_key=true;self.source_key=None;if !self.snapshot.visible {self.output(f);}},
        2=>{self.source_key=if self.source_active&&f.flags&1==1{Some(f.clone())}else{None};if !self.snapshot.visible && self.source_active {
            if self.waiting_key && f.flags&1==0{return;}
            if self.hu.as_ref().is_some_and(|h|h.video_backlog() || h.queued+f.payload.len()+16>MAX_QUEUE/2){
                if let Some(h)=self.hu.as_mut(){h.discard_video();}self.output(Frame::new(6,0,vec![]));self.waiting_key=true;self.request_key();if f.flags&1==0{return;}
            }
            self.waiting_key=false;self.output(f);
        }},
        3=>{if let Some(h)=self.hu.as_mut(){h.trim_audio();}self.output(f);},
        _=>self.output(f),
    }}
    fn emit_menu(&self,mut v:Value){
        // Bind feedback to the displayed rows. The owner must compare this
        // snapshot before acting, so old input cannot approve a new dialog.
        v["menu_rows"]=if v["type"]=="menu-error"{json!(self.snapshot.rows)}else{self.presented.as_ref().map(|v|json!(v.rows)).unwrap_or_else(||json!([]))};
        let _=self.events.try_send(v);
    }
    fn flush_backs(&mut self){let pending=std::mem::take(&mut self.back_frames);self.back_last=None;self.back_first=None;for f in pending{self.source(f);}}
    fn back_expired(&self,now:Instant)->bool{self.back_last.is_some_and(|t|now.duration_since(t)>=Duration::from_millis(350))||self.back_first.is_some_and(|t|now.duration_since(t)>=Duration::from_millis(900))}
    fn input(&mut self,f:Frame){
        if f.kind==130{self.source(f);return;}
        let Ok(v)=serde_json::from_slice::<Value>(&f.payload)else{return;};
        if !self.snapshot.visible {
            let now=Instant::now();if self.back_expired(now){self.flush_backs();}
            if matches!(v["type"].as_str(),Some("key-press"|"key"))&&v["code"].as_u64()==Some(1){
                self.back_first.get_or_insert(now);self.back_last=Some(now);self.back_frames.push(f);
                if self.back_frames.len()==3{self.back_frames.clear();self.back_last=None;self.back_first=None;let _=self.events.try_send(json!({"type":"open-menu"}));}return;
            }
            if !self.back_frames.is_empty()&&matches!(v["type"].as_str(),Some("touch"|"key-press"|"key"|"focus")){self.flush_backs();}
        }
        if self.snapshot.visible {
            if v["type"]=="keyframe" {self.painted=None;}else if v["type"]=="touch" {self.menu_touch(v);}else if matches!(v["type"].as_str(),Some("touch"|"key-press"|"key"|"focus")){
                self.emit_menu(v);
            }
        }else{if v["type"]=="touch"{self.touch=Some(v);}self.source(f);}
    }
    fn menu_touch(&mut self,v:Value){
        let Some(c)=v["contacts"].as_array().and_then(|c|c.first())else{self.menu_touch=None;return};
        let (Some(x),Some(y),Some(down))=(c["x"].as_f64(),c["y"].as_f64(),c["down"].as_bool())else{self.menu_touch=None;return};
        if !x.is_finite() || !y.is_finite() || !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y){self.menu_touch=None;return;}
        let Some(displayed)=self.presented.as_ref().filter(|v|v.rows==self.snapshot.rows)else{self.menu_touch=None;return};
        let row=touch_row(x,y,displayed.focus,displayed.rows.len());
        if down {
            if let Some((_,px,py,moved))=self.menu_touch.as_mut(){if (x-*px).abs()*800.0>12.0 || (y-*py).abs()*480.0>12.0{*moved=true;}}
            else if let Some(row)=row {self.menu_touch=Some((row,x,y,false));}
        }else if let Some((pressed,_,_,moved))=self.menu_touch.take(){
            if let Some(row)=row {let op=if moved{"menu-focus"}else{"menu-select"};if moved || row==pressed {self.emit_menu(json!({"type":op,"index":row}));}}
        }
    }
    fn update(&mut self){
        let view=match self.view.lock(){Ok(v)=>v.clone(),Err(_)=>return};
        if view.revision==self.snapshot.revision{return;}
        let changed=view.visible!=self.snapshot.visible;let model_changed=view.rows!=self.snapshot.rows;self.snapshot=view;self.cached=None;
        if let Some(h)=self.hu.as_mut(){h.discard_video();}
        if model_changed{self.menu_touch=None;}
        if changed {
            self.back_frames.clear();self.back_last=None;self.back_first=None;
            self.presented=None;
            self.menu_touch=None;
            if let Some(h)=self.hu.as_mut(){h.discard_video();}
            if self.snapshot.visible {
                if let Some(mut v)=self.touch.take(){if let Some(contacts)=v["contacts"].as_array_mut(){for c in contacts{c["down"]=Value::Bool(false);}}self.source(feedback(v));}
            }
            self.output(Frame::new(6,0,vec![]));self.output(Frame::new(4,u8::from(self.snapshot.visible||self.source_active),vec![]));
            self.waiting_key=true;self.request_key();
            self.replay_source_key();
        }
        self.painted=None;
    }
    fn take_rendered(&mut self){
        let result={let mut slot=self.rendered.lock().unwrap_or_else(|e|e.into_inner());
            if slot.as_ref().is_some_and(|(revision,_)|*revision<=self.snapshot.revision){slot.take()}else{None}};
        if let Some((revision,result))=result{if self.snapshot.visible&&revision==self.snapshot.revision{
            match result{Ok(data)=>{self.cached=Some((revision,data));self.painted=None;},Err(_)=>self.emit_menu(json!({"type":"menu-error"}))}
        }}
    }
    fn run(&mut self){
        while !self.stop.load(Ordering::Acquire){
            self.update();if self.back_expired(Instant::now()){self.flush_backs();}
            if self.source.is_none(){match self.listener.accept(){Ok((s,_))=>{self.source=Pipe::new(s).ok();self.source_active=false;self.waiting_key=true;self.last_key=None;self.request_key();
                if !self.snapshot.visible{self.output(configure());self.output(Frame::new(6,0,vec![]));self.output(Frame::new(4,0,vec![]));}},Err(e) if e.kind()==ErrorKind::WouldBlock=>{},Err(_)=>break}}
            if self.hu.is_none() && Instant::now()>=self.retry {self.retry=Instant::now()+Duration::from_millis(200);if let Ok(s)=connect_output(&self.output){self.hu=Pipe::new(s).ok();if self.hu.is_some(){self.state();}}}
            if let Some(p)=self.source.as_mut(){match p.pump(false){Ok(frames)=>{for f in frames{self.incoming(f);}},Err(_)=>self.lose_source()}}
            if let Some(p)=self.hu.as_mut(){match p.pump(true){Ok(frames)=>{for f in frames{self.input(f);}},Err(_)=>{self.hu=None;self.painted=None;self.presented=None;self.waiting_key=true;}}}
            self.take_rendered();
            // A static view reuses its independent IDR. Only its monotonic PTS
            // changes; no native call can block audio/control IO here.
            if self.hu.is_some() && self.snapshot.visible && self.painted.is_none_or(|t|t.elapsed()>=Duration::from_secs(1)) {
                if let Some((revision,data))=&self.cached{if *revision==self.snapshot.revision{self.output(menu_frame(data.clone()));self.painted=Some(Instant::now());if self.hu.is_some(){self.presented=Some(self.snapshot.clone());}}}
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
    #[test]fn static_key_replays_only_while_it_is_latest_source_picture(){
        let root=std::env::temp_dir().join(format!("carlink-static-key-{}",std::process::id()));fs::create_dir(&root).unwrap();
        let listener=UnixListener::bind(root.join("source")).unwrap();let(tx,_rx)=mpsc::sync_channel(32);
        let mut w=Worker::new(listener,root.join("live"),Arc::new(Mutex::new(View::default())),Arc::new(AtomicBool::new(false)),tx,Arc::new(Mutex::new(None)));
        w.snapshot.visible=true;w.incoming(Frame::new(4,1,vec![]));w.incoming(Frame::new(2,1,vec![1,2,3]));assert!(w.source_key.is_some());
        let(socket,_peer)=UnixStream::pair().unwrap();w.hu=Some(Pipe::new(socket).unwrap());w.snapshot.visible=false;w.replay_source_key();
        assert!(!w.waiting_key);assert_eq!(w.hu.as_ref().unwrap().output.len(),1);
        w.incoming(Frame::new(2,0,vec![4]));assert!(w.source_key.is_none());
        w.incoming(Frame::new(2,1,vec![5]));w.incoming(Frame::new(6,0,vec![]));assert!(w.source_key.is_none());
        w.incoming(Frame::new(2,1,vec![6]));w.incoming(Frame::new(4,0,vec![]));assert!(w.source_key.is_none());
        drop(w);fs::remove_dir_all(root).unwrap();
    }
    #[test]fn touch_maps_visible_scroll_rows(){assert_eq!(touch_row(0.5,0.2,0,3),Some(0));assert_eq!(touch_row(0.5,0.2,8,15),Some(2));assert_eq!(touch_row(0.01,0.2,0,3),None);assert_eq!(touch_row(0.5,0.95,0,3),None);}
    #[test]fn triple_back_is_consumed_but_single_back_and_order_are_preserved(){
        let root=std::env::temp_dir().join(format!("carlink-back-{}",std::process::id()));fs::create_dir(&root).unwrap();
        let listener=UnixListener::bind(root.join("source")).unwrap();let (tx,rx)=mpsc::sync_channel(32);
        let mut w=Worker::new(listener,root.join("live"),Arc::new(Mutex::new(View::default())),Arc::new(AtomicBool::new(false)),tx,Arc::new(Mutex::new(None)));
        let (socket,_peer)=UnixStream::pair().unwrap();w.source=Some(Pipe::new(socket).unwrap());
        for _ in 0..3{w.input(feedback(json!({"type":"key-press","code":1})));}
        assert_eq!(rx.try_recv().unwrap()["type"],"open-menu");assert!(w.source.as_ref().unwrap().output.is_empty());
        w.input(feedback(json!({"type":"key-press","code":1})));w.input(keyframe());
        w.input(feedback(json!({"type":"key-press","code":1})));w.input(feedback(json!({"type":"key-press","code":1})));
        assert_eq!(rx.try_recv().unwrap()["type"],"open-menu");assert_eq!(w.source.as_ref().unwrap().output.len(),1);
        w.source.as_mut().unwrap().output.clear();w.source.as_mut().unwrap().queued=0;

        w.input(feedback(json!({"type":"key-press","code":1})));assert_eq!(w.back_frames.len(),1);
        assert!(w.back_expired(Instant::now()+Duration::from_millis(351)));w.flush_backs();assert_eq!(w.source.as_ref().unwrap().output.len(),1);
        w.input(feedback(json!({"type":"key-press","code":1})));w.input(feedback(json!({"type":"key-press","code":2})));
        assert!(w.back_frames.is_empty());assert_eq!(w.source.as_ref().unwrap().output.len(),3);
        w.input(feedback(json!({"type":"key-press","code":1})));w.lose_source();assert!(w.back_frames.is_empty());assert_eq!(rx.try_recv().unwrap()["type"],"source-state");
        drop(w);fs::remove_file(root.join("source")).unwrap();fs::remove_dir(root).unwrap();
    }
    #[test]fn rejects_bad_media(){assert!(valid(&configure(),false));assert!(!valid(&Frame::new(3,0,vec![1,2,3]),false));assert!(!valid(&Frame::new(129,0,b"{}".to_vec()),false));assert!(valid(&feedback(json!({"type":"keyframe"})),true));assert!(!valid(&Frame::new(2,1,vec![1,2,3]),false));}
    #[test]fn partial_video_is_never_removed(){let(a,_)=UnixStream::pair().unwrap();let mut p=Pipe::new(a).unwrap();p.push(Frame::new(2,1,vec![0,0,0,1,5])).unwrap();p.output.front_mut().unwrap().offset=2;p.queued-=2;p.push(Frame::new(2,1,vec![0,0,0,1,5])).unwrap();p.push(Frame::new(3,0,vec![0;4])).unwrap();p.discard_video();assert_eq!(p.output.len(),2);assert_eq!(p.output.front().unwrap().offset,2);assert_eq!(p.queued,p.output.iter().map(|p|p.data.len()-p.offset).sum::<usize>());}
    #[test]fn cached_batch_is_parsed_without_new_socket_input(){
        let(a,_peer)=UnixStream::pair().unwrap();let mut p=Pipe::new(a).unwrap();
        for _ in 0..100{p.input.extend(Frame::new(4,1,vec![]).wire());}
        assert_eq!(p.pump(false).unwrap().len(),64);assert_eq!(p.pump(false).unwrap().len(),36);assert!(p.input.is_empty());
    }
    #[test]fn congested_audio_is_bounded_and_partial_media_kept(){
        let(a,_peer)=UnixStream::pair().unwrap();let mut p=Pipe::new(a).unwrap();
        for _ in 0..12{p.push(Frame::new(3,0,vec![0;128])).unwrap();}
        p.output.front_mut().unwrap().offset=3;p.queued-=3;p.trim_audio();
        assert_eq!(p.output.len(),6);assert_eq!(p.output.front().unwrap().offset,3);
        p.output.front_mut().unwrap().queued_at=Instant::now()-Duration::from_millis(301);
        assert!(p.pump(false).is_err());
        p.discard_media();assert_eq!(p.output.len(),1);assert_eq!(p.output.front().unwrap().offset,3);
    }
    #[test]fn full_output_accept_queue_never_blocks_connect(){
        let path=std::env::temp_dir().join(format!("carlink-menu-full-{}",std::process::id()));let server=UnixListener::bind(&path).unwrap();assert_eq!(unsafe{libc::listen(server.as_raw_fd(),1)},0);let start=Instant::now();let mut held=vec![];let mut full=false;
        for _ in 0..8{match connect_output(&path){Ok(s)=>held.push(s),Err(_)=>{full=true;break;}}}
        assert!(full);assert!(start.elapsed()<Duration::from_secs(2));drop(held);drop(server);fs::remove_file(path).unwrap();
    }
    #[test]fn render_results_only_apply_to_the_matching_view(){
        let root=std::env::temp_dir().join(format!("carlink-menu-revision-{}",std::process::id()));fs::create_dir(&root).unwrap();
        let path=root.join("source");let listener=UnixListener::bind(&path).unwrap();let slot:RenderSlot=Arc::new(Mutex::new(Some((1,Ok(vec![1])))));
        let(tx,_rx)=mpsc::sync_channel(32);let mut worker=Worker::new(listener,root.join("live"),Arc::new(Mutex::new(View::default())),Arc::new(AtomicBool::new(false)),tx,slot.clone());
        worker.snapshot=View{rows:vec!["new".into()],focus:0,visible:true,revision:2};worker.take_rendered();assert!(worker.cached.is_none());
        *slot.lock().unwrap()=Some((3,Ok(vec![3])));worker.take_rendered();assert!(worker.cached.is_none());assert!(slot.lock().unwrap().is_some());
        worker.snapshot.revision=3;worker.take_rendered();assert_eq!(worker.cached.as_ref().unwrap().1,vec![3]);assert!(slot.lock().unwrap().is_none());
        drop(worker);fs::remove_file(path).unwrap();fs::remove_dir(root).unwrap();
    }
    #[test]fn renderer_initialization_failure_releases_source_socket(){
        let root=std::env::temp_dir().join(format!("carlink-menu-invalid-{}",std::process::id()));fs::create_dir(&root).unwrap();let source=root.join("source");
        assert!(Mux::start(&source,&root.join("live"),&root.join("missing-library"),&root.join("missing-font")).is_err());
        assert!(!source.exists());fs::remove_dir(root).unwrap();
    }
    fn recv(s:&mut UnixStream)->Frame {let mut h=[0u8;4];s.read_exact(&mut h).unwrap();let n=u32::from_le_bytes(h)as usize;assert!((12..=MAX_FRAME).contains(&n));let mut b=vec![0;n];s.read_exact(&mut b).unwrap();Frame{kind:b[0],flags:b[1],pts:u64::from_le_bytes(b[4..12].try_into().unwrap()),payload:b[12..].to_vec()}}
    fn until(s:&mut UnixStream,kind:u8)->Frame {for _ in 0..40{let f=recv(s);if f.kind==kind{return f;}}panic!("expected frame kind {kind}");}
    #[test]fn slow_native_render_does_not_hold_pcm_and_static_idr_is_cached(){
        let Ok(library)=std::env::var("CARLINK_MENU_LIBRARY")else{return};let font=std::env::var("CARLINK_MENU_FONT").unwrap();
        let root=std::env::temp_dir().join(format!("carlink-menu-audio-{}",std::process::id()));fs::create_dir(&root).unwrap();let source=root.join("source");let live=root.join("live");
        let mux=Mux::start_inner(&source,&live,Path::new(&library),Path::new(&font),Duration::from_millis(800)).unwrap();
        mux.set_menu(vec!["first view".into()],0,true).unwrap();let server=UnixListener::bind(&live).unwrap();
        let(mut hu,_)=server.accept().unwrap();hu.set_read_timeout(Some(Duration::from_millis(150))).unwrap();
        let mut upstream=UnixStream::connect(&source).unwrap();upstream.write_all(&configure().wire()).unwrap();upstream.write_all(&Frame::new(4,1,vec![]).wire()).unwrap();
        // Thirty PCM writes traverse the actual mux while native work is held
        // for 800ms. An IO-thread renderer would time out on the first write.
        for sequence in 0..30u8{let payload=vec![sequence;256];let before=Instant::now();upstream.write_all(&Frame::new(3,0,payload.clone()).wire()).unwrap();assert_eq!(until(&mut hu,3).payload,payload);assert!(before.elapsed()<Duration::from_millis(200));thread::sleep(Duration::from_millis(10));}
        mux.set_menu(vec!["latest view".into()],0,true).unwrap();hu.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
        let mut first=until(&mut hu,2);let mut heartbeat=until(&mut hu,2);
        for _ in 0..2{if first.payload==heartbeat.payload{break;}first=heartbeat;heartbeat=until(&mut hu,2);}
        assert_eq!(first.payload,heartbeat.payload);assert!(heartbeat.pts>first.pts);
        // Decoder keyframe requests replay the same IDR without encoding.
        hu.write_all(&feedback(json!({"type":"keyframe","screen":0})).wire()).unwrap();hu.set_read_timeout(Some(Duration::from_millis(200))).unwrap();let replay=until(&mut hu,2);assert_eq!(replay.payload,first.payload);
        mux.set_menu(vec!["pending render".into()],0,true).unwrap();thread::sleep(Duration::from_millis(20));let before=Instant::now();
        drop(mux);assert!(before.elapsed()<Duration::from_secs(2));assert!(!source.exists());drop(hu);drop(upstream);drop(server);fs::remove_file(live).unwrap();fs::remove_dir(root).unwrap();
    }
    #[test]fn mux_real_native_contract(){
        let Ok(lib)=std::env::var("CARLINK_MENU_LIBRARY")else{return};let font=std::env::var("CARLINK_MENU_FONT").unwrap();
        let root=std::env::temp_dir().join(format!("carlink-menu-ci-{}",std::process::id()));fs::create_dir(&root).unwrap();
        let source=root.join("source");let live=root.join("live");
        let mux=Mux::start(&source,&live,Path::new(&lib),Path::new(&font)).unwrap();
        mux.set_menu(vec!["\u{8fde}\u{63a5} iPhone".into(),"\u{8fde}\u{63a5}\u{534e}\u{4e3a}".into()],1,true).unwrap();
        // Discovery/input socket exists before HU; the late HU can recover.
        let server=UnixListener::bind(&live).unwrap();let(mut hu,_)=server.accept().unwrap();hu.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let menu=until(&mut hu,2);assert_eq!(menu.flags&1,1);assert!(menu.payload.len()>200);
        // Local menu works while no phone has connected at all.
        for down in [true,false]{hu.write_all(&feedback(json!({"type":"touch","contacts":[{"id":0,"x":0.5,"y":0.2,"down":down}]})).wire()).unwrap();}
        let deadline=Instant::now()+Duration::from_secs(3);loop{if let Some(v)=mux.try_input(){if v["type"]=="source-state"{continue;}assert_eq!(v["type"],"menu-select");assert_eq!(v["index"],0);assert_eq!(v["menu_rows"],json!(["\u{8fde}\u{63a5} iPhone","\u{8fde}\u{63a5}\u{534e}\u{4e3a}"]));break;}assert!(Instant::now()<deadline);thread::sleep(Duration::from_millis(10));}
        let mut upstream=UnixStream::connect(&source).unwrap();upstream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        upstream.write_all(&configure().wire()).unwrap();upstream.write_all(&Frame::new(4,1,vec![]).wire()).unwrap();
        upstream.write_all(&Frame::new(3,0,vec![7;256]).wire()).unwrap();assert_eq!(until(&mut hu,3).payload,vec![7;256]);
        hu.write_all(&feedback(json!({"type":"key-press","code":6})).wire()).unwrap();
        let input_deadline=Instant::now()+Duration::from_secs(3);let input=loop{if let Some(v)=mux.try_input(){if v["type"]=="key-press"{break v;}}assert!(Instant::now()<input_deadline);thread::sleep(Duration::from_millis(10));};assert_eq!(input["code"],6);assert_eq!(input["menu_rows"],json!(["\u{8fde}\u{63a5} iPhone","\u{8fde}\u{63a5}\u{534e}\u{4e3a}"]));
        mux.set_menu(vec!["\u{8fde}\u{63a5} iPhone".into()],0,false).unwrap();thread::sleep(Duration::from_millis(100));
        let key=vec![0,0,0,1,0x65,0xff,0xab,0xcd];upstream.write_all(&Frame::new(2,1,key.clone()).wire()).unwrap();
        let deadline=Instant::now()+Duration::from_secs(5);loop{let f=recv(&mut hu);if f.kind==2 && f.payload==key{break;}assert!(Instant::now()<deadline);}
        hu.write_all(&feedback(json!({"type":"touch","contacts":[{"id":0,"x":0.5,"y":0.4,"down":true}]})).wire()).unwrap();
        loop{let f=recv(&mut upstream);let v:Value=serde_json::from_slice(&f.payload).unwrap();if v["type"]=="touch"{break;}}
        mux.set_menu(vec!["CarLink".into()],0,true).unwrap();
        loop{let f=recv(&mut upstream);let v:Value=serde_json::from_slice(&f.payload).unwrap();if v["type"]=="touch"{assert_eq!(v["contacts"][0]["down"],false);break;}}
        drop(hu);drop(server);thread::sleep(Duration::from_millis(100));fs::remove_file(&live).unwrap();
        let server=UnixListener::bind(&live).unwrap();let(mut hu,_)=server.accept().unwrap();hu.set_read_timeout(Some(Duration::from_secs(5))).unwrap();assert_eq!(until(&mut hu,2).flags&1,1);
        // A replacement source cannot reuse the previous source's references.
        drop(upstream);thread::sleep(Duration::from_millis(80));mux.set_menu(vec!["CarLink".into()],0,false).unwrap();thread::sleep(Duration::from_millis(80));
        let mut upstream=UnixStream::connect(&source).unwrap();upstream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        upstream.write_all(&configure().wire()).unwrap();upstream.write_all(&Frame::new(4,1,vec![]).wire()).unwrap();
        let predicted=vec![0,0,0,1,0x41,0x88];let replacement=vec![0,0,0,1,0x65,0x99];
        upstream.write_all(&Frame::new(2,0,predicted.clone()).wire()).unwrap();upstream.write_all(&Frame::new(2,1,replacement.clone()).wire()).unwrap();
        loop{let f=recv(&mut hu);if f.kind==2{assert_ne!(f.payload,predicted);if f.payload==replacement{break;}}}
        drop(mux);assert!(!source.exists());drop(server);drop(hu);drop(upstream);fs::remove_file(live).unwrap();fs::remove_dir(root).unwrap();
    }
}
}
#[cfg(unix)]pub use unix::Mux;
