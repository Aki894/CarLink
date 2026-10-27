//! R3 lifecycle owner: one persistent CarLife output, one wireless backend.
//! Commands are local/private; backend callbacks carry a generation token.
#[cfg(unix)]
mod linux {
use serde::{Deserialize,Serialize};
use serde_json::{Value,json};
use std::{collections::VecDeque,fs::{self,File,OpenOptions},io::{BufRead,BufReader,Read,Write},
    os::unix::{ffi::OsStrExt,fs::{OpenOptionsExt,PermissionsExt,FileTypeExt,MetadataExt},io::AsRawFd,net::{UnixListener,UnixStream},process::CommandExt},
    path::{Path,PathBuf},process::{Child,Command,Stdio},sync::{atomic::{AtomicBool,Ordering},mpsc::{self,Receiver}},thread,time::{Duration,Instant,SystemTime,UNIX_EPOCH}};
type Result<T>=std::result::Result<T,String>;
static STOP:AtomicBool=AtomicBool::new(false);
extern "C" fn stop(_:i32){STOP.store(true,Ordering::Relaxed);}
#[derive(Clone,Copy,Debug,PartialEq,Eq,Deserialize,Serialize)]
#[serde(rename_all="lowercase")]
pub enum Protocol{Carplay,Hicar}
#[derive(Clone,Deserialize,Serialize)]
pub struct Device{pub protocol:Protocol,pub address:String,pub name:String,#[serde(default="yes")]pub auto:bool,#[serde(default)]pub last_used:u64}
fn yes()->bool{true}
fn mac(s:&str)->bool{let p=s.split(':').collect::<Vec<_>>();p.len()==6&&p.iter().all(|s|s.len()==2&&u8::from_str_radix(s,16).is_ok())}
impl Device{fn validate(&self)->Result<()>{if !mac(&self.address)||self.name.is_empty()||self.name.len()>96||self.name.chars().any(char::is_control){Err("invalid device record".into())}else{Ok(())}}}
#[derive(Deserialize)]
struct Config{schema:u32,socket:PathBuf,live_socket:PathBuf,state:PathBuf,carlife:Vec<String>,carplay:Vec<String>,hicar:Vec<String>,
    #[serde(default)]devices:Vec<Device>,#[serde(default)]initial:Option<Protocol>,#[serde(default)]bluetoothctl:Option<String>,
    #[serde(default)]radio_lock:Option<PathBuf>,
    #[serde(default)]media_socket:Option<PathBuf>,
    #[serde(default)]web_listen:Option<std::net::SocketAddr>,
    #[serde(default)]source_socket:Option<PathBuf>,#[serde(default)]menu_library:Option<PathBuf>,#[serde(default)]menu_font:Option<PathBuf>,
    #[serde(default="connect_seconds")]connect_seconds:u64,#[serde(default="stable_seconds")]stable_seconds:u64,#[serde(default="pair_seconds")]pair_seconds:u64}
fn connect_seconds()->u64{30}fn stable_seconds()->u64{10}fn pair_seconds()->u64{120}
#[derive(Default,Clone,Copy,PartialEq,Deserialize,Serialize)]
#[serde(rename_all="lowercase")]enum Policy{#[default]Last,Favorite,Manual}
#[derive(Default,Clone,Copy,PartialEq,Deserialize,Serialize)]
#[serde(rename_all="lowercase")]enum BootProtocol{#[default]Last,Carplay,Hicar}
#[derive(Default,Deserialize,Serialize)]
struct Store{#[serde(default)]selected_protocol:Option<Protocol>,#[serde(default)]boot_protocol:BootProtocol,#[serde(default)]devices:Vec<Device>,#[serde(default)]last:Option<Device>,#[serde(default)]paused:bool,#[serde(default)]policy:Policy,#[serde(default)]favorite:Option<Device>,#[serde(default)]preferred:Option<Device>,#[serde(default)]boot_id:Option<String>}
fn private_dir(path:&Path)->Result<()>{
    fs::create_dir_all(path).map_err(|_|"cannot create private manager directory")?;
    let meta=fs::symlink_metadata(path).map_err(|_|"cannot inspect private manager directory")?;
    if !meta.is_dir()||meta.file_type().is_symlink(){return Err("manager path is not a directory".into());}
    fs::set_permissions(path,fs::Permissions::from_mode(0o700)).map_err(|_|"cannot secure manager directory".into())
}
// Called only while both manager ownership locks are held. A successful or
// queued connection belongs to a live server and must never be removed.
fn clear_stale_socket(path:&Path)->Result<()>{
    let before=match fs::symlink_metadata(path){Ok(m)=>m,Err(e) if e.kind()==std::io::ErrorKind::NotFound=>return Ok(()),Err(_)=>return Err("cannot inspect runtime socket".into())};
    let uid=unsafe{libc::geteuid()};
    if !before.file_type().is_socket()||before.uid()!=uid{return Err("runtime endpoint is not an owned socket".into());}
    let bytes=path.as_os_str().as_bytes();let mut address:libc::sockaddr_un=unsafe{std::mem::zeroed()};
    if bytes.len()>=address.sun_path.len(){return Err("runtime socket path is too long".into());}
    address.sun_family=libc::AF_UNIX as libc::sa_family_t;
    for (dst,src) in address.sun_path.iter_mut().zip(bytes){*dst=*src as libc::c_char;}
    let fd=unsafe{libc::socket(libc::AF_UNIX,libc::SOCK_STREAM|libc::SOCK_NONBLOCK|libc::SOCK_CLOEXEC,0)};
    if fd<0{return Err("cannot inspect runtime socket owner".into());}
    let connected=unsafe{libc::connect(fd,&address as *const _ as *const libc::sockaddr,std::mem::size_of::<libc::sockaddr_un>()as libc::socklen_t)};
    let error=std::io::Error::last_os_error();unsafe{libc::close(fd);}
    if connected==0||error.raw_os_error()!=Some(libc::ECONNREFUSED){return Err("runtime socket has an active or unverified owner".into());}
    let after=fs::symlink_metadata(path).map_err(|_|"runtime endpoint changed during inspection")?;
    if !after.file_type().is_socket()||after.uid()!=uid||after.dev()!=before.dev()||after.ino()!=before.ino(){return Err("runtime endpoint changed during inspection".into());}
    fs::remove_file(path).map_err(|_|"cannot remove inactive runtime socket".into())
}
struct SocketPath{path:PathBuf,dev:u64,ino:u64,uid:u32}
impl SocketPath{fn owned(path:&Path)->Result<Self>{let m=fs::symlink_metadata(path).map_err(|_|"cannot inspect bound runtime socket")?;
    if !m.file_type().is_socket()||m.uid()!=unsafe{libc::geteuid()}{return Err("bound runtime socket ownership changed".into());}
    Ok(Self{path:path.into(),dev:m.dev(),ino:m.ino(),uid:m.uid()})}}
impl Drop for SocketPath{fn drop(&mut self){if fs::symlink_metadata(&self.path).is_ok_and(|m|m.file_type().is_socket()&&m.uid()==self.uid&&m.dev()==self.dev&&m.ino()==self.ino){let _=fs::remove_file(&self.path);}}}
fn private_read(path:&Path)->Result<Vec<u8>>{
    let mut f=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path).map_err(|_|"cannot read private manager file")?;
    let meta=f.metadata().map_err(|_|"cannot stat manager file")?;
    if !meta.is_file()||meta.permissions().mode()&0o077!=0||meta.len()>65536{return Err("manager file must be a private regular file".into());}
    let mut bytes=vec![];std::io::Read::by_ref(&mut f).take(65537).read_to_end(&mut bytes).map_err(|_|"manager read failed")?;if bytes.len()>65536{return Err("manager file too large".into());}Ok(bytes)
}
fn save(path:&Path,store:&Store)->Result<()>{
    let parent=path.parent().ok_or("state parent missing")?;private_dir(parent)?;
    let tmp=path.with_extension(format!("tmp-{}",std::process::id()));let mut f=OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp).map_err(|_|"cannot create manager state transaction")?;
    let result=(||{serde_json::to_writer(&mut f,store).map_err(|_|"cannot serialize manager state")?;f.sync_all().map_err(|_|"cannot sync manager state")?;
        fs::rename(&tmp,path).map_err(|_|"cannot publish manager state")?;File::open(parent).and_then(|f|f.sync_all()).map_err(|_|"cannot sync manager state directory")?;Ok(())})();
    if result.is_err(){let _=fs::remove_file(&tmp);}result
}
#[derive(Clone,Serialize)]
struct Status{schema:u32,generation:u64,default_protocol:BootProtocol,protocol:Option<Protocol>,target:Option<Device>,active:bool,pairing:bool,paused:bool,call_active:bool,
    carlife_pid:Option<u32>,backend_pid:Option<u32>,usb_connected:bool,menu_visible:bool,last_error:Option<String>,last_stage:Option<String>}
fn event(name:&str){println!("{}",json!({"event":name}));}
fn diagnostic_stage(name:&str)->bool{matches!(name,"bluetooth_reconnect_ready"|"bluetooth_reconnect_failed"|"ready"|"controller_setting"|"rfcomm_connected"|"rust_connection_worker_ready"|"rust_hisight_ready"|"rust_dmsdp_ready"|"rust_media_ready"|"rust_radio_ready"|"tcp_connected"|"hisight_connected"|"auth_parse_stopped"|"control_parse_stopped"|"control_peer_timeout"|"hisight_parse_stopped"|"parse_stopped"|"stream_error"|"tcp_stream_error"|"hisight_error"|"stopped")}
fn stage_summary(v:&Value)->Option<Value>{
    let stage=v["event"].as_str().filter(|s|diagnostic_stage(s))?;
    // Reconstruct, never forward the source object or its dynamic fields.
    let mut summary=json!({"event":"manager_backend_stage","stage":stage});
    if let Some(epoch)=v["session_epoch"].as_u64(){summary["session_epoch"]=epoch.into();}Some(summary)
}
fn stage_report(generation:u64,current:u64,v:&Value)->Option<Value>{
    if generation!=current||v["event"]!="manager_backend_stage"{return None;}
    let stage=v["stage"].as_str().filter(|s|diagnostic_stage(s))?;
    Some(json!({"event":"manager_backend_stage","generation":generation,"stage":stage}))
}
struct Process{child:Child,pgid:i32,events:Receiver<Value>,generation:u64}
fn spawn(args:&[String],generation:u64,source:Option<&Path>)->Result<Process>{
    if args.is_empty(){return Err("backend command is empty".into());}
    let mut cmd=Command::new(&args[0]);cmd.args(&args[1..]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::inherit());
    unsafe{cmd.pre_exec(||if libc::setpgid(0,0)==0{Ok(())}else{Err(std::io::Error::last_os_error())});}
    if let Some(path)=source{cmd.env("CARLINK_LIVE_SOCKET",path);}
    let mut child=cmd.spawn().map_err(|_|"cannot start projection process")?;let pgid=child.id() as i32;
    let out=child.stdout.take().ok_or("missing child event stream")?;let (tx,rx)=mpsc::sync_channel(64);
    thread::spawn(move||{
        let mut reader=BufReader::new(out);
        loop{let mut line=vec![];let read=std::io::Read::by_ref(&mut reader).take(16385).read_until(b'\n',&mut line);
            if !matches!(read,Ok(n) if n>0){break;}if line.len()>16384{break;}
            if let Ok(v)=serde_json::from_slice::<Value>(&line){
                if let Some(summary)=stage_summary(&v){let _=tx.try_send(summary);}
                if matches!(v["event"].as_str(),Some("stage"|"retry"|"forward_latency"|"bridge_stats"|"hisight_closed"|"target_selected"|"session_error"|"voice_stats"|"hicar_audio_service"|"microphone_error"))
                    ||v["event"]=="carlife_state"&&v["detail"]=="AwaitingVideoSource"{
                    let _=tx.try_send(v);
                }
            }
        }
    });Ok(Process{child,pgid,events:rx,generation})
}
impl Process{
    fn end(&mut self)->Result<()>{
        if self.pgid<=0{return Ok(());}
        unsafe{libc::kill(-self.pgid,libc::SIGTERM);}
        let deadline=Instant::now()+Duration::from_secs(8);
        loop{
            let exited=self.child.try_wait().map_err(|_|"cannot reap projection process")?.is_some();
            if exited&&unsafe{libc::kill(-self.pgid,0)}!=0{self.pgid=0;return Ok(());}
            if Instant::now()>=deadline{break;}thread::sleep(Duration::from_millis(20));
        }
        unsafe{libc::kill(-self.pgid,libc::SIGKILL);}
        self.child.wait().map_err(|_|"cannot reap stopped projection process")?;
        // Killed descendants can remain zombies briefly; they own no radio FDs.
        self.pgid=0;Ok(())
    }
}
impl Drop for Process{fn drop(&mut self){let _=self.end();}}
#[derive(Clone)]struct Choice{protocol:Protocol,device:Option<Device>,pairing:bool}
struct PairPin(String);
impl Drop for PairPin{fn drop(&mut self){unsafe{crate::hicar::crypto::wipe(self.0.as_bytes_mut());}}}
struct Owner{config:Config,store:Store,status:Status,life:Option<Process>,backend:Option<Process>,choice:Option<Choice>,
    life_started:Instant,life_retry_at:Instant,life_failures:u8,life_ready:bool,last_media_report:Option<Instant>,
    started:Instant,recovery_deadline:Option<Instant>,ready:Option<Instant>,seen_media:bool,committed:bool,pair_deadline:Option<Instant>,repair_used:bool,
    retry_at:Instant,tried:VecDeque<String>,last_auth:Option<Instant>,refusals:u8,mux:Option<crate::manager_menu::Mux>,menu_focus:usize,menu_pending:Option<Value>,menu_idle:bool,wireless_ready:bool,source_ready:bool,session_epoch:u64,closed_hicar_epoch:Option<u64>,source_floor:u64,bound_source_id:u64,pair_pin:Option<PairPin>}
impl Owner{
    fn new(config:Config)->Result<Self>{
        if config.schema!=1||config.devices.len()>8||!(2..=120).contains(&config.connect_seconds)||!(1..=60).contains(&config.stable_seconds)||!(5..=120).contains(&config.pair_seconds){return Err("unsupported manager configuration".into());}
        for path in [&config.socket,&config.live_socket,&config.state]{if !path.is_absolute(){return Err("manager paths must be absolute".into());}}
        for args in [&config.carlife,&config.carplay,&config.hicar]{if args.len()>64||args.iter().any(|v|v.contains('\0')||v.len()>4096){return Err("invalid backend command".into());}}
        for d in &config.devices{d.validate()?;}
        let mut store=if config.state.exists(){serde_json::from_slice::<Store>(&private_read(&config.state)?).map_err(|_|"invalid device store")?}else{Store::default()};
        if store.devices.len()>8{return Err("too many stored devices".into());}for d in &store.devices{d.validate()?;}
        if let Some(last)=&store.last{last.validate()?;if !store.devices.iter().any(|d|d.protocol==last.protocol&&d.address.eq_ignore_ascii_case(&last.address)){store.last=None;}}
        for d in config.devices.iter().filter(|_|!config.state.exists()){if !store.devices.iter().any(|v|v.protocol==d.protocol&&v.address.eq_ignore_ascii_case(&d.address)){store.devices.push(d.clone());}}
        if store.devices.len()>8{return Err("too many configured devices".into());}
        let boot=fs::read_to_string("/proc/sys/kernel/random/boot_id").ok().map(|s|s.trim().to_owned());
        if boot.is_some()&&store.boot_id!=boot{store.paused=false;store.boot_id=boot;
            store.selected_protocol=match store.boot_protocol{BootProtocol::Carplay=>Some(Protocol::Carplay),BootProtocol::Hicar=>Some(Protocol::Hicar),BootProtocol::Last=>store.selected_protocol.or(store.preferred.as_ref().or(store.last.as_ref()).map(|d|d.protocol))};
        }
        let paused=store.paused;let protocol=store.selected_protocol.or(store.preferred.as_ref().or(store.last.as_ref()).map(|d|d.protocol)).or(config.initial).unwrap_or(Protocol::Carplay);let default_protocol=store.boot_protocol;
        Ok(Self{config,store,status:Status{schema:1,generation:0,default_protocol,protocol:Some(protocol),target:None,active:false,pairing:false,paused,call_active:false,carlife_pid:None,backend_pid:None,usb_connected:false,menu_visible:false,last_error:None,last_stage:None},
            life:None,backend:None,choice:None,life_started:Instant::now(),life_retry_at:Instant::now(),life_failures:0,life_ready:false,last_media_report:None,started:Instant::now(),recovery_deadline:None,ready:None,seen_media:false,committed:false,pair_deadline:None,repair_used:false,retry_at:Instant::now(),tried:VecDeque::new(),last_auth:None,refusals:0,mux:None,menu_focus:0,menu_pending:None,menu_idle:true,wireless_ready:false,source_ready:false,session_epoch:0,closed_hicar_epoch:None,source_floor:0,bound_source_id:0,pair_pin:None})
    }
    fn selected_protocol(&self)->Protocol{self.store.selected_protocol.or(self.status.protocol).or(self.store.preferred.as_ref().or(self.store.last.as_ref()).map(|d|d.protocol)).or(self.config.initial).unwrap_or(Protocol::Carplay)}
    fn automatic(&self)->Option<Choice>{
        if self.store.paused||self.store.policy==Policy::Manual{return None;}
        let protocol=self.selected_protocol();
        let eligible=|d:&&Device|d.protocol==protocol&&d.auto&&!self.tried.contains(&d.address);
        let device=self.store.preferred.as_ref().or(self.store.last.as_ref()).and_then(|d|self.store.devices.iter().find(|v|v.protocol==protocol&&v.address.eq_ignore_ascii_case(&d.address))).filter(eligible).cloned()
            .or_else(||self.store.devices.iter().filter(eligible).max_by_key(|d|d.last_used).cloned());
        if let Some(device)=device{return Some(Choice{protocol,device:Some(device),pairing:false});}
        // Never fall back to another protocol. Retry the current set instead.
        if self.store.devices.iter().any(|d|d.protocol==protocol){return None;}
        Some(Choice{protocol,device:None,pairing:true})
    }
    fn stop_backend(&mut self)->Result<()>{
        self.status.call_active=false;
        self.pair_pin=None;
        self.menu_pending=None;self.menu_focus=0;
        let had_carplay=self.backend.is_some()&&self.status.protocol==Some(Protocol::Carplay);
        if let Some(mut p)=self.backend.take(){
            p.end()?;
            // Unregistering a profile can leave its selected phone's ACL up.
            // Drop that transport after the backend exits, retaining its bond
            // and application keys. An absent/already-down link is harmless.
            if let Some(target)=self.status.target.as_ref(){
                if self.bluetooth(&["--timeout","3","disconnect",&target.address]).is_err(){event("manager_target_disconnect_failed");}
            }
        }
        if let Some(mux)=&self.mux{self.source_floor=mux.retire_sources()?;}else{self.source_floor=self.bound_source_id.saturating_add(1);}
        self.bound_source_id=0;
        self.status.backend_pid=None;self.status.active=false;self.ready=None;self.seen_media=false;self.wireless_ready=false;self.source_ready=false;self.show_recovery_menu();
        // Only our terminated receiver's socket in its private directory.
        let media=self.config.media_socket.as_deref().unwrap_or(Path::new("/run/carlink-carplay/media.sock"));
        if had_carplay&&media.parent().is_some_and(|p|p.is_dir())&&fs::symlink_metadata(media).is_ok_and(|m|m.file_type().is_socket()){
            fs::remove_file(media).map_err(|_|"cannot remove stopped receiver socket")?;
        }Ok(())
    }
    fn select(&mut self,choice:Choice)->Result<()>{
        if self.status.call_active{return Err("finish the active call before switching phones".into());}
        if choice.protocol==Protocol::Hicar&&((choice.device.is_none()&&!choice.pairing)||self.config.hicar.is_empty()){return Err("HiCar requires a selected target or explicit pairing window and configured backend".into());}
        // Validate the persisted PIN before interrupting the current projection.
        // Identity creation and rotation belong to provisioning, never this UI.
        let pair_pin=if choice.protocol==Protocol::Hicar&&choice.pairing{Some(self.read_pair_pin()?)}else{None};
        self.status.active=false;self.show_recovery_menu();
        self.stop_backend()?;self.session_epoch=0;self.closed_hicar_epoch=None;self.status.generation=self.status.generation.checked_add(1).ok_or("generation exhausted")?;
        let mut args=if choice.protocol==Protocol::Carplay{self.config.carplay.clone()}else{self.config.hicar.clone()};
        if choice.pairing {args.extend(["--pair".into(),self.config.pair_seconds.to_string()]);}
        if let Some(d)=&choice.device{args.extend([if choice.protocol==Protocol::Carplay{"--iphone".into()}else{"--peer".into()},d.address.clone()]);}
        if choice.protocol==Protocol::Hicar {
            let epoch=SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_|"invalid system clock")?.as_secs();
            args.extend(["--sequence".into(),((epoch+self.status.generation)%256).to_string()]);
        }
        if choice.protocol==Protocol::Hicar{if let Some(path)=&self.config.source_socket{if let Some(i)=args.iter().position(|v|v=="--live-socket"){args[i+1]=path.to_string_lossy().into_owned();}}}
        let process=spawn(&args,self.status.generation,self.config.source_socket.as_deref())?;
        self.store.selected_protocol=Some(choice.protocol);save(&self.config.state,&self.store)?;
        self.status.protocol=Some(choice.protocol);self.status.target=choice.device.clone();self.status.pairing=choice.pairing;
        self.status.backend_pid=Some(process.child.id());self.status.menu_visible=true;self.menu_idle=true;self.status.last_error=None;self.status.last_stage=None;self.backend=Some(process);self.choice=Some(choice.clone());
        self.started=Instant::now();self.recovery_deadline=None;self.ready=None;self.seen_media=false;self.committed=false;self.repair_used=false;self.refusals=0;
        self.pair_pin=pair_pin;self.pair_deadline=choice.pairing.then(||Instant::now()+Duration::from_secs(self.config.pair_seconds));event("manager_backend_started");Ok(())
    }
    fn read_pair_pin(&self)->Result<PairPin>{
        let path=if let Some(index)=self.config.hicar.iter().position(|v|v=="--identity"){
            Path::new(self.config.hicar.get(index+1).ok_or("HiCar identity path is missing")?)
        }else{Path::new("/var/lib/carlink/hicar/production/identity.json")};
        let identity=crate::hicar::identity::Identity::load(path)?;
        Ok(PairPin(identity.pin.clone()))
    }
    fn cancel_pairing(&mut self)->Result<()>{
        if !self.status.pairing{return Err("no pairing window is active".into());}
        self.stop_backend()?;self.choice=None;self.status.protocol=Some(self.selected_protocol());self.status.target=None;
        self.status.pairing=false;self.pair_deadline=None;self.tried.clear();self.retry_at=Instant::now();
        self.show_recovery_menu();event("manager_pairing_cancelled");Ok(())
    }
    fn repair(&mut self)->Result<()>{
        let d=self.choice.as_ref().and_then(|c|c.device.clone()).ok_or("no target to repair")?;
        if d.protocol!=Protocol::Carplay||self.status.active||self.repair_used{return Err("pairing repair is unavailable in this state".into());}
        self.stop_backend()?;
        self.bluetooth(&["--timeout","5","remove",&d.address])?;
        self.select(Choice{protocol:Protocol::Carplay,device:Some(d),pairing:true})?;self.repair_used=true;event("manager_target_repair_window");Ok(())
    }
    fn bluetooth(&self,args:&[&str])->Result<()>{
        let tool=self.config.bluetoothctl.as_deref().unwrap_or("/usr/bin/bluetoothctl");
        let mut child=Command::new(tool).args(args).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|_|"cannot start Bluetooth control")?;
        let until=Instant::now()+Duration::from_secs(6);let success=loop{if let Some(status)=child.try_wait().map_err(|_|"pairing cleanup failed")?{break status.success();}
            if Instant::now()>=until{let _=child.kill();let _=child.wait();break false;}thread::sleep(Duration::from_millis(20));};
        if !success{return Err("Bluetooth control failed".into());}Ok(())
    }
    fn show_recovery_menu(&mut self){let explicit=self.status.menu_visible&&!self.menu_idle;self.status.menu_visible=true;self.menu_idle=!explicit;self.menu_pending=None;self.menu_focus=0;if let Some(mux)=&self.mux{let (rows,_)=self.menu_model();let _=mux.set_menu(rows,0,true);}}
    fn source_lost(&mut self){self.recovery_deadline=Some(Instant::now()+Duration::from_secs(8));self.status.call_active=false;self.wireless_ready=false;self.status.active=false;self.ready=None;self.seen_media=false;self.show_recovery_menu();self.started=Instant::now()-Duration::from_secs((if self.status.protocol==Some(Protocol::Hicar){120}else{self.config.connect_seconds}).saturating_sub(8));}
    fn hicar_session_closed(&mut self){
        // Media workers can finish a report after their control channel has
        // closed. A terminal epoch cannot become healthy again from that report.
        self.closed_hicar_epoch=Some(self.closed_hicar_epoch.unwrap_or(0).max(self.session_epoch));
        self.source_lost();
    }
    fn mark_active(&mut self)->Result<()>{
        if self.wireless_ready&&(self.mux.is_none()||self.source_ready){
            if !self.status.active {
                self.recovery_deadline=None;self.ready=Some(Instant::now());self.status.active=true;self.status.last_error=None;
                if let Some(d)=self.status.target.as_ref().filter(|d|self.store.devices.iter().any(|v|v.protocol==d.protocol&&v.address.eq_ignore_ascii_case(&d.address))){
                    self.store.preferred=Some(d.clone());save(&self.config.state,&self.store)?;
                }
            }
        }Ok(())
    }
    fn ingest(&mut self,generation:u64,v:Value)->Result<()>{
        if generation!=self.status.generation{return Ok(());}
        if self.status.protocol==Some(Protocol::Hicar){if let Some(epoch)=v["session_epoch"].as_u64(){
            if epoch<self.session_epoch{return Ok(());}self.session_epoch=epoch;
        }}
        if self.status.protocol==Some(Protocol::Hicar){
            if v["event"]=="bridge_stats"&&self.last_media_report.is_none_or(|t|t.elapsed()>=Duration::from_secs(5)){
                let mut report=json!({"event":"manager_hicar_audio"});
                for name in ["audio_frames","pcm_blocks","audio_lost","audio_resets","audio_underruns","mix_late","microphone_active","microphone_packets","call_active","pending","dropped"]{
                    if v[name].is_u64()||v[name].is_boolean(){report[name]=v[name].clone();}
                }
                println!("{report}");self.last_media_report=Some(Instant::now());
            }
            if v["event"]=="hicar_audio_service"{println!("{}",json!({"event":"manager_hicar_audio_service","service":v["service"].as_str().filter(|n|matches!(*n,"speaker1"|"mic1"|"modemMic"|"modemSpeaker")).unwrap_or("unknown"),"ready":v["ready"]==true,"stopped":v["stopped"]==true}));}
            if v["event"]=="microphone_error"{println!("{}",json!({"event":"manager_hicar_microphone_error","capture_stopped":true}));}
        }
        let name=v["event"].as_str().unwrap_or("");
        if name=="manager_backend_stage"{
            if let Some(report)=stage_report(generation,self.status.generation,&v){
                let stage=report["stage"].as_str().unwrap();self.status.last_stage=Some(stage.into());
                // A normal RFCOMM authentication handoff can end its stream.
                // Projection health is determined by control/media lifecycle.
                if matches!(stage,"auth_parse_stopped"|"control_parse_stopped"|"control_peer_timeout"|"hisight_parse_stopped"|"parse_stopped"|"tcp_stream_error"|"hisight_error"){self.status.last_error=Some(stage.into());}
                if matches!(stage,"control_parse_stopped"|"control_peer_timeout"|"hisight_parse_stopped"|"hisight_error"){
                    if self.status.protocol==Some(Protocol::Hicar){self.hicar_session_closed();}else{self.source_lost();}
                }
                println!("{report}");
            }return Ok(());
        }
        if name=="retry"||name=="stage"&&v["stage"]=="WaitingForPairedIphone" {
            if self.status.active {self.wireless_ready=false;self.source_lost();}
            let reason=v["reason"].as_str().unwrap_or("");
            if reason.contains("no longer paired")&&self.status.protocol==Some(Protocol::Carplay)&&!self.status.pairing {
                if let Some(d)=self.status.target.clone(){self.select(Choice{protocol:Protocol::Carplay,device:Some(d),pairing:true})?;
                    self.repair_used=true;event("manager_pairing_needed");return Ok(());}
            }
            if reason.contains("Connection refused"){self.refusals=self.refusals.saturating_add(1);}
            if (auth_mismatch(reason)||self.refusals>=3)&&self.status.protocol==Some(Protocol::Carplay)&&!self.status.active&&!self.repair_used{
                let now=Instant::now();if self.last_auth.is_none_or(|t|now.duration_since(t)>Duration::from_secs(120)){
                    self.last_auth=Some(now);return self.repair();
                }
            }
        }
        let ready=match self.status.protocol{
            Some(Protocol::Carplay)=>name=="stage"&&v["stage"]=="WirelessActive",
            Some(Protocol::Hicar)=>name=="bridge_stats"&&v["active"]==true
                &&self.closed_hicar_epoch.is_none_or(|closed|self.session_epoch>closed)
                &&v.get("epoch_video_frames").unwrap_or(&v["video_frames"]).as_u64().unwrap_or(0)>0,
            None=>false,
        };
        if ready {self.wireless_ready=true;self.mark_active()?;}
        if self.status.protocol==Some(Protocol::Hicar)&&name=="bridge_stats"&&v["connected"]==false&&v["video_ended"]==true&&self.status.active{self.hicar_session_closed();}
        if name=="forward_latency"&&v["sent"].as_u64().unwrap_or(0)>0||name=="bridge_stats"&&v["connected"]==true&&v["video_frames"].as_u64().unwrap_or(0)>0{self.seen_media=true;}
        if name=="stage"&&v["stage"]=="ConnectingBluetooth"{
            if let Some(address)=v["address"].as_str().filter(|s|mac(s)){
                if self.status.pairing {
                    let current=self.store.devices.iter().find(|d|d.protocol==Protocol::Carplay&&d.address.eq_ignore_ascii_case(address)).cloned()
                        .or_else(||self.status.target.as_ref().filter(|d|d.protocol==Protocol::Carplay&&d.address.eq_ignore_ascii_case(address)).cloned());
                    self.status.target=Some(current.unwrap_or(Device{protocol:Protocol::Carplay,address:address.to_uppercase(),name:format!("iPhone · {}",&address[address.len()-5..].to_uppercase()),auto:true,last_used:0}));
                }
            }
        }
        if name=="target_selected"&&self.status.pairing&&self.status.protocol==Some(Protocol::Hicar){
            if let Some(address)=v["address"].as_str().filter(|s|mac(s)){if self.status.target.is_none(){
                self.status.target=Some(self.store.devices.iter().find(|d|d.protocol==Protocol::Hicar&&d.address.eq_ignore_ascii_case(address)).cloned().unwrap_or(Device{protocol:Protocol::Hicar,address:address.to_uppercase(),name:format!("Huawei · {}",&address[address.len()-5..].to_uppercase()),auto:true,last_used:0}));
            }}
        }
        if name=="hisight_closed" {self.wireless_ready=false;self.hicar_session_closed();}
        Ok(())
    }
    fn command(&mut self,v:Value)->Result<Value>{
        match v["op"].as_str().ok_or("missing control operation")?{
            "status"=>{},
            "menu"=>{if self.mux.is_none(){return Err("car menu is not configured".into());}self.status.menu_visible=v["visible"].as_bool().ok_or("menu visibility required")?||!self.status.active;self.menu_idle=false;self.menu_focus=0;self.menu_pending=None;},
            "devices"=>return Ok(json!({"ok":true,"devices":self.store.devices})),
            "remember"=>{let d:Device=serde_json::from_value(v["device"].clone()).map_err(|_|"invalid device record")?;d.validate()?;
                self.store.devices.retain(|x|!(x.protocol==d.protocol&&x.address.eq_ignore_ascii_case(&d.address)));
                if self.store.devices.len()>=8{return Err("device store is full".into());}self.store.devices.push(d);save(&self.config.state,&self.store)?;},
            "switch_protocol"=>{
                let protocol=if v.get("protocol").is_some(){serde_json::from_value(v["protocol"].clone()).map_err(|_|"invalid protocol")?}else{match self.selected_protocol(){Protocol::Carplay=>Protocol::Hicar,Protocol::Hicar=>Protocol::Carplay}};
                if self.status.call_active{return Err("finish the active call before switching protocols".into());}
                self.stop_backend()?;self.choice=None;self.store.selected_protocol=Some(protocol);self.status.protocol=Some(protocol);self.status.target=None;self.status.pairing=false;self.pair_deadline=None;self.tried.clear();self.store.paused=false;self.status.paused=false;self.store.policy=Policy::Last;save(&self.config.state,&self.store)?;
                self.retry_at=Instant::now();self.show_recovery_menu();
            },
            "default_protocol"=>{let mode:BootProtocol=serde_json::from_value(v["mode"].clone()).map_err(|_|"invalid default protocol")?;self.store.boot_protocol=mode;self.status.default_protocol=mode;save(&self.config.state,&self.store)?;},
            "connect_device"=>{self.store.paused=false;self.status.paused=false;self.store.policy=Policy::Last;self.tried.clear();save(&self.config.state,&self.store)?;
                if !self.status.active{self.select(Choice{protocol:self.selected_protocol(),device:None,pairing:true})?;self.menu_idle=true;}
            },
            "connect"=>{let address=v["address"].as_str().ok_or("missing target address")?;
                let protocol:Protocol=serde_json::from_value(v["protocol"].clone()).map_err(|_|"missing protocol")?;
                let d=self.store.devices.iter().find(|d|d.protocol==protocol&&d.address.eq_ignore_ascii_case(address)).cloned().ok_or("unknown target device")?;
                self.tried.clear();self.select(Choice{protocol,device:Some(d),pairing:false})?;self.store.paused=false;self.status.paused=false;save(&self.config.state,&self.store)?;},
            "pair"=>{let protocol:Protocol=serde_json::from_value(v["protocol"].clone()).map_err(|_|"missing protocol")?;
                if self.store.devices.len()>=8{return Err("device store is full; forget a device first".into());}
                self.select(Choice{protocol,device:None,pairing:true})?;self.menu_idle=true;},
            "cancel_pair"=>self.cancel_pairing()?,
            "policy"=>{let mode:Policy=serde_json::from_value(v["mode"].clone()).map_err(|_|"invalid connection policy")?;
                if mode==Policy::Favorite{let p:Protocol=serde_json::from_value(v["protocol"].clone()).map_err(|_|"missing favorite protocol")?;let a=v["address"].as_str().ok_or("missing favorite target")?;
                    self.store.selected_protocol=Some(p);if self.backend.is_none(){self.status.protocol=Some(p);}
                    self.store.favorite=Some(self.store.devices.iter().find(|d|d.protocol==p&&d.address.eq_ignore_ascii_case(a)).cloned().ok_or("unknown favorite target")?);}
                self.store.policy=mode;save(&self.config.state,&self.store)?;},
            "forget"=>{if v["confirm"]!=true{return Err("forget requires explicit confirmation".into());}
                let p:Protocol=serde_json::from_value(v["protocol"].clone()).map_err(|_|"missing target protocol")?;let a=v["address"].as_str().ok_or("missing target")?;
                if !self.store.devices.iter().any(|d|d.protocol==p&&d.address.eq_ignore_ascii_case(a)){return Err("unknown target device".into());}
                if self.status.target.as_ref().is_some_and(|d|d.protocol==p&&d.address.eq_ignore_ascii_case(a)){self.stop_backend()?;self.choice=None;self.status.target=None;self.status.protocol=Some(self.selected_protocol());self.status.pairing=false;self.pair_deadline=None;}
                if !self.store.devices.iter().any(|d|d.protocol!=p&&d.address.eq_ignore_ascii_case(a)){self.bluetooth(&["--timeout","5","remove",a])?;}
                self.store.devices.retain(|d|!(d.protocol==p&&d.address.eq_ignore_ascii_case(a)));
                if self.store.last.as_ref().is_some_and(|d|d.protocol==p&&d.address.eq_ignore_ascii_case(a)){self.store.last=None;}
                if self.store.preferred.as_ref().is_some_and(|d|d.protocol==p&&d.address.eq_ignore_ascii_case(a)){self.store.preferred=None;}
                if self.store.favorite.as_ref().is_some_and(|d|d.protocol==p&&d.address.eq_ignore_ascii_case(a)){self.store.favorite=None;}
                self.store.paused=true;self.status.paused=true;save(&self.config.state,&self.store)?;},
            "repair"=>self.repair()?,
            "disconnect"=>{self.stop_backend()?;self.choice=None;self.status.protocol=Some(self.selected_protocol());self.status.target=None;self.status.pairing=false;self.pair_deadline=None;
                self.store.paused=true;self.status.paused=true;save(&self.config.state,&self.store)?;},
            "pause"=>{self.store.paused=true;self.status.paused=true;save(&self.config.state,&self.store)?;},
            "resume"=>{self.store.paused=false;self.status.paused=false;self.tried.clear();save(&self.config.state,&self.store)?;
                if self.backend.is_none(){if let Some(c)=self.automatic(){self.select(c)?;}}},
            "shutdown"=>STOP.store(true,Ordering::Relaxed),_=>return Err("unsupported manager operation".into()),
        }Ok(json!({"ok":true,"status":self.status}))
    }
    fn menu_model(&self)->(Vec<String>,Vec<Value>){
        let protocol=self.selected_protocol();let current=if protocol==Protocol::Carplay{"CarPlay"}else{"HiCar"};let other=if protocol==Protocol::Carplay{"HiCar"}else{"CarPlay"};
        let first=format!("当前协议：{current}，点击切换到 {other}");
        let (second,action)=if self.status.active{
            (format!("断开 {}",self.status.target.as_ref().map(|d|d.name.as_str()).unwrap_or(current)),json!({"op":"disconnect"}))
        }else if self.store.paused{("连接设备".into(),json!({"op":"connect_device"}))}
        else if self.status.pairing{
            (if let Some(pin)=&self.pair_pin{format!("配对中 · PIN：{}",pin.0)}else{"配对中".into()},json!({"op":"connect_device"}))
        }else{("连接设备 · 正在自动重连".into(),json!({"op":"connect_device"}))};
        let (label,next)=match self.store.boot_protocol{BootProtocol::Last=>("上一次选择","hicar"),BootProtocol::Hicar=>("华为 HiCar","carplay"),BootProtocol::Carplay=>("苹果 CarPlay","last")};
        (vec![first,second,format!("默认协议：{label}")],vec![json!({"op":"switch_protocol"}),action,json!({"op":"default_protocol","mode":next})])
    }
    fn source_event_current(&mut self,v:&Value)->bool{
        if !matches!(v["type"].as_str(),Some("source-state"|"presentation-state"|"call-state")){return true;}
        let Some(id)=v["source_id"].as_u64()else{
            // Unit fixtures predate source IDs. Runtime callbacks always carry
            // an ID assigned by the resident mux, never by a backend payload.
            return cfg!(test);
        };
        if id<self.source_floor||id<self.bound_source_id{return false;}
        self.bound_source_id=id;true
    }
    fn menu_input(&mut self,v:Value)->Result<()>{
        if !self.source_event_current(&v){return Ok(());}
        if v["type"]=="call-state"{self.status.call_active=v["active"]==true;return Ok(());}
        if v["type"]=="open-menu"{self.status.menu_visible=true;self.menu_idle=false;self.menu_focus=0;self.menu_pending=None;return Ok(());}
        if v["type"]=="presentation-state"{
            // The CarPlay producer derives this flag from the OXP session phase.
            // HiCar derives it from display-surface state, which may pause during
            // an app transition while its authenticated session remains alive.
            if self.status.protocol==Some(Protocol::Carplay){
                self.source_ready=v["active"]==true;
                if !self.source_ready&&(self.status.active||self.wireless_ready){self.source_lost();}
                else if self.source_ready{
                    // This producer flag comes directly from OXP session ACTIVE,
                    // including an IPC-only reconnect of an existing session.
                    self.wireless_ready=true;self.mark_active()?;
                }
            }
            return Ok(());
        }
        if v["type"]=="source-state"{self.source_ready=v["active"]==true;if !self.source_ready{self.status.call_active=false;if self.status.active{self.source_lost();}}self.mark_active()?;return Ok(());}
        if !self.status.menu_visible{return Ok(());}let (rows,actions)=self.menu_model();
        if v["menu_rows"]!=json!(rows){return Ok(());}
        let kind=v["type"].as_str().unwrap_or("");if kind=="menu-error"{self.status.last_error=Some("local menu encoder failed".into());return Ok(());}let mut chosen=None;
        if kind=="key-press"||kind=="key"{match v["code"].as_u64(){
            Some(3|5)=>self.menu_focus=self.menu_focus.saturating_sub(1),Some(4|6)=>self.menu_focus=(self.menu_focus+1).min(rows.len()-1),
            Some(2)=>chosen=Some(self.menu_focus),Some(0|1)=>{if self.menu_pending.is_some(){self.menu_pending=None;self.menu_focus=0;}else if self.status.pairing&&self.pair_pin.is_some(){self.cancel_pairing()?;return Ok(());}else if self.status.active{self.status.menu_visible=false;self.menu_idle=false;}},_=>{}}}
        if kind=="menu-focus"{if let Some(i)=v["index"].as_u64().filter(|i|(*i as usize)<rows.len()){self.menu_focus=i as usize;}}
        if kind=="menu-select"{chosen=v["index"].as_u64().map(|n|n as usize);}
        if let Some(i)=chosen.filter(|i|*i<actions.len()){let action=actions[i].clone();match action["op"].as_str(){
            Some("menu_cancel")=>{self.menu_pending=None;self.menu_focus=0;},
            _=>{self.menu_pending=None;self.menu_focus=0;let close=action["op"]=="connect";self.command(action)?;if close{self.status.menu_visible=true;self.menu_idle=true;}}
        }}Ok(())
    }
    fn can_commit(&self)->bool{
        !self.committed&&self.status.active&&self.status.usb_connected&&self.seen_media
            &&self.ready.is_some_and(|t|t.elapsed()>=Duration::from_secs(self.config.stable_seconds))
    }
    fn record_stable_target(&mut self)->Result<()>{
        if let Some(mut d)=self.status.target.clone(){
            if let Some(current)=self.store.devices.iter().find(|v|v.protocol==d.protocol&&v.address.eq_ignore_ascii_case(&d.address)){d.name=current.name.clone();d.auto=current.auto;}
            d.last_used=SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_|"invalid system clock")?.as_secs();
            self.store.devices.retain(|v|!(v.protocol==d.protocol&&v.address.eq_ignore_ascii_case(&d.address)));
            if self.store.devices.len()<8{self.store.devices.push(d.clone());}self.store.last=Some(d.clone());self.store.preferred=Some(d.clone());self.status.target=Some(d);save(&self.config.state,&self.store)?;
        }Ok(())
    }
    fn tick(&mut self)->Result<()>{
        let mut inputs=vec![];if let Some(mux)=&self.mux{for _ in 0..16{if let Some(v)=mux.try_input(){inputs.push(v);}else{break;}}}
        for v in inputs{self.menu_input(v)?;}
        if self.menu_idle&&self.status.active{self.status.menu_visible=false;self.menu_idle=false;}
        if let Some(mux)=&self.mux{let (rows,_)=self.menu_model();self.menu_focus=self.menu_focus.min(rows.len()-1);mux.set_menu(rows,self.menu_focus,self.status.menu_visible)?;}

        if let Some(life)=self.life.as_mut(){for v in life.events.try_iter().take(16){if v["event"]=="voice_stats"{println!("{}",json!({"event":"manager_voice_stats","microphone_active":v["microphone_active"]==true,"microphone_packets":v["microphone_packets"].as_u64().unwrap_or(0),"microphone_bytes":v["microphone_bytes"].as_u64().unwrap_or(0),"call_active":v["call_active"]==true}));}if v["event"]=="carlife_state"{self.status.usb_connected=true;self.life_ready=true;self.status.last_error=None;}if v["event"]=="session_error"{
            self.status.usb_connected=false;
            self.status.last_error=Some(v["detail"].as_str().unwrap_or("CarLife output failed").chars().take(256).collect());event("manager_carlife_error");
        }}}
        if self.life.as_mut().is_some_and(|p|p.child.try_wait().ok().flatten().is_some()){
            self.life=None;self.status.carlife_pid=None;self.status.usb_connected=false;self.defer_carlife();event("manager_carlife_restart");
            if fs::symlink_metadata(&self.config.live_socket).is_ok_and(|m|m.file_type().is_socket()){
                fs::remove_file(&self.config.live_socket).map_err(|_|"cannot clear terminated CarLife endpoint")?;
            }
        }
        if self.life.is_none()&&Instant::now()>=self.life_retry_at {
            let p=spawn(&self.config.carlife,0,None)?;self.status.carlife_pid=Some(p.child.id());self.life=Some(p);self.life_started=Instant::now();self.life_ready=false;
        }
        if let Some(p)=self.backend.as_mut(){let generation=p.generation;let events=p.events.try_iter().take(64).collect::<Vec<_>>();
            for e in events{self.ingest(generation,e)?;}}
        if self.backend.as_mut().is_some_and(|p|p.child.try_wait().ok().flatten().is_some()){
            self.source_lost();
            if let Some(d)=&self.status.target{if !self.tried.contains(&d.address){self.tried.push_back(d.address.clone());}}
            self.stop_backend()?;self.retry_at=Instant::now()+Duration::from_secs(2);event("manager_backend_exited");
        }
        if self.can_commit(){
            self.record_stable_target()?;
            if self.status.pairing&&self.status.protocol==Some(Protocol::Carplay){
                self.bluetooth(&["--timeout","3","discoverable","off"])?;
                self.bluetooth(&["--timeout","3","pairable","off"])?;
            }
            self.committed=true;self.status.pairing=false;self.pair_deadline=None;self.pair_pin=None;event("manager_stable_session");
        }
        if self.pair_deadline.is_some_and(|t|Instant::now()>=t){self.pair_pin=None;}
        if self.pair_deadline.is_some_and(|t|Instant::now()>=t)&&!self.status.active{
            self.stop_backend()?;self.pair_deadline=None;self.status.pairing=false;self.choice=None;self.retry_at=Instant::now()+Duration::from_secs(8);
        }
        if !self.status.active&&!self.status.pairing&&self.backend.is_some()&&(if self.status.protocol==Some(Protocol::Hicar){self.recovery_deadline.is_some_and(|t|Instant::now()>=t)}else{self.started.elapsed()>Duration::from_secs(self.config.connect_seconds+if self.refusals>0{15}else{0})}){
            if let Some(d)=&self.status.target{self.tried.push_back(d.address.clone());}
            self.stop_backend()?;self.retry_at=Instant::now()+Duration::from_secs(2);
        }
        if self.backend.is_none()&&!self.store.paused&&Instant::now()>=self.retry_at&&self.life.is_some(){
            if let Some(c)=self.automatic(){self.select(c)?;}else{self.tried.clear();self.retry_at=Instant::now()+Duration::from_secs(8);}
        }Ok(())
    }
    fn defer_carlife(&mut self){
        // A failing HU must see a meaningful detach interval. Do not reset on
        // initial negotiation: a broken session may fail immediately afterward.
        self.life_failures=if self.life_ready&&self.life_started.elapsed()>=Duration::from_secs(30){0}else{self.life_failures.saturating_add(1)};
        self.life_retry_at=Instant::now()+Duration::from_millis(500u64<<self.life_failures.min(4));
    }
}
fn auth_mismatch(s:&str)->bool{["AuthenticationFailed","AuthenticationRejected","AuthenticationCanceled","br-connection-key-missing","PIN or Key Missing"].iter().any(|p|s.contains(p))}
fn exchange(stream:&mut UnixStream)->Result<Value>{
    stream.set_read_timeout(Some(Duration::from_millis(500))).map_err(|_|"control timeout failed")?;
    let mut bytes=vec![];BufReader::new(stream).take(16385).read_until(b'\n',&mut bytes).map_err(|_|"control read failed")?;
    if bytes.len()>16384||bytes.last()!=Some(&b'\n'){return Err("invalid bounded control request".into());}
    serde_json::from_slice(&bytes).map_err(|_|"invalid control JSON".into())
}
fn serve(config:&Path)->Result<()>{
    let config:Config=serde_json::from_slice(&private_read(config)?).map_err(|_|"invalid manager configuration")?;
    let owner=Owner::new(config)?;let parent=owner.config.socket.parent().ok_or("control parent missing")?;private_dir(parent)?;
    let radio_path=owner.config.radio_lock.as_deref().unwrap_or(Path::new("/run/lock/carlink-radio.lock"));
    if !radio_path.is_absolute(){return Err("radio lock path must be absolute".into());}
    let radio=OpenOptions::new().read(true).write(true).create(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(radio_path).map_err(|_|"cannot open radio ownership lock")?;
    if !radio.metadata().map_err(|_|"cannot stat radio lock")?.is_file()||unsafe{libc::flock(radio.as_raw_fd(),libc::LOCK_EX|libc::LOCK_NB)}!=0{
        return Err("another process owns the wireless radio".into());
    }
    let lock=OpenOptions::new().read(true).write(true).create(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(parent.join("owner.lock")).map_err(|_|"cannot acquire manager lock")?;
    if unsafe{libc::flock(lock.as_raw_fd(),libc::LOCK_EX|libc::LOCK_NB)}!=0{return Err("another connection manager owns this runtime".into());}
    // Move runtime ownership after the locks in drop order, including every
    // startup-error path (for example a busy HTTP port after Mux::start).
    let mut owner=owner;
    clear_stale_socket(&owner.config.socket)?;
    private_dir(owner.config.live_socket.parent().ok_or("live parent missing")?)?;
    clear_stale_socket(&owner.config.live_socket)?;
    if let Some(source)=&owner.config.source_socket{private_dir(source.parent().ok_or("source parent missing")?)?;clear_stale_socket(source)?;}
    let listener=UnixListener::bind(&owner.config.socket).map_err(|_|"cannot bind manager control")?;
    let _control_path=SocketPath::owned(&owner.config.socket)?;
    fs::set_permissions(&owner.config.socket,fs::Permissions::from_mode(0o600)).map_err(|_|"cannot secure manager control")?;
    listener.set_nonblocking(true).map_err(|_|"cannot configure manager control")?;
    unsafe{libc::signal(libc::SIGTERM,stop as libc::sighandler_t);libc::signal(libc::SIGINT,stop as libc::sighandler_t);}
    match (&owner.config.source_socket,&owner.config.menu_library,&owner.config.menu_font){
        (Some(source),Some(library),Some(font))=>{private_dir(source.parent().ok_or("source parent missing")?)?;owner.mux=Some(crate::manager_menu::Mux::start(source,&owner.config.live_socket,library,font)?);owner.status.menu_visible=true;},
        (None,None,None)=>{},_=>return Err("source socket, menu library and font must be configured together".into())}
    let _web=owner.config.web_listen.map(|listen|crate::manager_web::start(listen,owner.config.socket.clone())).transpose()?;
    save(&owner.config.state,&owner.store)?;
    event("manager_started");let result=(||{while !STOP.load(Ordering::Relaxed){
        for _ in 0..4{match listener.accept(){Ok((mut stream,_))=>{
            let response=exchange(&mut stream).and_then(|v|owner.command(v)).unwrap_or_else(|e|{owner.status.last_error=Some(e.clone());json!({"ok":false,"error":e})});
            let _=stream.set_write_timeout(Some(Duration::from_millis(500)));let _=serde_json::to_writer(&mut stream,&response);let _=stream.write_all(b"\n");
        },Err(e) if e.kind()==std::io::ErrorKind::WouldBlock=>break,Err(_)=>return Err("manager accept failed".into())}}
        if let Err(error)=owner.tick(){owner.status.last_error=Some(error);owner.retry_at=Instant::now()+Duration::from_secs(3);}
        thread::sleep(Duration::from_millis(30));
    }Ok(())})();
    // Stop request workers before waiting on children, and release the source
    // socket while radio/owner locks still protect its namespace.
    drop(_web);
    let _=owner.stop_backend();if let Some(mut p)=owner.life.take(){let _=p.end();}
    owner.mux=None;event("manager_stopped");result
}
pub fn run(args:&[String])->Result<()>{match args.first().map(String::as_str){
    Some("run") if args.len()==3&&args[1]=="--config"=>serve(Path::new(&args[2])),
    Some("ctl") if args.len()==3=>{let mut stream=UnixStream::connect(&args[1]).map_err(|_|"manager is not running")?;
        stream.set_write_timeout(Some(Duration::from_secs(12))).map_err(|_|"control timeout failed")?;
        let v:Value=serde_json::from_str(&args[2]).map_err(|_|"invalid control request")?;let mut bytes=serde_json::to_vec(&v).map_err(|_|"invalid control request")?;
        if bytes.len()>16384{return Err("control request too large".into());}bytes.push(b'\n');stream.write_all(&bytes).map_err(|_|"control write failed")?;
        stream.set_read_timeout(Some(Duration::from_secs(15))).map_err(|_|"control timeout failed")?;let mut line=String::new();BufReader::new(stream).take(32769).read_line(&mut line).map_err(|_|"control reply failed")?;
        if line.len()>32768{return Err("control reply too large".into());}print!("{line}");Ok(())},
    _=>Err("Usage: carlink manager run --config PRIVATE_JSON | ctl SOCKET JSON".into()),
}}
#[cfg(test)]mod tests{use super::*;
    #[test]fn active_call_blocks_selection_and_source_loss_releases_protection(){
        let root=test_root("call-guard");let mut owner=test_owner(&root);
        owner.menu_input(json!({"type":"call-state","active":true})).unwrap();
        assert!(owner.status.call_active);
        assert_eq!(owner.select(Choice{protocol:Protocol::Carplay,device:None,pairing:true}).unwrap_err(),"finish the active call before switching phones");
        assert!(owner.backend.is_none());owner.menu_input(json!({"type":"source-state","active":false})).unwrap();assert!(!owner.status.call_active);
        fs::remove_dir(root).unwrap();
    }
    fn test_root(name:&str)->PathBuf{let p=std::env::temp_dir().join(format!("carlink-manager-{}-{name}",std::process::id()));fs::create_dir(&p).unwrap();p}
    fn test_owner(root:&Path)->Owner{Owner::new(Config{schema:1,socket:root.join("control"),live_socket:root.join("live"),state:root.join("state"),carlife:vec![],carplay:vec![],hicar:vec![],devices:vec![],initial:None,bluetoothctl:None,radio_lock:None,media_socket:None,web_listen:None,source_socket:None,menu_library:None,menu_font:None,connect_seconds:30,stable_seconds:10,pair_seconds:120}).unwrap()}
    #[test]fn hicar_pair_menu_uses_persisted_pin_privately_and_cancel_preserves_policy(){
        for (name,paused,policy) in [("pair-running",false,Policy::Last),("pair-paused",true,Policy::Last),("pair-manual",false,Policy::Manual)]{
            let root=test_root(name);let mut owner=test_owner(&root);let path=root.join("identity.json");
            let record=json!({"schema":1,"device_id":"ab".repeat(32),"pin":"012345","name":"CarLink"}).to_string();
            fs::write(&path,&record).unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(0o600)).unwrap();
            owner.config.hicar=vec!["/not-launched".into(),"--identity".into(),path.to_string_lossy().into_owned()];
            owner.pair_pin=Some(owner.read_pair_pin().unwrap());owner.status.protocol=Some(Protocol::Hicar);owner.status.pairing=true;owner.status.menu_visible=true;
            owner.store.paused=paused;owner.status.paused=paused;owner.store.policy=policy;
            let device=Device{protocol:Protocol::Carplay,address:"02:00:00:00:00:01".into(),name:"saved phone".into(),auto:true,last_used:1};
            owner.store.devices.push(device.clone());owner.store.last=Some(device);
            let (rows,actions)=owner.menu_model();assert_eq!(rows.len(),3);assert_eq!(actions.len(),3);
            if !paused{assert!(rows[1].contains("012345"));}
            assert!(!serde_json::to_string(&owner.status).unwrap().contains("012345"));
            assert!(!owner.command(json!({"op":"status"})).unwrap().to_string().contains("012345"));
            assert!(!stage_summary(&json!({"event":"ready","pin":"012345"})).unwrap().to_string().contains("012345"));
            owner.command(json!({"op":"cancel_pair"})).unwrap();
            assert!(owner.pair_pin.is_none());assert!(!owner.status.pairing);assert!(owner.status.menu_visible);assert_eq!(owner.store.paused,paused);assert!(owner.store.policy==policy);assert_eq!(owner.store.devices.len(),1);
            assert_eq!(owner.automatic().is_some(),!paused&&policy==Policy::Last);assert_eq!(fs::read_to_string(&path).unwrap(),record);
            fs::remove_file(path).unwrap();fs::remove_dir(root).unwrap();
        }
    }
    #[test]fn missing_pair_identity_does_not_interrupt_active_projection(){
        let root=test_root("pair-identity-missing");let mut owner=test_owner(&root);
        owner.config.hicar=vec!["/not-launched".into(),"--identity".into(),root.join("missing").to_string_lossy().into_owned()];
        owner.backend=Some(spawn(&["/bin/sleep".into(),"60".into()],0,None).unwrap());owner.status.active=true;owner.status.protocol=Some(Protocol::Carplay);
        let pid=owner.backend.as_ref().unwrap().child.id();assert!(owner.select(Choice{protocol:Protocol::Hicar,device:None,pairing:true}).is_err());
        assert!(owner.status.active);assert_eq!(owner.backend.as_ref().unwrap().child.id(),pid);assert!(owner.backend.as_mut().unwrap().child.try_wait().unwrap().is_none());
        owner.stop_backend().unwrap();fs::remove_dir(root).unwrap();
    }
    #[test]fn stale_socket_cleanup_preserves_active_files_and_symlinks(){
        fn await_stale(path:&Path){
            // A parallel Command::spawn can briefly inherit this CLOEXEC FD
            // between fork and exec. The live-owner guard must keep refusing
            // it until that temporary listener reference is actually gone.
            let deadline=Instant::now()+Duration::from_secs(2);
            loop {match clear_stale_socket(path){Ok(())=>break,Err(e)=>{assert!(Instant::now()<deadline,"{e}");thread::sleep(Duration::from_millis(5));}}}
        }
        let root=test_root("socket-paths");let path=root.join("socket");let listener=UnixListener::bind(&path).unwrap();
        // Fill its pending queue as well: EAGAIN is still a live owner.
        assert_eq!(unsafe{libc::listen(listener.as_raw_fd(),1)},0);
        for _ in 0..8{assert!(clear_stale_socket(&path).is_err());assert!(path.exists());}
        drop(listener);await_stale(&path);assert!(!path.exists());
        fs::write(&path,b"keep").unwrap();assert!(clear_stale_socket(&path).is_err());assert_eq!(fs::read(&path).unwrap(),b"keep");fs::remove_file(&path).unwrap();
        let other=root.join("other");drop(UnixListener::bind(&other).unwrap());std::os::unix::fs::symlink(&other,&path).unwrap();
        assert!(clear_stale_socket(&path).is_err());assert!(fs::symlink_metadata(&path).unwrap().file_type().is_symlink());assert!(other.exists());
        fs::remove_file(path).unwrap();await_stale(&other);fs::remove_dir(root).unwrap();
    }
    #[test]fn bound_socket_guard_cleans_startup_errors_and_preserves_replacements(){
        let root=test_root("socket-guard");let path=root.join("control");
        let failed=(||->Result<()>{let _listener=UnixListener::bind(&path).unwrap();let _path=SocketPath::owned(&path)?;Err("later startup stage failed".into())})();
        assert!(failed.is_err());assert!(!path.exists());
        let listener=UnixListener::bind(&path).unwrap();let guard=SocketPath::owned(&path).unwrap();fs::remove_file(&path).unwrap();fs::write(&path,b"replacement").unwrap();
        drop(guard);assert_eq!(fs::read(&path).unwrap(),b"replacement");drop(listener);fs::remove_file(path).unwrap();fs::remove_dir(root).unwrap();
    }
    #[test]fn repair_and_stable_commit_keep_current_user_device_fields(){
        let root=test_root("device-fields");let mut owner=test_owner(&root);
        let device=Device{protocol:Protocol::Carplay,address:"02:00:00:00:00:01".into(),name:"my phone".into(),auto:false,last_used:1};owner.store.devices.push(device.clone());
        owner.status.protocol=Some(Protocol::Carplay);owner.status.pairing=true;owner.status.generation=1;
        owner.ingest(1,json!({"event":"stage","stage":"ConnectingBluetooth","address":device.address})).unwrap();
        assert_eq!(owner.status.target.as_ref().unwrap().name,"my phone");assert!(!owner.status.target.as_ref().unwrap().auto);
        owner.store.devices[0].name="updated during connection".into();owner.store.devices[0].auto=true;owner.record_stable_target().unwrap();
        assert_eq!(owner.status.target.as_ref().unwrap().name,"updated during connection");assert!(owner.status.target.as_ref().unwrap().auto);
        let saved:Store=serde_json::from_slice(&private_read(&root.join("state")).unwrap()).unwrap();assert_eq!(saved.devices[0].name,"updated during connection");assert!(saved.devices[0].auto);
        fs::remove_file(root.join("state")).unwrap();fs::remove_dir(root).unwrap();
    }
    #[test]fn stopping_a_session_revokes_its_pending_menu_confirmation(){
        let root=test_root("menu-confirm");let mut owner=test_owner(&root);owner.menu_pending=Some(json!({"op":"pair","protocol":"hicar"}));owner.menu_focus=1;
        owner.stop_backend().unwrap();assert!(owner.menu_pending.is_none());assert_eq!(owner.menu_focus,0);fs::remove_dir(root).unwrap();
    }
    #[test]fn stopping_backend_disconnects_only_its_target_and_tolerates_an_absent_link(){
        let root=test_root("target-acl");let mut owner=test_owner(&root);let tool=root.join("bluetoothctl");
        fs::write(&tool,b"#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"${0%/*}/args\"\nexit 1\n").unwrap();fs::set_permissions(&tool,fs::Permissions::from_mode(0o700)).unwrap();owner.config.bluetoothctl=Some(tool.to_string_lossy().into_owned());
        let target=Device{protocol:Protocol::Hicar,address:"02:00:00:00:00:01".into(),name:"selected".into(),auto:true,last_used:0};
        let other=Device{protocol:Protocol::Carplay,address:"02:00:00:00:00:02".into(),name:"other".into(),auto:true,last_used:0};
        owner.store.devices=vec![target.clone(),other];owner.status.target=Some(target.clone());owner.status.protocol=Some(Protocol::Hicar);
        owner.backend=Some(spawn(&["/bin/sleep".into(),"60".into()],1,None).unwrap());
        // The mock reports an already-disconnected link with a nonzero exit.
        owner.stop_backend().unwrap();let first=fs::read_to_string(root.join("args")).unwrap();
        assert_eq!(first,format!("--timeout\n3\ndisconnect\n{}\n",target.address));assert_eq!(owner.store.devices.len(),2);assert!(owner.backend.is_none());
        owner.stop_backend().unwrap();assert_eq!(fs::read_to_string(root.join("args")).unwrap(),first);
        fs::remove_file(tool).unwrap();fs::remove_file(root.join("args")).unwrap();fs::remove_dir(root).unwrap();
    }
    #[test]fn backend_stage_summaries_never_forward_raw_fields(){
        let raw=json!({"event":"rfcomm_connected","address":"02:00:00:00:00:01","reason":"private detail","payload":[1,2],"pin":"test-only","key":"test-only","metadata":{"anything":true}});
        let summary=stage_summary(&raw).unwrap();assert_eq!(summary,json!({"event":"manager_backend_stage","stage":"rfcomm_connected"}));
        let report=stage_report(7,7,&summary).unwrap();assert_eq!(report,json!({"event":"manager_backend_stage","generation":7,"stage":"rfcomm_connected"}));
        assert!(stage_summary(&json!({"event":"private_identity","stage":"ready"})).is_none());
        assert!(stage_report(7,7,&json!({"event":"manager_backend_stage","stage":"ready\nprivate"})).is_none());
    }
    #[test]fn stale_backend_stage_cannot_log_or_change_diagnostics(){
        let root=test_root("stage-generation");let mut owner=test_owner(&root);owner.status.generation=3;owner.status.last_stage=Some("ready".into());
        let summary=stage_summary(&json!({"event":"stream_error","reason":"private detail","payload":[9]})).unwrap();
        assert!(stage_report(2,3,&summary).is_none());owner.ingest(2,summary.clone()).unwrap();
        assert_eq!(owner.status.last_stage.as_deref(),Some("ready"));assert!(owner.status.last_error.is_none());
        owner.ingest(3,summary).unwrap();assert_eq!(owner.status.last_stage.as_deref(),Some("stream_error"));assert!(owner.status.last_error.is_none());
        owner.ingest(3,json!({"event":"manager_backend_stage","stage":"unlisted_private_value"})).unwrap();assert_eq!(owner.status.last_stage.as_deref(),Some("stream_error"));
        fs::remove_dir(root).unwrap();
    }
    #[test]fn stable_source_is_remembered_only_with_a_connected_head_unit(){
        let root=test_root("head-unit-gate");let mut owner=test_owner(&root);
        let device=Device{protocol:Protocol::Hicar,address:"02:00:00:00:00:02".into(),name:"test phone".into(),auto:true,last_used:0};
        owner.status.target=Some(device.clone());owner.store.devices.push(device);
        owner.status.active=true;owner.seen_media=true;owner.ready=Some(Instant::now()-Duration::from_secs(owner.config.stable_seconds+1));
        assert!(!owner.status.usb_connected);assert!(!owner.can_commit());
        assert!(owner.status.active);assert!(owner.store.last.is_none());assert!(!root.join("state").exists());
        // AwaitingVideoSource confirms the CarLife head-unit handshake, not
        // merely a backend's successful write into the resident source mux.
        owner.status.usb_connected=true;assert!(owner.can_commit());owner.record_stable_target().unwrap();owner.committed=true;
        assert!(owner.store.last.is_some());assert!(root.join("state").exists());assert!(!owner.can_commit());
        owner.committed=false;owner.status.usb_connected=false;assert!(!owner.can_commit());assert!(owner.status.active);
        fs::remove_file(root.join("state")).unwrap();fs::remove_dir(root).unwrap();
    }
    #[test]fn reconnect_menu_preference_and_boot_pause(){
        let root=test_root("reconnect-preference");let mut owner=test_owner(&root);
        let a=Device{protocol:Protocol::Carplay,address:"02:00:00:00:00:01".into(),name:"apple".into(),auto:true,last_used:100};
        let b=Device{protocol:Protocol::Hicar,address:"02:00:00:00:00:02".into(),name:"huawei".into(),auto:true,last_used:0};
        owner.store.devices=vec![a.clone(),b.clone()];owner.store.last=Some(a);owner.status.target=Some(b);owner.status.protocol=Some(Protocol::Hicar);
        owner.wireless_ready=true;owner.mark_active().unwrap();assert!(owner.status.active);assert_eq!(owner.automatic().unwrap().protocol,Protocol::Hicar);
        assert!(owner.store.last.as_ref().unwrap().protocol==Protocol::Carplay); // Usage is still gated by HU.
        owner.source_lost();assert!(!owner.status.active);assert!(owner.status.menu_visible);assert!(owner.menu_idle);
        owner.menu_input(json!({"type":"open-menu"})).unwrap();assert!(owner.status.menu_visible);assert!(!owner.menu_idle);
        owner.store.paused=true;owner.store.boot_id=Some("previous-boot".into());save(&owner.config.state,&owner.store).unwrap();
        let restarted=test_owner(&root);assert!(!restarted.store.paused);assert_eq!(restarted.automatic().unwrap().protocol,Protocol::Hicar);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]fn usb_retry_backoff_is_bounded_and_independent_of_wireless(){
        let root=test_root("usb-backoff");let mut owner=test_owner(&root);let wireless=owner.retry_at;
        for expected in [1000,2000,4000,8000,8000,8000]{owner.life_started=Instant::now();let before=Instant::now();owner.defer_carlife();
            let delay=owner.life_retry_at.duration_since(before).as_millis();assert!(delay>=expected&&delay<=expected+50);assert_eq!(owner.retry_at,wireless);}
        owner.life_started=Instant::now()-Duration::from_secs(31);owner.defer_carlife();assert!(owner.life_failures>0);
        owner.life_ready=true;let before=Instant::now();owner.defer_carlife();
        assert_eq!(owner.life_failures,0);assert!(owner.life_retry_at.duration_since(before)<Duration::from_millis(550));fs::remove_dir_all(root).unwrap();
    }
    #[test]fn inactive_hicar_and_previous_epoch_cannot_hide_recovery_menu(){
        let root=test_root("hicar-disconnect");let mut owner=test_owner(&root);owner.status.protocol=Some(Protocol::Hicar);
        owner.wireless_ready=true;owner.status.active=true;owner.status.menu_visible=false;
        owner.ingest(0,json!({"event":"hisight_closed"})).unwrap();
        assert!(!owner.status.active&&!owner.wireless_ready&&owner.status.menu_visible);
        owner.ingest(0,json!({"event":"bridge_stats","active":true,"video_frames":90,"epoch_video_frames":0})).unwrap();
        assert!(!owner.status.active&&owner.status.menu_visible);
        owner.mark_active().unwrap();assert!(!owner.status.active);
        owner.ingest(0,json!({"event":"bridge_stats","session_epoch":1,"active":true,"video_frames":91,"epoch_video_frames":1})).unwrap();assert!(owner.status.active);
        owner.ingest(0,json!({"event":"bridge_stats","session_epoch":2,"active":true,"epoch_video_frames":1})).unwrap();
        owner.ingest(0,json!({"event":"hisight_closed","session_epoch":1})).unwrap();assert!(owner.status.active);
        owner.ingest(0,json!({"event":"manager_backend_stage","session_epoch":1,"stage":"control_peer_timeout"})).unwrap();assert!(owner.status.active);
        owner.ingest(0,json!({"event":"hisight_closed","session_epoch":2})).unwrap();assert!(!owner.status.active&&owner.status.menu_visible);
        owner.ingest(0,json!({"event":"bridge_stats","session_epoch":2,"active":true,"epoch_video_frames":9})).unwrap();assert!(!owner.status.active);
        owner.ingest(0,json!({"event":"bridge_stats","session_epoch":1,"active":true,"epoch_video_frames":9})).unwrap();assert!(!owner.status.active);
        // The producer socket can stay resident across a wireless reconnect;
        // readiness of a strictly newer session restores projection directly.
        owner.ingest(0,json!({"event":"bridge_stats","session_epoch":3,"active":true,"epoch_video_frames":1})).unwrap();assert!(owner.status.active);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]fn selection_remains_on_requested_protocol_when_phone_is_offline(){
        let root=test_root("protocol-stickiness");let mut owner=test_owner(&root);
        let apple=Device{protocol:Protocol::Carplay,address:"02:00:00:00:00:01".into(),name:"apple".into(),auto:true,last_used:200};
        let huawei=Device{protocol:Protocol::Hicar,address:"02:00:00:00:00:02".into(),name:"huawei".into(),auto:true,last_used:100};
        owner.store.devices=vec![apple.clone(),huawei.clone()];owner.store.last=Some(apple);
        owner.store.selected_protocol=Some(Protocol::Hicar);owner.status.protocol=Some(Protocol::Hicar);
        assert_eq!(owner.automatic().unwrap().protocol,Protocol::Hicar);owner.tried.push_back(huawei.address);
        assert!(owner.automatic().is_none());owner.tried.clear();assert_eq!(owner.automatic().unwrap().protocol,Protocol::Hicar);
        owner.command(json!({"op":"disconnect"})).unwrap();assert_eq!(owner.status.protocol,Some(Protocol::Hicar));assert!(owner.automatic().is_none());
        owner.command(json!({"op":"default_protocol","mode":"carplay"})).unwrap();assert!(owner.store.boot_protocol==BootProtocol::Carplay);
        let (rows,actions)=owner.menu_model();assert_eq!(rows.len(),3);assert_eq!(actions.len(),3);assert_eq!(actions[1]["op"],"connect_device");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]fn connect_device_accepts_saved_phones_instead_of_new_device_only_enrollment(){
        let root=test_root("resume-saved-phone");let mut owner=test_owner(&root);let script=root.join("receiver");
        fs::write(&script,b"#!/bin/sh\nprintf '%s\\n' \"$@\" > \"${0%/*}/receiver-args\"\nexec sleep 60\n").unwrap();
        fs::set_permissions(&script,fs::Permissions::from_mode(0o700)).unwrap();owner.config.carplay=vec![script.to_string_lossy().into_owned()];
        owner.store.devices.push(Device{protocol:Protocol::Carplay,address:"02:00:00:00:00:01".into(),name:"saved iPhone".into(),auto:true,last_used:1});
        owner.store.selected_protocol=Some(Protocol::Carplay);owner.command(json!({"op":"disconnect"})).unwrap();assert!(owner.store.paused);
        owner.command(json!({"op":"connect_device"})).unwrap();assert!(!owner.store.paused);assert!(owner.status.pairing);
        let until=Instant::now()+Duration::from_secs(2);let args=loop{if let Ok(args)=fs::read_to_string(root.join("receiver-args")){if args.lines().any(|arg|arg=="--pair"){break args;}}assert!(Instant::now()<until);thread::sleep(Duration::from_millis(5));};
        assert!(args.lines().any(|arg|arg=="--pair"));assert!(!args.lines().any(|arg|arg=="--new-device-only"));
        owner.stop_backend().unwrap();fs::remove_dir_all(root).unwrap();
    }
    #[test]fn boot_default_overrides_only_on_actual_boot_and_keeps_last_selection_option(){
        let root=test_root("boot-protocol");let mut owner=test_owner(&root);
        owner.store.selected_protocol=Some(Protocol::Hicar);owner.store.boot_protocol=BootProtocol::Carplay;owner.store.boot_id=Some("previous-boot".into());save(&owner.config.state,&owner.store).unwrap();
        let mut restarted=test_owner(&root);assert_eq!(restarted.status.protocol,Some(Protocol::Carplay));assert_eq!(restarted.automatic().unwrap().protocol,Protocol::Carplay);
        restarted.store.selected_protocol=Some(Protocol::Hicar);save(&restarted.config.state,&restarted.store).unwrap();
        let same_boot=test_owner(&root);assert_eq!(same_boot.status.protocol,Some(Protocol::Hicar));
        restarted.store.boot_protocol=BootProtocol::Last;restarted.store.boot_id=Some("previous-boot".into());save(&restarted.config.state,&restarted.store).unwrap();
        let last=test_owner(&root);assert_eq!(last.status.protocol,Some(Protocol::Hicar));fs::remove_dir_all(root).unwrap();
    }
    #[test]fn video_presentation_pause_does_not_end_authenticated_hicar_session(){
        let root=test_root("presentation-pause");let mut owner=test_owner(&root);owner.status.protocol=Some(Protocol::Hicar);owner.wireless_ready=true;owner.status.active=true;owner.status.menu_visible=false;
        owner.ingest(0,json!({"event":"bridge_stats","active":false,"connected":true,"video_frames":100})).unwrap();assert!(owner.status.active);assert!(!owner.status.menu_visible);
        owner.ingest(0,json!({"event":"hisight_closed"})).unwrap();assert!(!owner.status.active);assert!(owner.status.menu_visible);fs::remove_dir_all(root).unwrap();
    }
    #[test]fn carplay_session_phase_returns_menu_immediately_but_hicar_surface_pause_does_not(){
        let root=test_root("session-presentation");let mut owner=test_owner(&root);
        owner.status.protocol=Some(Protocol::Carplay);owner.wireless_ready=true;owner.source_ready=true;owner.status.active=true;owner.status.menu_visible=false;
        owner.menu_input(json!({"type":"presentation-state","active":false})).unwrap();
        assert!(!owner.status.active);assert!(owner.status.menu_visible);assert!(!owner.source_ready);assert!(!owner.wireless_ready);
        owner.menu_input(json!({"type":"presentation-state","active":true})).unwrap();assert!(owner.status.active);
        owner.ingest(0,json!({"event":"stage","stage":"WirelessActive"})).unwrap();assert!(owner.status.active);
        owner.status.protocol=Some(Protocol::Hicar);owner.status.menu_visible=false;
        owner.menu_input(json!({"type":"presentation-state","active":false})).unwrap();
        assert!(owner.status.active);assert!(!owner.status.menu_visible);assert!(owner.source_ready);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]fn active_carplay_phase_restores_local_ipc_without_wireless_reconnect(){
        let root=test_root("carplay-ipc-recovery");let mut owner=test_owner(&root);
        owner.status.protocol=Some(Protocol::Carplay);owner.status.active=true;owner.status.menu_visible=false;owner.wireless_ready=true;owner.source_ready=true;
        owner.source_floor=7;owner.bound_source_id=7;
        owner.menu_input(json!({"type":"source-state","source_id":7,"active":false})).unwrap();
        assert!(!owner.status.active);assert!(!owner.wireless_ready);assert!(owner.status.menu_visible);
        owner.menu_input(json!({"type":"presentation-state","source_id":8,"active":true})).unwrap();
        assert!(owner.status.active);assert!(owner.wireless_ready);assert!(owner.source_ready);assert!(owner.recovery_deadline.is_none());
        owner.menu_input(json!({"type":"presentation-state","source_id":7,"active":false})).unwrap();assert!(owner.status.active);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]fn stale_mux_source_and_call_events_cannot_override_new_protocol_session(){
        let root=test_root("mux-source-fence");let mut owner=test_owner(&root);
        owner.status.protocol=Some(Protocol::Carplay);owner.status.active=true;owner.status.menu_visible=false;
        owner.source_ready=true;owner.wireless_ready=true;owner.source_floor=3;owner.bound_source_id=3;
        for kind in ["source-state","presentation-state","call-state"]{
            owner.menu_input(json!({"type":kind,"source_id":2,"active":false})).unwrap();
        }
        owner.menu_input(json!({"type":"call-state","source_id":2,"active":true})).unwrap();
        assert!(owner.status.active);assert!(!owner.status.menu_visible);assert!(!owner.status.call_active);assert!(owner.source_ready);
        owner.menu_input(json!({"type":"source-state","source_id":4,"active":true})).unwrap();
        owner.menu_input(json!({"type":"call-state","source_id":4,"active":true})).unwrap();assert!(owner.status.call_active);assert_eq!(owner.bound_source_id,4);
        owner.menu_input(json!({"type":"source-state","source_id":3,"active":false})).unwrap();
        owner.menu_input(json!({"type":"call-state","source_id":3,"active":false})).unwrap();assert!(owner.status.active);assert!(owner.status.call_active);
        // HU menu gestures are not wireless-source callbacks and stay usable.
        owner.menu_input(json!({"type":"open-menu"})).unwrap();assert!(owner.status.menu_visible);
        owner.stop_backend().unwrap();assert_eq!(owner.source_floor,5);
        owner.menu_input(json!({"type":"call-state","source_id":4,"active":true})).unwrap();assert!(!owner.status.call_active);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]fn ordinary_offline_does_not_erase_pairing(){for s in ["Host is down","Connection timed out","No route to host","AirPlay session ended"]{assert!(!auth_mismatch(s));}assert!(auth_mismatch("org.bluez.Error.AuthenticationFailed"));}
    #[test]fn stale_generation_cannot_steal_active_backend(){
        let c=Config{schema:1,socket:"/tmp/cm/control".into(),live_socket:"/tmp/cm/live".into(),state:"/tmp/cm/missing-state".into(),carlife:vec![],carplay:vec![],hicar:vec![],devices:vec![],initial:None,bluetoothctl:None,radio_lock:None,media_socket:None,web_listen:None,source_socket:None,menu_library:None,menu_font:None,connect_seconds:30,stable_seconds:10,pair_seconds:120};
        let mut owner=Owner::new(c).unwrap();owner.status.generation=3;owner.status.protocol=Some(Protocol::Carplay);
        owner.ingest(2,json!({"event":"stage","stage":"WirelessActive"})).unwrap();assert!(!owner.status.active);
    }
}
}
pub fn run(args:&[String])->std::result::Result<(),String>{
    #[cfg(unix)]{linux::run(args)}
    #[cfg(not(unix))]{let _=args;Err("connection manager requires Linux/Unix".into())}
}
