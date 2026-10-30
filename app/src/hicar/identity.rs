//! Compatible schema-1 identities, atomically initialized once on the device.
use super::{Result,json,crypto::{Crypto,wipe}};
use std::path::Path;
pub struct Identity {pub device_id:String,pub pin:String,pub name:String}
impl Identity {
    /// Publish a new identity only if absent. Existing identities are validated
    /// without changing their bytes, PIN, permissions or native HUKS storage.
    #[cfg(unix)]pub fn ensure(path:&Path,name:&str)->Result<bool>{
        use std::{fs::{self,OpenOptions,DirBuilder},io::{Read,Write},os::unix::fs::{OpenOptionsExt,DirBuilderExt},path::Component};
        if !path.is_absolute()||path.as_os_str().len()>4096||path.file_name().is_none()
            ||path.components().any(|c|matches!(c,Component::ParentDir))||name.is_empty()||name.chars().count()>64||name.chars().any(char::is_control){return Err("invalid identity initialization parameters".into());}
        match fs::symlink_metadata(path){Ok(_)=>{Self::load(path)?;return Ok(false)},Err(e)if e.kind()==std::io::ErrorKind::NotFound=>{},Err(_)=>return Err("cannot inspect private identity".into())}
        let parent=path.parent().ok_or("identity parent required")?;
        let mut directory=std::path::PathBuf::new();
        for component in parent.components(){
            directory.push(component.as_os_str());
            match fs::symlink_metadata(&directory){
                Ok(meta)if meta.is_dir()&&!meta.file_type().is_symlink()=>{},
                Ok(_)=>return Err("identity parent must be a real directory".into()),
                Err(e)if e.kind()==std::io::ErrorKind::NotFound=>{
                    match DirBuilder::new().mode(0o700).create(&directory){Ok(())=>{},Err(e)if e.kind()==std::io::ErrorKind::AlreadyExists=>{
                        let m=fs::symlink_metadata(&directory).map_err(|_|"cannot inspect identity directory")?;
                        if !m.is_dir()||m.file_type().is_symlink(){return Err("identity directory changed".into());}
                    },Err(_)=>return Err("cannot create private identity directory".into())}
                },Err(_)=>return Err("cannot inspect identity directory".into()),
            }
        }
        let mut random=[0u8;44];
        std::fs::File::open("/dev/urandom").and_then(|mut f|f.read_exact(&mut random)).map_err(|_|"identity random source unavailable")?;
        // Reject the short modulo tail to keep all six-digit PINs equiprobable.
        let mut pin=u32::from_le_bytes(random[32..36].try_into().unwrap());
        while pin>=4_294_000_000{
            let mut b=[0u8;4];std::fs::File::open("/dev/urandom").and_then(|mut f|f.read_exact(&mut b)).map_err(|_|"PIN random source unavailable")?;pin=u32::from_le_bytes(b);wipe(&mut b);
        }
        let mut encoded=serde_json::to_vec(&serde_json::json!({"schema":1,"device_id":super::hex(&random[..32]),"pin":format!("{:06}",pin%1_000_000),"name":name})).map_err(|_|"cannot encode private identity")?;
        let temporary=parent.join(format!(".identity-{}",super::hex(&random[36..])));wipe(&mut random);
        let mut created=false;
        let result=(||{
            let mut file=OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC).open(&temporary).map_err(|_|"cannot create private identity file")?;
            created=true;
            file.write_all(&encoded).and_then(|_|file.sync_all()).map_err(|_|"cannot persist private identity")?;
            match fs::hard_link(&temporary,path){
                Ok(())=>{std::fs::File::open(parent).and_then(|f|f.sync_all()).map_err(|_|"cannot sync identity directory")?;Ok(true)},
                Err(e)if e.kind()==std::io::ErrorKind::AlreadyExists=>{Self::load(path)?;Ok(false)},
                Err(_)=>Err("cannot publish private identity".into()),
            }
        })();
        wipe(&mut encoded);if created{let _=fs::remove_file(&temporary);}result
    }
    pub fn parse(data:&[u8])->Result<Self>{
        if data.len()>4096{return Err("identity file exceeds limit".into());}
        let value=json::parse(data)?;
        if value.get("schema").and_then(|v|v.as_u64())!=Some(1){return Err("unsupported identity schema".into());}
        let text=|name:&str|value.get(name).and_then(|v|v.as_str()).map(str::to_owned).ok_or("invalid identity field".to_owned());
        let device_id=text("device_id")?;let pin=text("pin")?;let name=text("name")?;
        if device_id.len()!=64||!device_id.bytes().all(|b|b.is_ascii_hexdigit())
            ||pin.len()!=6||!pin.bytes().all(|b|b.is_ascii_digit())||name.is_empty()||name.chars().count()>64 {
            return Err("invalid private identity".into());
        }Ok(Self{device_id,pin,name})
    }
    pub fn wire_id(&self,model:&str,address:&str,crypto:&Crypto)->Result<String>{
        let parts:Vec<_>=address.split(':').collect();
        if model.len()!=8||!model.bytes().all(|b|b.is_ascii_hexdigit())||parts.len()!=6
            ||parts.iter().any(|p|p.len()!=2||!p.bytes().all(|b|b.is_ascii_hexdigit())){return Err("invalid model or adapter address".into());}
        let seed=format!("{}{}",model,parts.join("")).to_ascii_uppercase();
        Ok(super::hex(&crypto.sha256(seed.as_bytes())?).to_ascii_uppercase())
    }
    #[cfg(unix)]pub fn load(path:&Path)->Result<Self>{
        use std::{fs::OpenOptions,io::Read,os::unix::fs::{OpenOptionsExt,PermissionsExt}};
        let f=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC)
            .open(path).map_err(|_|"cannot open private identity")?;
        let meta=f.metadata().map_err(|_|"cannot stat private identity")?;
        if !meta.is_file()||meta.len()>4096||meta.permissions().mode()&0o077!=0{return Err("identity must be a private regular file".into());}
        let mut data=Vec::new();f.take(4097).read_to_end(&mut data).map_err(|_|"cannot read identity")?;
        let result=Self::parse(&data);wipe(&mut data);result
    }
}
impl Drop for Identity{fn drop(&mut self){unsafe{wipe(self.device_id.as_bytes_mut());wipe(self.pin.as_bytes_mut());}}}

