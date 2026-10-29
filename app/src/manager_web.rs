//! Small local-network maintenance UI. No pairing secrets are exposed here.
use std::{io::{Read,Write},net::{TcpListener,TcpStream,SocketAddr,IpAddr},os::unix::{ffi::OsStrExt,io::FromRawFd,net::UnixStream},path::{Path,PathBuf},sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}},thread,time::{Duration,Instant}};
use serde_json::{Value,json};
#[derive(Default)]struct Active{tcp:Mutex<Option<TcpStream>>,ipc:Mutex<Option<UnixStream>>}
impl Active{fn close(&self){if let Some(s)=self.tcp.lock().unwrap_or_else(|e|e.into_inner()).take(){let _=s.shutdown(std::net::Shutdown::Both);}if let Some(s)=self.ipc.lock().unwrap_or_else(|e|e.into_inner()).take(){let _=s.shutdown(std::net::Shutdown::Both);}}}
pub struct Web {stop:Arc<AtomicBool>,active:Arc<Active>,task:Option<thread::JoinHandle<()>>}
impl Drop for Web {fn drop(&mut self){self.stop.store(true,Ordering::Relaxed);self.active.close();if let Some(t)=self.task.take(){let _=t.join();}}}
fn local(ip:IpAddr)->bool{match ip{IpAddr::V4(a)=>a.is_loopback()||matches!(a.octets(),[192,168,77,_]|[192,168,49,_]|[10,77,0,_]),IpAddr::V6(a)=>a.is_loopback()}}
fn read_before(s:&mut TcpStream,mut buffer:&mut [u8],deadline:Instant)->Result<(),String>{
 while !buffer.is_empty(){let remaining=deadline.checked_duration_since(Instant::now()).filter(|d|!d.is_zero()).ok_or("request deadline")?;
 s.set_read_timeout(Some(remaining.min(Duration::from_millis(700)))).map_err(|_|"timeout")?;
 match s.read(buffer){Ok(0)=>return Err("request ended early".into()),Ok(n)=>{buffer=&mut buffer[n..];},Err(e) if e.kind()==std::io::ErrorKind::Interrupted=>continue,Err(_)=>return Err("request read".into()),}}
 Ok(())
}
fn request(s:&mut TcpStream,port:u16)->Result<(String,String,Vec<u8>),String>{
 s.set_read_timeout(Some(Duration::from_millis(700))).map_err(|_|"timeout")?;
 let deadline=std::time::Instant::now()+Duration::from_secs(2);
 let mut head=Vec::new();let mut one=[0];while !head.ends_with(b"\r\n\r\n"){if std::time::Instant::now()>=deadline||head.len()>=8192{return Err("headers too large".into());}read_before(s,&mut one,deadline)?;head.push(one[0]);}
 let text=std::str::from_utf8(&head).map_err(|_|"header encoding")?;let mut lines=text.split("\r\n");let first=lines.next().unwrap_or("").split(' ').collect::<Vec<_>>();
 if first.len()!=3||first[2]!="HTTP/1.1"{return Err("HTTP version".into());}
 let mut headers=std::collections::HashMap::new();for line in lines.filter(|s|!s.is_empty()) {let (k,v)=line.split_once(':').ok_or("header shape")?;if headers.insert(k.to_ascii_lowercase(),v.trim()).is_some(){return Err("duplicate header".into());}}
 if headers.contains_key("transfer-encoding"){return Err("chunked requests disabled".into());}
 let host=*headers.get("host").ok_or("missing host")?;
 let own=s.local_addr().map_err(|_|"local address")?;let valid=host==format!("{}",SocketAddr::new(own.ip(),port))||host==format!("carlink.local:{port}");
 if !valid{return Err("unrecognized host".into());}
 let length=headers.get("content-length").map(|v|v.parse::<usize>()).transpose().map_err(|_|"body length")?.unwrap_or(0);
 if length>16384{return Err("body too large".into());}
 if first[0]=="POST" && (headers.get("origin").copied()!=Some(format!("http://{host}").as_str())||headers.get("content-type").copied()!=Some("application/json")){return Err("same-origin JSON required".into());}
 if first[0]!="POST"&&length!=0{return Err("unexpected request body".into());}
 let mut body=vec![0;length];read_before(s,&mut body,deadline)?;Ok((first[0].into(),first[1].into(),body))
}
fn response(s:&mut TcpStream,code:u16,kind:&str,body:&[u8]){let _=s.set_write_timeout(Some(Duration::from_millis(700)));let _=write!(s,"HTTP/1.1 {code} Response\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'\r\n\r\n",body.len());let _=s.write_all(body);
 // Finish the response before discarding a rejected request's unread bytes.
 // An immediate close with unread data can send RST and erase the response.
 let _=s.shutdown(std::net::Shutdown::Write);let _=s.set_read_timeout(Some(Duration::from_millis(50)));
 let deadline=std::time::Instant::now()+Duration::from_millis(150);let mut bytes=[0;1024];let mut total=0;
 while total<16384&&std::time::Instant::now()<deadline{match s.read(&mut bytes){Ok(0)=>break,Ok(n)=>total+=n,Err(_)=>break,}}
}
fn connect_control(path:&Path)->Result<UnixStream,String>{
 let bytes=path.as_os_str().as_bytes();let mut address:libc::sockaddr_un=unsafe{std::mem::zeroed()};
 if bytes.len()>=address.sun_path.len(){return Err("control path too long".into());}
 address.sun_family=libc::AF_UNIX as libc::sa_family_t;for (dst,src) in address.sun_path.iter_mut().zip(bytes){*dst=*src as libc::c_char;}
 let fd=unsafe{libc::socket(libc::AF_UNIX,libc::SOCK_STREAM|libc::SOCK_NONBLOCK|libc::SOCK_CLOEXEC,0)};
 if fd<0{return Err("manager unavailable".into());}
 if unsafe{libc::connect(fd,&address as *const _ as *const libc::sockaddr,std::mem::size_of::<libc::sockaddr_un>()as libc::socklen_t)}!=0{unsafe{libc::close(fd);}return Err("manager unavailable".into());}
 let stream=unsafe{UnixStream::from_raw_fd(fd)};stream.set_nonblocking(false).map_err(|_|"control setup")?;Ok(stream)
}
fn control(path:&Path,v:Value,active:&Active,stop:&AtomicBool)->Result<Value,String>{
 if stop.load(Ordering::Relaxed){return Err("web UI stopping".into());}
 let mut s=connect_control(path)?;let monitor=s.try_clone().map_err(|_|"control monitor")?;
 *active.ipc.lock().unwrap_or_else(|e|e.into_inner())=Some(monitor);
 let result=(||{
  if stop.load(Ordering::Relaxed){active.close();return Err("web UI stopping".into());}
  s.set_write_timeout(Some(Duration::from_secs(1))).map_err(|_|"timeout")?;
  let mut data=serde_json::to_vec(&v).map_err(|_|"JSON")?;data.push(b'\n');s.write_all(&data).map_err(|_|"control write")?;
  let deadline=Instant::now()+Duration::from_secs(12);let mut bytes=Vec::new();let mut chunk=[0u8;1024];
  while bytes.last()!=Some(&b'\n'){
   if stop.load(Ordering::Relaxed){return Err("web UI stopping".into());}
   let remaining=deadline.checked_duration_since(Instant::now()).filter(|d|!d.is_zero()).ok_or("control deadline")?;
   s.set_read_timeout(Some(remaining)).map_err(|_|"timeout")?;
   let n=s.read(&mut chunk).map_err(|_|"control read")?;if n==0{return Err("control closed".into());}
   bytes.extend_from_slice(&chunk[..n]);if bytes.len()>32768{return Err("control reply too large".into());}
  }
  serde_json::from_slice(&bytes).map_err(|_|"control reply".into())
 })();
 active.ipc.lock().unwrap_or_else(|e|e.into_inner()).take();result
}
pub fn start(listen:SocketAddr,socket:PathBuf)->Result<Web,String>{
 let listener=TcpListener::bind(listen).map_err(|_|"cannot bind maintenance web UI")?;listener.set_nonblocking(true).map_err(|_|"web nonblocking")?;let port=listener.local_addr().map_err(|_|"web bind address")?.port();let stop=Arc::new(AtomicBool::new(false));let quit=stop.clone();let active=Arc::new(Active::default());let channels=active.clone();
 let task=thread::spawn(move||{while !quit.load(Ordering::Relaxed){let (mut s,peer)=match listener.accept(){Ok(v)=>v,Err(_)=>{thread::sleep(Duration::from_millis(50));continue;}};
 *channels.tcp.lock().unwrap_or_else(|e|e.into_inner())=s.try_clone().ok();
 if quit.load(Ordering::Relaxed){channels.close();break;}
 if !local(peer.ip()){response(&mut s,403,"text/plain",b"Local maintenance network only");channels.tcp.lock().unwrap_or_else(|e|e.into_inner()).take();continue;}
 let result=request(&mut s,port);match result{
 Ok((m,p,body))=>match (m.as_str(),p.as_str()){
 ("GET","/")=>response(&mut s,200,"text/html; charset=utf-8",include_bytes!("../ui/manager.html")),
 ("GET","/app.js")=>response(&mut s,200,"application/javascript; charset=utf-8",include_bytes!("../ui/manager.js")),
 ("GET","/style.css")=>response(&mut s,200,"text/css; charset=utf-8",include_bytes!("../ui/manager.css")),
 ("GET","/api/status")|("GET","/api/devices")|("POST","/api/control")=>{
 let v=if m=="GET"{Ok(json!({"op":if p.ends_with("status"){"status"}else{"devices"}}))}else{serde_json::from_slice::<Value>(&body).map_err(|_|"invalid JSON".to_string())};
 let checked=v.and_then(|v|{if !matches!(v["op"].as_str(),Some("status"|"devices"|"connect"|"pair"|"pause"|"resume"|"disconnect"|"menu"|"volume"|"policy"|"forget")){return Err("unsupported web operation".into());}Ok(v)});
 match checked{
  Ok(v) if matches!(v["op"].as_str(),Some("connect"|"pair"|"disconnect"|"forget"|"resume"))=>{
   // A source change may tear down the very AP carrying this browser. Finish
   // its bounded HTTP acknowledgement before asking the owner to change radio
   // state. The existing single worker then waits on the private control path.
   response(&mut s,202,"application/json",br#"{"ok":true,"accepted":true}"#);
   let completed=matches!(control(&socket,v,&channels,&quit),Ok(ref value) if value["ok"]==true);
   if !completed&&!quit.load(Ordering::Relaxed){
    println!("{}",json!({"event":"manager_web_control_error","stage":"connection_change"}));
   }
  },
  value=>{let r=value.and_then(|v|control(&socket,v,&channels,&quit)).unwrap_or_else(|e|json!({"ok":false,"error":e}));response(&mut s,200,"application/json",&serde_json::to_vec(&r).unwrap_or_default());}
 }},
 _=>response(&mut s,404,"text/plain",b"Not found")},
 Err(_)=>response(&mut s,400,"text/plain",b"Invalid local request"),}
 channels.tcp.lock().unwrap_or_else(|e|e.into_inner()).take();
 }});Ok(Web{stop,active,task:Some(task)})
}
#[cfg(test)]mod tests {use super::*;
 #[test]fn only_management_networks(){for a in ["127.0.0.1","192.168.77.1","192.168.49.2","10.77.0.189"]{assert!(local(a.parse().unwrap()));}for a in ["8.8.8.8","192.168.0.18","10.78.0.1"]{assert!(!local(a.parse().unwrap()));}}
 #[test]fn a_dripping_post_body_has_one_total_deadline(){
  let server=TcpListener::bind("127.0.0.1:0").unwrap();let address=server.local_addr().unwrap();
  let sender=thread::spawn(move||{let mut s=TcpStream::connect(address).unwrap();
   write!(s,"POST /api/control HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\nContent-Type: application/json\r\nContent-Length: 32\r\n\r\n").unwrap();
   for _ in 0..16{if s.write_all(b" ").is_err(){break;}thread::sleep(Duration::from_millis(300));}
  });
  let(mut peer,_)=server.accept().unwrap();let start=Instant::now();assert!(request(&mut peer,address.port()).is_err());
  let elapsed=start.elapsed();assert!(elapsed>=Duration::from_millis(1700));assert!(elapsed<Duration::from_secs(3));drop(peer);sender.join().unwrap();
 }
 #[test]fn source_change_ack_precedes_private_control_completion(){
  use std::{fs,os::unix::net::UnixListener};
  let path=std::env::temp_dir().join(format!("carlink-web-ack-{}",std::process::id()));let ipc=UnixListener::bind(&path).unwrap();
  let reserve=TcpListener::bind("127.0.0.1:0").unwrap();let address=reserve.local_addr().unwrap();drop(reserve);
  let web=start(address,path.clone()).unwrap();let mut browser=TcpStream::connect(address).unwrap();browser.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
  let body=br#"{"op":"connect","protocol":"hicar","address":"02:00:00:00:00:01"}"#;
  write!(browser,"POST /api/control HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",body.len()).unwrap();browser.write_all(body).unwrap();
  // No private controller has even accepted yet, much less changed a network.
  let mut reply=Vec::new();browser.read_to_end(&mut reply).unwrap();assert!(reply.starts_with(b"HTTP/1.1 202 "));assert!(reply.ends_with(br#"{"ok":true,"accepted":true}"#));
  ipc.set_nonblocking(true).unwrap();let deadline=Instant::now()+Duration::from_secs(3);
  let(mut manager,_)=loop{match ipc.accept(){Ok(peer)=>break peer,Err(e) if e.kind()==std::io::ErrorKind::WouldBlock=>{assert!(Instant::now()<deadline,"acknowledged command was never forwarded");thread::sleep(Duration::from_millis(10));},Err(e)=>panic!("private accept: {e}")}};
  manager.set_read_timeout(Some(Duration::from_secs(3))).unwrap();let mut command=[0u8;1024];let n=manager.read(&mut command).unwrap();assert!(n>0);
  // Keep the accepted private command unanswered. Drop must still interrupt it.
  let before=Instant::now();drop(web);assert!(before.elapsed()<Duration::from_secs(2));
  drop(manager);drop(browser);drop(ipc);fs::remove_file(path).unwrap();
 }
 #[test]fn shutdown_interrupts_a_request_waiting_on_the_manager(){
  use std::{fs,os::unix::net::UnixListener};
  let path=std::env::temp_dir().join(format!("carlink-web-drop-{}",std::process::id()));let ipc=UnixListener::bind(&path).unwrap();
  let reserve=TcpListener::bind("127.0.0.1:0").unwrap();let address=reserve.local_addr().unwrap();drop(reserve);
  let web=start(address,path.clone()).unwrap();let mut browser=TcpStream::connect(address).unwrap();
  write!(browser,"GET /api/status HTTP/1.1\r\nHost: {address}\r\n\r\n").unwrap();
  let(mut manager,_)=ipc.accept().unwrap();manager.set_read_timeout(Some(Duration::from_secs(3))).unwrap();let mut request=[0u8;128];assert!(manager.read(&mut request).unwrap()>0);
  // Keep both peers open, but deliberately never answer the private request.
  let before=Instant::now();drop(web);assert!(before.elapsed()<Duration::from_secs(2));
  drop(manager);drop(browser);drop(ipc);fs::remove_file(path).unwrap();
 }
}
