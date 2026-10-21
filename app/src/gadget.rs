//! Temporary, explicitly owned FunctionFS AOA1 USB transport.
//! No networking, pairing data, persistent services or other gadgets are changed.

const WINUSB_INTERFACE_GUID: &str = "{B608E05E-72F0-466A-B77C-793EF58A88D5}";

fn os_feature(index: u16, count: u16, body: &[u8]) -> Vec<u8> {
    // Packed FunctionFS header, NOT the 16/10-byte Windows wire headers.
    let mut data = vec![0]; // Interface zero.
    data.extend_from_slice(&(11u32 + body.len() as u32).to_le_bytes());
    data.extend_from_slice(&0x0100u16.to_le_bytes());
    data.extend_from_slice(&index.to_le_bytes());
    data.extend_from_slice(&count.to_le_bytes()); // Compat: bCount + Reserved.
    data.extend_from_slice(body);
    data
}

fn winusb_features() -> [Vec<u8>; 2] {
    let mut compatible = vec![0, 1]; // Interface zero, Reserved1 must equal one.
    compatible.extend_from_slice(b"WINUSB\0\0");
    compatible.extend_from_slice(&[0; 14]); // SubCompatibleID[8], Reserved2[6].

    // FunctionFS accepts UTF-8 property strings and converts them to UTF-16LE
    // in composite.c. Supplying UTF-16 here would encode them twice.
    // Linux 6.18 composite.c does not serialize type 7 (REG_MULTI_SZ).
    // Microsoft's supported single-interface equivalent is the singular
    // DeviceInterfaceGUID REG_SZ, which WinUSB registers for discovery.
    let name = b"DeviceInterfaceGUID\0";
    let value = format!("{WINUSB_INTERFACE_GUID}\0").into_bytes();
    let mut property = Vec::new();
    property.extend_from_slice(&(14u32 + name.len() as u32 + value.len() as u32).to_le_bytes());
    property.extend_from_slice(&1u32.to_le_bytes()); // REG_SZ.
    property.extend_from_slice(&(name.len() as u16).to_le_bytes());
    property.extend_from_slice(name);
    property.extend_from_slice(&(value.len() as u32).to_le_bytes());
    property.extend_from_slice(&value);
    [os_feature(4, 1, &compatible), os_feature(5, 1, &property)]
}

fn descriptors() -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&3u32.to_le_bytes()); // Full-speed descriptor count.
    body.extend_from_slice(&3u32.to_le_bytes()); // High-speed descriptor count.
    body.extend_from_slice(&2u32.to_le_bytes()); // MS OS feature descriptor count.
    for packet in [64u16, 512u16] {
        body.extend_from_slice(&[9, 4, 0, 0, 2, 255, 255, 0, 1]);
        for address in [0x01, 0x82] {
            body.extend_from_slice(&[7, 5, address, 2]);
            body.extend_from_slice(&packet.to_le_bytes());
            body.push(0);
        }
    }
    for feature in winusb_features() {
        body.extend_from_slice(&feature);
    }
    let mut result = Vec::new();
    result.extend_from_slice(&3u32.to_le_bytes()); // FUNCTIONFS_DESCRIPTORS_MAGIC_V2.
    result.extend_from_slice(&(12u32 + body.len() as u32).to_le_bytes());
    // FS, HS, MS OS, device vendor requests, and requests before configuration.
    result.extend_from_slice(&(1u32 | 2 | 8 | 64 | 128).to_le_bytes());
    result.extend_from_slice(&body);
    result
}

fn strings() -> Vec<u8> {
    let text = b"CarLink AOA1\0";
    let mut result = Vec::new();
    result.extend_from_slice(&2u32.to_le_bytes());
    result.extend_from_slice(&(18u32 + text.len() as u32).to_le_bytes());
    result.extend_from_slice(&1u32.to_le_bytes());
    result.extend_from_slice(&1u32.to_le_bytes());
    result.extend_from_slice(&0x0409u16.to_le_bytes());
    result.extend_from_slice(text);
    result
}

#[derive(Debug, PartialEq, Eq)]
enum Request {
    Protocol,
    Identity { index: u16, length: usize },
    Start,
}

