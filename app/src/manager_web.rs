//! Small local-network maintenance UI. No pairing secrets are exposed here.
use std::{io::{Read,Write},net::{TcpListener,TcpStream,SocketAddr,IpAddr},os::unix::net::UnixStream,path::PathBuf,sync::{Arc,atomic::{AtomicBool,Ordering}},thread,time::Duration};
use serde_json::{Value,json};
pub struct Web {stop:Arc<AtomicBool>,task:Option<thread::JoinHandle<()>>}
impl Drop for Web {fn drop(&mut self){self.stop.store(true,Ordering::Relaxed);if let Some(t)=self.task.take(){let _=t.join();}}}
fn local(ip:IpAddr)->bool{match ip{IpAddr::V4(a)=>a.is_loopback()||matches!(a.octets(),[192,168,77,_]|[192,168,49,_]|[10,77,0,_]),IpAddr::V6(a)=>a.is_loopback()}}
fn request(s:&mut TcpStream,port:u16)->Result<(String,String,Vec<u8>),String>{
 s.set_read_timeout(Some(Duration::from_millis(700))).map_err(|_|"timeout")?;
 let deadline=std::time::Instant::now()+Duration::from_secs(2);
 let mut head=Vec::new();let mut one=[0];while !head.ends_with(b"\r\n\r\n"){if std::time::Instant::now()>=deadline||head.len()>=8192{return Err("headers too large".into());}s.read_exact(&mut one).map_err(|_|"header read")?;head.push(one[0]);}
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
 let mut body=vec![0;length];s.read_exact(&mut body).map_err(|_|"body read")?;Ok((first[0].into(),first[1].into(),body))
}
fn response(s:&mut TcpStream,code:u16,kind:&str,body:&[u8]){let _=s.set_write_timeout(Some(Duration::from_millis(700)));let _=write!(s,"HTTP/1.1 {code} Response\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'\r\n\r\n",body.len());let _=s.write_all(body);}
fn control(path:&PathBuf,v:Value)->Result<Value,String>{let mut s=UnixStream::connect(path).map_err(|_|"manager unavailable")?;s.set_read_timeout(Some(Duration::from_secs(12))).map_err(|_|"timeout")?;s.set_write_timeout(Some(Duration::from_secs(1))).map_err(|_|"timeout")?;let mut data=serde_json::to_vec(&v).map_err(|_|"JSON")?;data.push(b'\n');s.write_all(&data).map_err(|_|"control write")?;let mut bytes=Vec::new();let mut one=[0];while bytes.last()!=Some(&b'\n'){if bytes.len()>32768{return Err("control reply too large".into());}s.read_exact(&mut one).map_err(|_|"control read")?;bytes.push(one[0]);}serde_json::from_slice(&bytes).map_err(|_|"control reply".into())}
pub fn start(listen:SocketAddr,socket:PathBuf)->Result<Web,String>{
 let listener=TcpListener::bind(listen).map_err(|_|"cannot bind maintenance web UI")?;listener.set_nonblocking(true).map_err(|_|"web nonblocking")?;let port=listener.local_addr().map_err(|_|"web bind address")?.port();let stop=Arc::new(AtomicBool::new(false));let quit=stop.clone();
 let task=thread::spawn(move||{while !quit.load(Ordering::Relaxed){let (mut s,peer)=match listener.accept(){Ok(v)=>v,Err(_)=>{thread::sleep(Duration::from_millis(50));continue;}};if !local(peer.ip()){response(&mut s,403,"text/plain",b"Local maintenance network only");continue;}
 let result=request(&mut s,port);match result{
 Ok((m,p,body))=>match (m.as_str(),p.as_str()){
 ("GET","/")=>response(&mut s,200,"text/html; charset=utf-8",include_bytes!("../ui/manager.html")),
 ("GET","/app.js")=>response(&mut s,200,"application/javascript; charset=utf-8",include_bytes!("../ui/manager.js")),
 ("GET","/style.css")=>response(&mut s,200,"text/css; charset=utf-8",include_bytes!("../ui/manager.css")),
 ("GET","/api/status")|("GET","/api/devices")|("POST","/api/control")=>{
 let v=if m=="GET"{Ok(json!({"op":if p.ends_with("status"){"status"}else{"devices"}}))}else{serde_json::from_slice::<Value>(&body).map_err(|_|"invalid JSON".to_string())};
 let r=v.and_then(|v|{if !matches!(v["op"].as_str(),Some("status"|"devices"|"connect"|"pair"|"pause"|"resume"|"disconnect"|"menu"|"policy"|"forget")){return Err("unsupported web operation".into());}control(&socket,v)}).unwrap_or_else(|e|json!({"ok":false,"error":e}));response(&mut s,200,"application/json",&serde_json::to_vec(&r).unwrap_or_default());},
 _=>response(&mut s,404,"text/plain",b"Not found")},
 Err(_)=>response(&mut s,400,"text/plain",b"Invalid local request"),}
 }});Ok(Web{stop,task:Some(task)})
}
#[cfg(test)]mod tests {use super::*;#[test]fn only_management_networks(){for a in ["127.0.0.1","192.168.77.1","192.168.49.2","10.77.0.189"]{assert!(local(a.parse().unwrap()));}for a in ["8.8.8.8","192.168.0.18","10.78.0.1"]{assert!(!local(a.parse().unwrap()));}}}
