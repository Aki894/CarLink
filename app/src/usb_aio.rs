//! Small native Linux AIO RX queue for FunctionFS. Keep requests queued on MUSB
//! while userspace decodes the previous packet; synchronous requeue has a gap.

#[cfg(all(target_os = "linux", target_endian = "little", any(
    all(target_arch = "arm", target_pointer_width = "32"),
    all(target_arch = "x86_64", target_pointer_width = "64")
)))]
mod native {
    use std::collections::BTreeMap;
    use std::fs::File;
    use std::io;
    use std::os::fd::AsRawFd;
    use std::os::raw::{c_long, c_ulong};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    const DEPTH: usize = 32;
    const PACKET: usize = 512;
    // Linux v6.18 arch/arm/tools/syscall.tbl and
    // arch/x86/entry/syscalls/syscall_64.tbl. ARM getevents uses time32.
    #[cfg(target_arch = "arm")]
    const NUMBERS: [c_long; 4] = [243, 244, 245, 246];
    #[cfg(target_arch = "x86_64")]
    const NUMBERS: [c_long; 4] = [206, 207, 208, 209];

    // include/uapi/linux/aio_abi.h: all pointer/length fields inside iocb are
    // 64-bit even on ARM32; the context and syscall counts are native longs.
    #[repr(C, align(8))]
    #[derive(Default)]
    struct IoCb {
        data: u64,
        key: u32,
        rw_flags: u32,
        opcode: u16,
        priority: i16,
        fd: u32,
        buffer: u64,
        bytes: u64,
        offset: i64,
        reserved: u64,
        flags: u32,
        event_fd: u32,
    }

    #[repr(C, align(8))]
    #[derive(Clone, Copy, Default)]
    struct IoEvent {
        data: u64,
        object: u64,
        result: i64,
        result2: i64,
    }

    #[repr(C)]
    struct Timespec {
        seconds: c_long,
        nanoseconds: c_long,
    }

    extern "C" {
        fn syscall(number: c_long, ...) -> c_long;
    }

    struct Slot {
        cb: IoCb,
        buffer: [u8; PACKET],
        in_flight: bool,
    }

    #[derive(Default)]
    struct Ordered {
        next: u64,
        pending: BTreeMap<u64, Vec<u8>>,
    }

    impl Ordered {
        fn push(&mut self, sequence: u64, bytes: Vec<u8>) -> Result<(), String> {
            if sequence < self.next || self.pending.contains_key(&sequence) {
                return Err("duplicate or stale native AIO completion".into());
            }
            if self.pending.len() >= DEPTH * 2 {
                return Err("native AIO FIFO reorder bound exceeded".into());
            }
            self.pending.insert(sequence, bytes);
            Ok(())
        }

        fn pop(&mut self) -> Option<Vec<u8>> {
            let bytes = self.pending.remove(&self.next)?;
            self.next += 1;
            Some(bytes)
        }
    }

    pub struct ReadQueue {
        context: c_ulong,
        file: Option<File>,
        // Every cb and buffer has a stable address until successful io_destroy.
        // Moving/reallocating this Vec only moves Box pointers.
        slots: Vec<Box<Slot>>,
        submitted: u64,
        ordered: Ordered,
    }

    impl ReadQueue {
        pub fn new(file: File) -> Result<Self, String> {
            let mut context: c_ulong = 0;
            if unsafe { syscall(NUMBERS[0], DEPTH as c_ulong, &mut context as *mut c_ulong) } < 0 {
                return Err(format!("native AIO io_setup: {}", io::Error::last_os_error()));
            }
            let mut own = Self { context, file: Some(file), slots: Vec::with_capacity(DEPTH),
                submitted: 0, ordered: Ordered::default() };
            for _ in 0..DEPTH {
                own.slots.push(Box::new(Slot { cb: IoCb::default(), buffer: [0; PACKET], in_flight: false }));
            }
            // Queue the complete pipeline before the first completion wait.
            for index in 0..DEPTH { own.submit(index)?; }
            Ok(own)
        }

        fn submit(&mut self, index: usize) -> Result<(), String> {
            let fd = self.file.as_ref().ok_or("native AIO queue is closed")?.as_raw_fd();
            let slot = &mut self.slots[index];
            if slot.in_flight { return Err("native AIO slot submitted twice".into()); }
            slot.cb = IoCb { data: self.submitted, fd: fd as u32,
                buffer: slot.buffer.as_mut_ptr() as usize as u64,
                bytes: PACKET as u64, ..IoCb::default() }; // IOCB_CMD_PREAD = 0.
            let mut pointer = &mut slot.cb as *mut IoCb;
            let result = unsafe { syscall(NUMBERS[3], self.context, 1 as c_long, &mut pointer as *mut *mut IoCb) };
            if result != 1 {
                return Err(if result < 0 { format!("native AIO io_submit: {}", io::Error::last_os_error()) }
                    else { "native AIO io_submit did not queue its request".into() });
            }
            slot.in_flight = true;
            self.submitted = self.submitted.checked_add(1).ok_or("native AIO sequence overflow")?;
            Ok(())
        }