fn request(setup: &[u8; 8]) -> Result<Request, String> {
    let value = u16::from_le_bytes([setup[2], setup[3]]);
    let index = u16::from_le_bytes([setup[4], setup[5]]);
    let length = u16::from_le_bytes([setup[6], setup[7]]);
    match (setup[0], setup[1], value, index, length) {
        (0xc0, 51, 0, 0, 2) => Ok(Request::Protocol),
        (0x40, 52, 0, index, length) if index < 6 && (1..=256).contains(&length) => {
            Ok(Request::Identity { index, length: usize::from(length) })
        }
        (0x40, 53, 0, 0, 0) => Ok(Request::Start),
        _ => Err("unsupported AOA request parameters".into()),
    }
}

fn valid_identity(data: &[u8]) -> bool {
    data.last() == Some(&0)
        && !data[..data.len().saturating_sub(1)].contains(&0)
        && std::str::from_utf8(&data[..data.len().saturating_sub(1)]).is_ok()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Initial,
    Accessory,
}

impl Mode {
    fn start(&mut self) -> bool {
        let transition = *self == Self::Initial;
        *self = Self::Accessory;
        transition
    }
}

#[cfg(not(target_os = "linux"))]
pub fn run(_duration: std::time::Duration) -> Result<(), String> {
    Err("FunctionFS AOA gadget is supported only on Linux".into())
}