#[cfg(all(test,unix))]mod tests{
    use super::*;use std::{fs,os::unix::fs::{PermissionsExt,symlink}};
    #[test]fn python_schema_is_read_only_and_private(){
        let root=std::env::temp_dir().join(format!("carlink-id-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir(&root).unwrap();let path=root.join("identity.json");
        let record=serde_json::json!({"schema":1,"device_id":"ab".repeat(32),"pin":"012345","name":"CarLink 中文"}).to_string();
        fs::write(&path,&record).unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(0o600)).unwrap();
        let id=Identity::load(&path).unwrap();assert_eq!(id.pin,"012345");assert_eq!(id.name,"CarLink 中文");
        assert_eq!(fs::read(&path).unwrap(),record.as_bytes());
        symlink(&path,root.join("linked")).unwrap();assert!(Identity::load(&root.join("linked")).is_err());
        fs::set_permissions(&path,fs::Permissions::from_mode(0o644)).unwrap();assert!(Identity::load(&path).is_err());
        assert!(Identity::parse(b"{}").is_err());fs::remove_dir_all(root).unwrap();
    }
    #[test]fn initializes_once_concurrently_and_preserves_existing_keys(){
        let root=std::env::temp_dir().join(format!("carlink-init-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let path=root.join("production/identity.json");
        let threads:Vec<_>=(0..8).map(|_|{let path=path.clone();std::thread::spawn(move||Identity::ensure(&path,"CarLink"))}).collect();
        assert_eq!(threads.into_iter().map(|t|t.join().unwrap().unwrap()).filter(|created|*created).count(),1);
        let original=fs::read(&path).unwrap();let id=Identity::load(&path).unwrap();assert_eq!(id.name,"CarLink");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode()&0o777,0o600);
        assert_eq!(fs::metadata(path.parent().unwrap()).unwrap().permissions().mode()&0o777,0o700);
        fs::create_dir(root.join("production/keys")).unwrap();fs::write(root.join("production/keys/retained"),b"existing HUKS data").unwrap();
        assert!(!Identity::ensure(&path,"Another name").unwrap());assert_eq!(fs::read(&path).unwrap(),original);
        assert_eq!(fs::read(root.join("production/keys/retained")).unwrap(),b"existing HUKS data");
        assert!(fs::read_dir(path.parent().unwrap()).unwrap().all(|e|!e.unwrap().file_name().to_string_lossy().starts_with(".identity-")));
        let corrupt=root.join("bad.json");fs::write(&corrupt,b"invalid").unwrap();fs::set_permissions(&corrupt,fs::Permissions::from_mode(0o600)).unwrap();
        assert!(Identity::ensure(&corrupt,"CarLink").is_err());assert_eq!(fs::read(&corrupt).unwrap(),b"invalid");
        symlink(&path,root.join("linked.json")).unwrap();assert!(Identity::ensure(&root.join("linked.json"),"CarLink").is_err());
        symlink(root.join("production"),root.join("linked-dir")).unwrap();assert!(Identity::ensure(&root.join("linked-dir/new.json"),"CarLink").is_err());
        assert!(Identity::ensure(Path::new("relative.json"),"CarLink").is_err());
        assert!(Identity::ensure(&root.join("other.json"),"bad\nname").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