        pub fn next(&mut self, stop: &AtomicBool, deadline: Instant) -> Result<Option<Vec<u8>>, String> {
            while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
                if let Some(bytes) = self.ordered.pop() { return Ok(Some(bytes)); }
                let wait = deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(50));
                let mut timeout = Timespec { seconds: 0, nanoseconds: wait.as_nanos() as c_long };
                let mut events = [IoEvent::default(); DEPTH];
                let count = unsafe { syscall(NUMBERS[2], self.context, 1 as c_long, DEPTH as c_long,
                    events.as_mut_ptr(), &mut timeout as *mut Timespec) };
                if count < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::Interrupted { continue; }
                    if stop.load(Ordering::Acquire) { return Ok(None); }
                    return Err(format!("native AIO io_getevents: {error}"));
                }
                if count as usize > DEPTH { return Err("native AIO returned too many events".into()); }
                for event in &events[..count as usize] {
                    let index = self.slots.iter().position(|slot|
                        (&slot.cb as *const IoCb as usize as u64) == event.object)
                        .ok_or("native AIO completion references an unknown iocb")?;
                    let slot = &mut self.slots[index];
                    if !slot.in_flight || slot.cb.data != event.data {
                        return Err("native AIO completion token mismatch".into());
                    }
                    slot.in_flight = false;
                    if event.result < 0 || event.result2 != 0 {
                        if stop.load(Ordering::Acquire) { return Ok(None); }
                        let error = if event.result < 0 {
                            io::Error::from_raw_os_error((-event.result) as i32).to_string()
                        } else { format!("secondary status {}", event.result2) };
                        return Err(format!("native AIO USB read: {error}"));
                    }
                    let length = usize::try_from(event.result).map_err(|_| "native AIO invalid read length")?;
                    if length == 0 { return Err("native AIO USB read reached EOF".into()); }
                    if length > PACKET { return Err("native AIO read exceeds its buffer".into()); }
                    let bytes = slot.buffer[..length].to_vec();
                    // Copy, immediately replenish the MUSB request, THEN hand
                    // bytes to the bounded protocol channel. Never submit a
                    // short read for a header and never borrow in-flight data.
                    if !stop.load(Ordering::Acquire) && Instant::now() < deadline {
                        self.submit(index)?;
                    }
                    self.ordered.push(event.data, bytes)?;
                }
            }
            Ok(None)
        }

        pub fn close(&mut self) -> Result<(), String> {
            if self.context == 0 { return Ok(()); }
            // io_destroy waits for cancellation/completion before returning.
            // The control thread unbinds the UDC before joining this worker.
            let result = unsafe { syscall(NUMBERS[1], self.context) };
            self.context = 0;
            if result < 0 {
                let error = io::Error::last_os_error();
                // Retain both buffers/iocbs and endpoint FD if the kernel has
                // not confirmed destruction. The ordinary cleanup path will
                // report its still-busy mount rather than free kernel targets.
                std::mem::forget(std::mem::take(&mut self.slots));
                if let Some(file) = self.file.take() { std::mem::forget(file); }
                return Err(format!("native AIO io_destroy unconfirmed; resources retained: {error}"));
            }
            self.slots.clear();
            self.file.take();
            Ok(())
        }
    }

    impl Drop for ReadQueue {
        fn drop(&mut self) {
            if let Err(error) = self.close() { eprintln!("usb_gadget: {error}"); }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::mem::{size_of, MaybeUninit};

        #[test]
        fn native_aio_abi_layout_is_fixed_on_both_supported_targets() {
            assert_eq!(size_of::<IoCb>(), 64);
            assert_eq!(size_of::<IoEvent>(), 32);
            assert_eq!(size_of::<Timespec>(), 2 * size_of::<c_long>());
            let value = MaybeUninit::<IoCb>::uninit();
            let base = value.as_ptr();
            unsafe {
                assert_eq!(std::ptr::addr_of!((*base).fd) as usize - base as usize, 20);
                assert_eq!(std::ptr::addr_of!((*base).buffer) as usize - base as usize, 24);
                assert_eq!(std::ptr::addr_of!((*base).bytes) as usize - base as usize, 32);
                assert_eq!(std::ptr::addr_of!((*base).event_fd) as usize - base as usize, 60);
            }
        }

        #[test]
        fn completion_delivery_preserves_submission_fifo() {
            let mut order = Ordered::default();
            order.push(2, vec![2]).unwrap();
            assert!(order.pop().is_none());
            order.push(0, vec![0]).unwrap();
            order.push(1, vec![1]).unwrap();
            assert_eq!(order.pop(), Some(vec![0]));
            assert_eq!(order.pop(), Some(vec![1]));
            assert_eq!(order.pop(), Some(vec![2]));
            assert!(order.pop().is_none());
            assert!(order.push(1, vec![1]).is_err());
        }

        #[test]
        fn stalled_earlier_completion_cannot_create_an_unbounded_reorder_queue() {
            let mut order = Ordered::default();
            for sequence in 1..=(DEPTH * 2) as u64 { order.push(sequence, vec![1]).unwrap(); }
            assert!(order.push(100, vec![1]).is_err());
            assert!(order.push(1, vec![1]).is_err());
        }
    }
}

#[cfg(all(target_os = "linux", target_endian = "little", any(
    all(target_arch = "arm", target_pointer_width = "32"),
    all(target_arch = "x86_64", target_pointer_width = "64")
)))]
pub use native::ReadQueue;

#[cfg(not(all(target_os = "linux", target_endian = "little", any(
    all(target_arch = "arm", target_pointer_width = "32"),
    all(target_arch = "x86_64", target_pointer_width = "64")
))))]
pub struct ReadQueue;

#[cfg(not(all(target_os = "linux", target_endian = "little", any(
    all(target_arch = "arm", target_pointer_width = "32"),
    all(target_arch = "x86_64", target_pointer_width = "64")
))))]
impl ReadQueue {
    pub fn new(_file: std::fs::File) -> Result<Self, String> {
        Err("native USB AIO supports Linux little-endian ARM32 and x86_64 only".into())
    }
    pub fn next(&mut self, _stop: &std::sync::atomic::AtomicBool, _deadline: std::time::Instant)
        -> Result<Option<Vec<u8>>, String> { Err("native USB AIO unsupported".into()) }
    pub fn close(&mut self) -> Result<(), String> { Ok(()) }
}