#[cfg(target_os = "linux")]
pub fn run(duration: std::time::Duration) -> Result<(), String> {
    linux::run(duration)
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{descriptors, request, strings, valid_identity, Mode, Request};
    use std::ffi::CString;
    use std::fs::{self, File, OpenOptions};
    use std::io::{self, Read, Write};
    use std::os::fd::{AsRawFd, RawFd};
    use std::os::raw::{c_char, c_int, c_void};
    use std::os::unix::fs::{symlink, OpenOptionsExt};
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
    use std::sync::Arc;
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    const ROOT: &str = "/sys/kernel/config/usb_gadget/carlink-md";
    const FFS: &str = "/run/carlink-md-ffs";
    const NONBLOCK: i32 = 0x800;
    const POLLIN: i16 = 1;
    const POLLOUT: i16 = 4;
    const MAX_FRAME: usize = 1024 * 1024 + 8;
    static INTERRUPTED: AtomicBool = AtomicBool::new(false);

    #[repr(C)]
    struct PollFd {
        fd: c_int,
        events: i16,
        revents: i16,
    }

    extern "C" {
        fn poll(fds: *mut PollFd, count: usize, timeout: c_int) -> c_int;
        fn read(fd: c_int, buffer: *mut c_void, count: usize) -> isize;
        fn write(fd: c_int, buffer: *const c_void, count: usize) -> isize;
        fn mount(source: *const c_char, target: *const c_char, kind: *const c_char,
                 flags: usize, data: *const c_void) -> c_int;
        fn umount2(target: *const c_char, flags: c_int) -> c_int;
        fn flock(fd: c_int, operation: c_int) -> c_int;
        fn geteuid() -> u32;
        fn signal(number: c_int, handler: usize) -> usize;
    }

    extern "C" fn interrupted(_number: c_int) {
        INTERRUPTED.store(true, Ordering::Relaxed);
    }

    struct Signals(Vec<(c_int, usize)>);

    impl Signals {
        fn install() -> Result<Self, String> {
            INTERRUPTED.store(false, Ordering::Relaxed);
            let mut guard = Self(Vec::new());
            for number in [2, 15] {
                let previous = unsafe { signal(number, interrupted as usize) };
                if previous == usize::MAX {
                    return Err(format!("install signal handler: {}", io::Error::last_os_error()));
                }
                guard.0.push((number, previous));
            }
            Ok(guard)
        }
    }

    impl Drop for Signals {
        fn drop(&mut self) {
            for &(number, previous) in &self.0 {
                unsafe { signal(number, previous); }
            }
        }
    }

    fn put(path: impl AsRef<Path>, text: &str) -> Result<(), String> {
        fs::write(path.as_ref(), format!("{text}\n"))
            .map_err(|e| format!("write {}: {e}", path.as_ref().display()))
    }

    fn mkdir(path: impl AsRef<Path>) -> Result<(), String> {
        fs::create_dir(path.as_ref()).map_err(|e| format!("create {}: {e}", path.as_ref().display()))
    }

    fn open_endpoint(path: impl AsRef<Path>) -> Result<File, String> {
        OpenOptions::new().read(true).write(true).custom_flags(NONBLOCK).open(path.as_ref())
            .map_err(|e| format!("open {}: {e}", path.as_ref().display()))
    }

    fn load_module(name: &str, deadline: Instant) -> Result<(), String> {
        let mut child = Command::new("modprobe").arg(name).stdin(Stdio::null())
            .stdout(Stdio::null()).stderr(Stdio::null()).spawn()
            .map_err(|e| format!("start modprobe {name}: {e}"))?;
        let limit = std::cmp::min(deadline, Instant::now() + Duration::from_secs(3));
        loop {
            match child.try_wait().map_err(|e| format!("wait for modprobe {name}: {e}"))? {
                Some(status) if status.success() => return Ok(()),
                Some(_) => return Err(format!("modprobe {name} failed")),
                None if Instant::now() >= limit => {
                    let _ = child.kill();
                    let _ = child.try_wait();
                    return Err(format!("modprobe {name} exceeded startup deadline"));
                }
                None => thread::sleep(Duration::from_millis(10)),
            }
        }
    }

    fn ready(fd: RawFd, events: i16, timeout: Duration) -> Result<bool, String> {
        let mut entry = PollFd { fd, events, revents: 0 };
        let millis = timeout.as_millis().min(100) as i32;
        let count = unsafe { poll(&mut entry, 1, millis) };
        if count < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted { return Ok(false); }
            return Err(format!("USB endpoint poll: {error}"));
        }
        if entry.revents & (8 | 16 | 32) != 0 {
            return Err("USB endpoint disabled or disconnected".into());
        }
        Ok(entry.revents & events != 0)
    }

    fn live(stop: &AtomicBool, deadline: Instant) -> bool {
        !stop.load(Ordering::Acquire) && !INTERRUPTED.load(Ordering::Relaxed)
            && Instant::now() < deadline
    }

    fn send_bounded<T>(sender: &SyncSender<T>, mut value: T, stop: &AtomicBool,
                       deadline: Instant) -> Result<(), String> {
        while live(stop, deadline) {
            match sender.try_send(value) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Full(back)) => { value = back; thread::sleep(Duration::from_millis(5)); }
                Err(TrySendError::Disconnected(_)) => return Err("USB worker channel closed".into()),
            }
        }
        Ok(())
    }

    struct Gadget {
        _lock: File,
        udc: String,
        created: bool,
        ffs_created: bool,
        mounted: bool,
        linked: bool,
        os_linked: bool,
        bound: bool,
        ep0: Option<File>,
        rx: Option<File>,
        tx: Option<File>,
        allow_cleanup: bool,
    }

    impl Gadget {
        fn open(deadline: Instant) -> Result<Self, String> {
            if unsafe { geteuid() } != 0 { return Err("USB gadget requires root".into()); }
            let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false)
                .open("/run/carlink-md.lock").map_err(|e| format!("open gadget lock: {e}"))?;
            if unsafe { flock(lock.as_raw_fd(), 2 | 4) } != 0 {
                return Err("another CarLink USB gadget process holds the lock".into());
            }
            if Path::new(ROOT).exists() || Path::new(FFS).exists() {
                return Err("CarLink gadget paths already exist; refusing to reuse them".into());
            }
            if Path::new("/sys/module/g_serial").exists() {
                return Err("g_serial is loaded; release USB serial explicitly over Ethernet before running".into());
            }
            load_module("libcomposite", deadline)?;
            load_module("usb_f_fs", deadline)?;
            for item in fs::read_dir("/sys/kernel/config/usb_gadget")
                .map_err(|e| format!("configfs usb_gadget unavailable: {e}"))? {
                let path = item.map_err(|e| e.to_string())?.path().join("UDC");
                if !fs::read_to_string(&path).map_err(|e| format!("inspect existing gadget: {e}"))?.trim().is_empty() {
                    return Err("an existing gadget owns a UDC; refusing to unbind it".into());
                }
            }
            let controllers: Vec<_> = fs::read_dir("/sys/class/udc").map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
            if controllers.len() != 1 { return Err(format!("expected one UDC, found {}", controllers.len())); }
            let controller = controllers[0].path();
            let function = fs::read_to_string(controller.join("function"))
                .map_err(|e| format!("inspect UDC ownership: {e}"))?;
            if !function.trim().is_empty() { return Err("UDC has a bound function; refusing to replace it".into()); }
            let udc = controllers[0].file_name().into_string().map_err(|_| "invalid UDC name")?;
            let mut own = Self { _lock: lock, udc, created: false, ffs_created: false,
                mounted: false, linked: false, os_linked: false, bound: false,
                ep0: None, rx: None, tx: None, allow_cleanup: true };
            mkdir(ROOT)?;
            own.created = true;
            put(Path::new(ROOT).join("idVendor"), "0x18d1")?;
            put(Path::new(ROOT).join("idProduct"), "0x4ee7")?;
            put(Path::new(ROOT).join("bcdUSB"), "0x0200")?;
            put(Path::new(ROOT).join("bcdDevice"), "0x0101")?;
            mkdir(Path::new(ROOT).join("strings/0x409"))?;
            for (key, value) in [("manufacturer", "CarLink"), ("product", "CarLink H3 MD"),
                                 ("serialnumber", "carlink-h3-lab")] {
                put(Path::new(ROOT).join("strings/0x409").join(key), value)?;
            }
            mkdir(Path::new(ROOT).join("configs/c.1"))?;
            put(Path::new(ROOT).join("configs/c.1/MaxPower"), "250")?;
            put(Path::new(ROOT).join("os_desc/use"), "1")?;
            put(Path::new(ROOT).join("os_desc/b_vendor_code"), "0x20")?;
            put(Path::new(ROOT).join("os_desc/qw_sign"), "MSFT100")?;
            symlink(Path::new(ROOT).join("configs/c.1"), Path::new(ROOT).join("os_desc/c.1"))
                .map_err(|e| format!("link MS OS descriptor configuration: {e}"))?;
            own.os_linked = true;
            mkdir(Path::new(ROOT).join("functions/ffs.carlink"))?;
            mkdir(FFS)?;
            own.ffs_created = true;
            let source = CString::new("carlink").unwrap();
            let target = CString::new(FFS).unwrap();
            let kind = CString::new("functionfs").unwrap();
            if unsafe { mount(source.as_ptr(), target.as_ptr(), kind.as_ptr(), 0, std::ptr::null()) } != 0 {
                return Err(format!("mount FunctionFS: {}", io::Error::last_os_error()));
            }
            own.mounted = true;
            let mut ep0 = open_endpoint(Path::new(FFS).join("ep0"))?;
            for data in [descriptors(), strings()] {
                let count = ep0.write(&data).map_err(|e| format!("write FunctionFS descriptors: {e}"))?;
                if count != data.len() { return Err("short FunctionFS descriptor write".into()); }
            }
            own.ep0 = Some(ep0);
            own.rx = Some(open_endpoint(Path::new(FFS).join("ep1"))?);
            own.tx = Some(open_endpoint(Path::new(FFS).join("ep2"))?);
            symlink(Path::new(ROOT).join("functions/ffs.carlink"), Path::new(ROOT).join("configs/c.1/ffs.carlink"))
                .map_err(|e| format!("link FunctionFS configuration: {e}"))?;
            own.linked = true;
            put(Path::new(ROOT).join("UDC"), &own.udc)?;
            own.bound = true;
            eprintln!("usb_gadget: initial identity 18d1:4ee7 bound");
            Ok(own)
        }

        fn accessory(&mut self, deadline: Instant) -> Result<(), String> {
            self.unbind()?;
            let delay = Duration::from_millis(300).min(deadline.saturating_duration_since(Instant::now()));
            thread::sleep(delay);
            if Instant::now() >= deadline { return Err("USB duration expired during AOA transition".into()); }
            put(Path::new(ROOT).join("idProduct"), "0x2d00")?;
            put(Path::new(ROOT).join("UDC"), &self.udc)?;
            self.bound = true;
            eprintln!("usb_gadget: accessory identity 18d1:2d00 bound");
            Ok(())
        }

        fn unbind(&mut self) -> Result<(), String> {
            if self.created && self.bound {
                put(Path::new(ROOT).join("UDC"), "")?;
                self.bound = false;
            }
            Ok(())
        }

        fn cleanup(&mut self) -> Result<(), String> {
            if !self.allow_cleanup { return Err("USB worker did not stop; owned gadget retained for diagnosis".into()); }
            if self.created { self.unbind()?; }
            self.rx.take(); self.tx.take(); self.ep0.take();
            let mut errors = Vec::new();
            if self.os_linked {
                if let Err(e) = fs::remove_file(Path::new(ROOT).join("os_desc/c.1")) { errors.push(e.to_string()); }
                else { self.os_linked = false; }
            }
            if self.linked {
                if let Err(e) = fs::remove_file(Path::new(ROOT).join("configs/c.1/ffs.carlink")) { errors.push(e.to_string()); }
                else { self.linked = false; }
            }
            if self.mounted {
                let target = CString::new(FFS).unwrap();
                if unsafe { umount2(target.as_ptr(), 0) } != 0 { errors.push(format!("umount FunctionFS: {}", io::Error::last_os_error())); }
                else { self.mounted = false; }
            }
            if self.ffs_created && !self.mounted {
                if let Err(e) = fs::remove_dir(FFS) { errors.push(e.to_string()); } else { self.ffs_created = false; }
            }
            if self.created && !self.mounted && !self.linked && !self.os_linked {
                for path in ["functions/ffs.carlink", "configs/c.1", "strings/0x409"] {
                    let child = Path::new(ROOT).join(path);
                    if child.exists() {
                        if let Err(e) = fs::remove_dir(child) { errors.push(e.to_string()); }
                    }
                }
                if let Err(e) = fs::remove_dir(ROOT) { errors.push(e.to_string()); } else { self.created = false; }
            }
            if errors.is_empty() { Ok(()) } else { Err(format!("gadget cleanup: {}", errors.join("; "))) }
        }
    }

    impl Drop for Gadget {
        fn drop(&mut self) {
            if let Err(e) = self.cleanup() { eprintln!("usb_gadget: {e}"); }
        }
    }

    fn transmit(file: &mut File, frame: &[u8], stop: &AtomicBool, deadline: Instant) -> Result<(), String> {
        if frame.len() < 8 || frame.len() > MAX_FRAME { return Err("invalid outgoing USB frame size".into()); }
        let channel = u32::from_be_bytes(frame[..4].try_into().unwrap());
        let length = u32::from_be_bytes(frame[4..8].try_into().unwrap()) as usize;
        if !(1..=6).contains(&channel) || length != frame.len() - 8 {
            return Err("invalid outgoing USB transport header".into());
        }
        // Preserve separate USB transfers for the outer header and body.
        for part in [&frame[..8], &frame[8..]] {
            let mut offset = 0;
            while offset < part.len() && live(stop, deadline) {
                if !ready(file.as_raw_fd(), POLLOUT, Duration::from_millis(50))? { continue; }
                match file.write(&part[offset..]) {
                    Ok(0) => return Err("zero-length USB bulk write".into()),
                    Ok(n) => offset += n,
                    Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => continue,
                    Err(e) => return Err(format!("USB bulk write: {e}")),
                }
            }
            if offset != part.len() { return Err("USB transfer interrupted before completion".into()); }
        }
        Ok(())
    }

    struct Workers {
        stop: Arc<AtomicBool>,
        enabled: Arc<AtomicBool>,
        errors: Receiver<String>,
        handles: Vec<JoinHandle<()>>,
    }

    impl Workers {
        fn start(mut rx: File, mut tx: File, deadline: Instant) -> Result<Self, String> {
            let stop = Arc::new(AtomicBool::new(false));
            let enabled = Arc::new(AtomicBool::new(true));
            let (incoming_tx, incoming_rx) = mpsc::sync_channel::<Vec<u8>>(16);
            let (outgoing_tx, outgoing_rx) = mpsc::sync_channel::<Vec<u8>>(8);
            let (errors_tx, errors) = mpsc::sync_channel::<String>(4);
            let mut own = Self { stop, enabled, errors, handles: Vec::new() };
            let stop = own.stop.clone(); let enabled = own.enabled.clone(); let errors_tx_rx = errors_tx.clone();
            own.handles.push(thread::Builder::new().name("carlink-usb-rx".into()).spawn(move || {
                let result = (|| {
                    let mut data = [0u8; 512]; // Never read only the 8-byte header on MUSB.
                    while live(&stop, deadline) {
                        if !enabled.load(Ordering::Acquire) { thread::sleep(Duration::from_millis(10)); continue; }
                        if !ready(rx.as_raw_fd(), POLLIN, Duration::from_millis(50))? { continue; }
                        match rx.read(&mut data) {
                            Ok(0) => return Err("USB bulk RX reached EOF".into()),
                            Ok(n) => send_bounded(&incoming_tx, data[..n].to_vec(), &stop, deadline)?,
                            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => continue,
                            Err(e) => return Err(format!("USB bulk read: {e}")),
                        }
                    }
                    Ok(())
                })();
                if let Err(e) = result { let _ = errors_tx_rx.try_send(e); stop.store(true, Ordering::Release); }
            }).map_err(|e| format!("start USB RX worker: {e}"))?);
            let stop = own.stop.clone(); let enabled = own.enabled.clone(); let errors_tx_tx = errors_tx.clone();
            let writer = thread::Builder::new().name("carlink-usb-tx".into()).spawn(move || {
                let result = (|| {
                    while live(&stop, deadline) {
                        if !enabled.load(Ordering::Acquire) { thread::sleep(Duration::from_millis(10)); continue; }
                        match outgoing_rx.recv_timeout(Duration::from_millis(50)) {
                            Ok(frame) => transmit(&mut tx, &frame, &stop, deadline)?,
                            Err(mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                        }
                    }
                    Ok(())
                })();
                if let Err(e) = result { let _ = errors_tx_tx.try_send(e); stop.store(true, Ordering::Release); }
            });
            match writer {
                Ok(handle) => own.handles.push(handle),
                Err(e) => { own.stop.store(true, Ordering::Release); return Err(format!("start USB TX worker: {e}")); }
            }
            let stop = own.stop.clone();
            let protocol = thread::Builder::new().name("carlink-usb-protocol".into()).spawn(move || {
                // Construct here: Endpoint is never sent/shared across threads.
                let mut endpoint = crate::runtime::Endpoint::new();
                let result = (|| {
                    while live(&stop, deadline) {
                        match incoming_rx.recv_timeout(Duration::from_millis(50)) {
                            Ok(chunk) => {
                                let responses = endpoint.receive(&chunk)?;
                                if responses.len() > 32 { return Err("too many responses to one USB chunk".into()); }
                                for frame in responses {
                                    if frame.len() > MAX_FRAME { return Err("USB response exceeds frame bound".into()); }
                                    send_bounded(&outgoing_tx, frame, &stop, deadline)?;
                                }
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    }
                    Ok(())
                })();
                let finished = endpoint.finish();
                if let Err(e) = result.and(finished) { let _ = errors_tx.try_send(e); stop.store(true, Ordering::Release); }
            });
            match protocol {
                Ok(handle) => own.handles.push(handle),
                Err(e) => { own.stop.store(true, Ordering::Release); return Err(format!("start USB protocol worker: {e}")); }
            }
            Ok(own)
        }

        fn finish(&mut self) -> Result<(), String> {
            self.stop.store(true, Ordering::Release);
            let limit = Instant::now() + Duration::from_secs(2);
            while self.handles.iter().any(|h| !h.is_finished()) && Instant::now() < limit {
                thread::sleep(Duration::from_millis(10));
            }
            if self.handles.iter().any(|h| !h.is_finished()) {
                return Err("USB worker remained blocked after UDC unbind; descriptors retained".into());
            }
            for handle in self.handles.drain(..) {
                handle.join().map_err(|_| "USB worker panicked".to_string())?;
            }
            if let Ok(error) = self.errors.try_recv() { return Err(error); }
            Ok(())
        }
    }

    fn control_data(fd: RawFd, input: bool, data: &mut [u8]) -> Result<usize, String> {
        let count = unsafe {
            if input { write(fd, data.as_ptr().cast(), data.len()) }
            else { read(fd, data.as_mut_ptr().cast(), data.len()) }
        };
        if count < 0 { Err(format!("AOA control transfer: {}", io::Error::last_os_error())) }
        else { Ok(count as usize) }
    }

    fn stall(fd: RawFd, device_to_host: bool) -> Result<(), String> {
        // FunctionFS stalls ep0 when the operation has the wrong direction.
        let count = unsafe {
            if device_to_host { read(fd, std::ptr::null_mut(), 0) }
            else { write(fd, std::ptr::null(), 0) }
        };
        if count < 0 {
            let error = io::Error::last_os_error();
            if !matches!(error.raw_os_error(), Some(51) | Some(32)) {
                return Err(format!("stall unsupported AOA request: {error}"));
            }
        }
        Ok(())
    }

    fn serve(gadget: &mut Gadget, deadline: Instant, workers: &mut Option<Workers>) -> Result<(), String> {
        let mut mode = Mode::Initial;
        let mut activated = false;
        let mut enabled = false;
        let mut events = [0u8; 12 * 16];
        while Instant::now() < deadline && !INTERRUPTED.load(Ordering::Relaxed) {
            if let Some(worker) = workers.as_ref() {
                if let Ok(error) = worker.errors.try_recv() { return Err(error); }
                if worker.stop.load(Ordering::Acquire) { return Ok(()); }
            }
            let ep0 = gadget.ep0.as_mut().ok_or("missing FunctionFS ep0")?;
            if !ready(ep0.as_raw_fd(), POLLIN, Duration::from_millis(50))? { continue; }
            let count = match ep0.read(&mut events) {
                Ok(0) => return Err("FunctionFS ep0 reached EOF".into()),
                Ok(n) => n,
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => continue,
                Err(e) => return Err(format!("read FunctionFS events: {e}")),
            };
            if count % 12 != 0 { return Err("partial FunctionFS event record".into()); }
            for event in events[..count].chunks_exact(12) {
                match event[8] {
                    0 => {} // BIND.
                    1 | 3 => {
                        enabled = false;
                        if let Some(worker) = workers.as_ref() { worker.enabled.store(false, Ordering::Release); worker.stop.store(true, Ordering::Release); }
                        if activated { eprintln!("usb_gadget: accessory session disabled"); return Ok(()); }
                    }
                    2 => {
                        enabled = true;
                        if mode == Mode::Accessory && !activated {
                            activated = true;
                            eprintln!("usb_gadget: accessory bulk session enabled");
                            let rx = gadget.rx.take().ok_or("missing USB RX endpoint")?;
                            let tx = gadget.tx.take().ok_or("missing USB TX endpoint")?;
                            *workers = Some(Workers::start(rx, tx, deadline)?);
                        }
                    }
                    4 => {
                        let setup: [u8; 8] = event[..8].try_into().unwrap();
                        let fd = gadget.ep0.as_ref().unwrap().as_raw_fd();
                        match request(&setup) {
                            Ok(Request::Protocol) => {
                                let mut version = [1u8, 0];
                                if control_data(fd, true, &mut version)? != 2 { return Err("short AOA protocol reply".into()); }
                            }
                            Ok(Request::Identity { index, length }) => {
                                let mut data = vec![0u8; length];
                                let count = control_data(fd, false, &mut data)?;
                                if count != length || !valid_identity(&data[..count]) {
                                    return Err("invalid AOA identity string encoding or length".into());
                                }
                                eprintln!("usb_gadget: AOA identity index={index} bytes={count}");
                            }
                            Ok(Request::Start) => {
                                control_data(fd, false, &mut [])?; // Complete status before disconnect.
                                if mode.start() {
                                    enabled = false;
                                    gadget.accessory(deadline)?;
                                } else {
                                    eprintln!("usb_gadget: repeated AOA start acknowledged");
                                }
                            }
                            Err(_) => { stall(fd, setup[0] & 0x80 != 0)?; }
                        }
                    }
                    5 => {
                        if let Some(worker) = workers.as_ref() { worker.enabled.store(false, Ordering::Release); }
                    }
                    6 => {
                        if let Some(worker) = workers.as_ref() { worker.enabled.store(enabled, Ordering::Release); }
                    }
                    _ => return Err("unknown FunctionFS event type".into()),
                }
            }
        }
        Ok(())
    }

    pub(super) fn run(duration: Duration) -> Result<(), String> {
        if duration.is_zero() { return Err("USB gadget duration must be positive".into()); }
        let deadline = Instant::now().checked_add(duration).ok_or("USB duration overflow")?;
        let _signals = Signals::install()?;
        let mut gadget = Gadget::open(deadline)?;
        let mut workers = None;
        let mut result = serve(&mut gadget, deadline, &mut workers);
        if let Some(worker) = workers.as_ref() { worker.stop.store(true, Ordering::Release); }
        if let Err(error) = gadget.unbind() {
            gadget.allow_cleanup = false;
            return Err(format!("cannot unbind owned gadget: {error}; descriptors retained"));
        }
        if let Some(worker) = workers.as_mut() {
            if let Err(error) = worker.finish() {
                if worker.handles.iter().any(|h| !h.is_finished()) { gadget.allow_cleanup = false; }
                result = Err(error);
            }
        }
        let cleanup = gadget.cleanup();
        result.and(cleanup)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(kind: u8, code: u8, value: u16, index: u16, length: u16) -> [u8; 8] {
        let mut data = [kind, code, 0, 0, 0, 0, 0, 0];
        data[2..4].copy_from_slice(&value.to_le_bytes());
        data[4..6].copy_from_slice(&index.to_le_bytes());
        data[6..8].copy_from_slice(&length.to_le_bytes());
        data
    }

    #[test]
    fn fs_hs_descriptors_have_fixed_bulk_directions_and_packet_sizes() {
        let data = descriptors();
        assert_eq!(data.len(), 189);
        assert_eq!(u32::from_le_bytes(data[4..8].try_into().unwrap()), 189);
        assert_eq!(u32::from_le_bytes(data[8..12].try_into().unwrap()), 203);
        assert_eq!(&data[12..24], &[3, 0, 0, 0, 3, 0, 0, 0, 2, 0, 0, 0]);
        for (base, size) in [(24, 64u16), (47, 512u16)] {
            assert_eq!(&data[base..base + 9], &[9, 4, 0, 0, 2, 255, 255, 0, 1]);
            for (offset, address) in [(9, 1), (16, 0x82)] {
                assert_eq!(&data[base + offset..base + offset + 4], &[7, 5, address, 2]);
                assert_eq!(&data[base + offset + 4..base + offset + 6], &size.to_le_bytes());
            }
        }
        let string_data = strings();
        assert_eq!(u32::from_le_bytes(string_data[4..8].try_into().unwrap()) as usize, string_data.len());
        assert_eq!(string_data.last(), Some(&0));
    }

    #[test]
    fn winusb_os_features_match_functionfs_packed_parser_layout() {
        let [compatible, properties] = winusb_features();
        for (data, index) in [(&compatible, 4u16), (&properties, 5u16)] {
            assert_eq!(data[0], 0);
            assert_eq!(u32::from_le_bytes(data[1..5].try_into().unwrap()) as usize, data.len());
            assert_eq!(&data[5..7], &0x0100u16.to_le_bytes());
            assert_eq!(&data[7..9], &index.to_le_bytes());
            assert_eq!(&data[9..11], &[1, 0]);
        }
        assert_eq!(compatible.len(), 11 + 24);
        assert_eq!(&compatible[11..13], &[0, 1]);
        assert_eq!(&compatible[13..21], b"WINUSB\0\0");
        assert!(compatible[21..].iter().all(|b| *b == 0));
        // Host-facing compatible descriptor is 40 bytes; FunctionFS is 35.
        assert_ne!(u32::from_le_bytes(compatible[1..5].try_into().unwrap()), 40);
        let property = &properties[11..];
        assert_eq!(u32::from_le_bytes(property[..4].try_into().unwrap()) as usize, property.len());
        assert_eq!(&property[4..8], &1u32.to_le_bytes());
        let name_length = u16::from_le_bytes(property[8..10].try_into().unwrap()) as usize;
        assert_eq!(&property[10..10 + name_length], b"DeviceInterfaceGUID\0");
        let length_offset = 10 + name_length;
        let value_length = u32::from_le_bytes(property[length_offset..length_offset + 4].try_into().unwrap()) as usize;
        let value = &property[length_offset + 4..];
        assert_eq!(value.len(), value_length);
        assert_eq!(value, format!("{WINUSB_INTERFACE_GUID}\0").as_bytes());
        // Composite doubles these UTF-8 ASCII lengths to form wire WCHARs.
        assert_eq!(10 + 14 + 2 * name_length + 2 * value_length, 142);
        assert_eq!(descriptors()[70..], [compatible, properties].concat());
    }

    #[test]
    fn aoa_requests_require_exact_direction_value_index_and_length() {
        assert_eq!(request(&setup(0xc0, 51, 0, 0, 2)).unwrap(), Request::Protocol);
        assert_eq!(request(&setup(0x40, 53, 0, 0, 0)).unwrap(), Request::Start);
        assert_eq!(request(&setup(0x40, 52, 0, 5, 256)).unwrap(), Request::Identity { index: 5, length: 256 });
        for bad in [setup(0x40, 51, 0, 0, 2), setup(0xc1, 51, 0, 0, 2),
                    setup(0xc0, 51, 1, 0, 2), setup(0xc0, 51, 0, 1, 2),
                    setup(0xc0, 51, 0, 0, 1), setup(0x40, 52, 0, 6, 10),
                    setup(0x40, 52, 0, 0, 0), setup(0x40, 52, 0, 0, 257),
                    setup(0x40, 53, 0, 0, 1), setup(0x40, 53, 0, 1, 0)] {
            assert!(request(&bad).is_err());
        }
    }

    #[test]
    fn accessory_repeated_aoa_requests_do_not_trigger_another_rebind() {
        let mut mode = Mode::Initial;
        assert!(mode.start());
        assert!(request(&setup(0xc0, 51, 0, 0, 2)).is_ok());
        assert!(request(&setup(0x40, 52, 0, 1, 8)).is_ok());
        assert!(!mode.start());
        assert!(!mode.start());
    }

    #[test]
    fn identity_requires_one_terminal_nul_and_valid_utf8() {
        assert!(valid_identity(b"CarLink\0"));
        assert!(valid_identity(b"\0"));
        assert!(!valid_identity(b""));
        assert!(!valid_identity(b"CarLink"));
        assert!(!valid_identity(b"a\0b\0"));
        assert!(!valid_identity(b"\xff\0"));
    }
}
