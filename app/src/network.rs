//! Select only the verified external AIC Wi-Fi device; never the Ethernet link.
use std::path::Path;
pub fn discover(root:&Path)->Result<String,String>{
    let mut candidates=Vec::new();
    for entry in std::fs::read_dir(root).map_err(|_|"cannot enumerate network interfaces")?{
        let entry=entry.map_err(|_|"cannot inspect network interface")?;let path=entry.path();
        let name=entry.file_name().into_string().map_err(|_|"invalid network interface name")?;
        if name.len()>15||!name.bytes().all(|b|b.is_ascii_alphanumeric()||b"_.-".contains(&b))||matches!(name.as_str(),"lo"|"end0"){continue;}
        if !path.join("wireless").is_dir()&&!path.join("phy80211").exists(){continue;}
        // This driver may register the netdev against its USB device instead
        // of an interface, whose parent "usb" driver has no module symlink.
        let aic_module=|p:std::path::PathBuf|p.canonicalize().ok().and_then(|p|p.file_name().map(|m|m.to_string_lossy().starts_with("aic8800"))).unwrap_or(false);
        let direct=aic_module(path.join("device/driver/module"));
        let interface=std::fs::read_dir(path.join("device")).ok().is_some_and(|entries|entries.filter_map(Result::ok).take(64).any(|entry|entry.file_name().to_string_lossy().contains(':')&&aic_module(entry.path().join("driver/module"))));
        let id=|field:&str|std::fs::read_to_string(path.join("device").join(field)).unwrap_or_default().trim().to_ascii_lowercase();
        let hardware=matches!((id("idVendor").as_str(),id("idProduct").as_str()),("a69c","8d80")|("368b","8d81"));
        if direct||interface||hardware{candidates.push(name);}
    }
    candidates.sort();match candidates.as_slice(){[name]=>Ok(name.clone()),[]=>Err("AX900/AIC8800 Wi-Fi interface is not ready".into()),_=>Err("multiple AIC Wi-Fi interfaces; configure one explicitly".into())}
}

#[cfg(all(test,unix))]mod tests{
    use super::*;use std::{fs,os::unix::fs::symlink};
    #[test]fn chooses_aic_not_ethernet_or_onboard_and_rejects_ambiguity(){
        let root=std::env::temp_dir().join(format!("carlink-net-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(root.join("net")).unwrap();let net=root.join("net");
        let add=|name:&str,module:&str,wifi:bool|{let path=net.join(name);fs::create_dir_all(path.join("device/driver")).unwrap();if wifi{fs::create_dir(path.join("wireless")).unwrap();}let target=root.join("modules").join(module);fs::create_dir_all(&target).unwrap();symlink(target,path.join("device/driver/module")).unwrap();};
        add("end0","aic8800_fdrv",false);add("wlan0","xradio_wlan",true);assert!(discover(&net).is_err());
        add("wlx112233445566","aic8800_fdrv",true);assert_eq!(discover(&net).unwrap(),"wlx112233445566");
        add("wlx667788990011","aic8800_fdrv",true);assert!(discover(&net).is_err());fs::remove_dir_all(root).unwrap();
    }
    #[test]fn usb_parent_netdev_uses_confirmed_runtime_id_or_interface_driver(){
        let root=std::env::temp_dir().join(format!("carlink-usb-net-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let net=root.join("net");let path=net.join("wlx112233445566");fs::create_dir_all(path.join("wireless")).unwrap();fs::create_dir_all(path.join("device")).unwrap();
        fs::write(path.join("device/idVendor"),b"a69c\n").unwrap();fs::write(path.join("device/idProduct"),b"8d80\n").unwrap();assert_eq!(discover(&net).unwrap(),"wlx112233445566");
        fs::write(path.join("device/idVendor"),b"368b\n").unwrap();fs::write(path.join("device/idProduct"),b"8d81\n").unwrap();assert_eq!(discover(&net).unwrap(),"wlx112233445566");
        fs::write(path.join("device/idVendor"),b"a69c\n").unwrap();
        fs::write(path.join("device/idProduct"),b"5721\n").unwrap();assert!(discover(&net).is_err());
        let module=root.join("modules/aic8800_fdrv");fs::create_dir_all(&module).unwrap();let interface=path.join("device/2-1:1.0/driver");fs::create_dir_all(&interface).unwrap();symlink(module,interface.join("module")).unwrap();assert_eq!(discover(&net).unwrap(),"wlx112233445566");
        fs::remove_dir_all(root).unwrap();
    }
}
