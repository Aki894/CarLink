//! Read-only Linux diagnostics. No shell commands or network requests.
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn quote(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c <= '\u{1f}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn object(fields: &[(&str, String)]) -> String {
    format!("{{{}}}", fields.iter().map(|(k, v)| format!("{}:{}", quote(k), v)).collect::<Vec<_>>().join(","))
}

fn array(values: Vec<String>) -> String {
    format!("[{}]", values.join(","))
}

fn error_status(error: &io::Error) -> &'static str {
    match error.kind() {
        io::ErrorKind::NotFound => "not_present",
        io::ErrorKind::PermissionDenied => "permission_denied",
        _ => "error",
    }
}

fn failure(path: &Path, error: io::Error) -> String {
    object(&[
        ("status", quote(error_status(&error))),
        ("path", quote(&path.to_string_lossy())),
        ("value", "null".into()),
        ("error", quote(&error.to_string())),
    ])
}

fn observation(path: &Path, value: String) -> String {
    object(&[
        ("status", quote("ok")),
        ("path", quote(&path.to_string_lossy())),
        ("value", value),
    ])
}

fn invalid(path: &Path, reason: &str) -> String {
    object(&[
        ("status", quote("invalid_data")),
        ("path", quote(&path.to_string_lossy())),
        ("value", "null".into()),
        ("error", quote(reason)),
    ])
}

fn read_text(path: &Path) -> String {
    match fs::read_to_string(path) {
        Ok(text) => observation(path, quote(text.trim())),
        Err(error) => failure(path, error),
    }
}

/// Record only the driver basename, not the arbitrary symlink target path.
fn driver(path: &Path) -> String {
    match fs::read_link(path) {
        Ok(target) => match target.file_name().and_then(|name| name.to_str()) {
            Some(name) => observation(path, quote(name)),
            None => invalid(path, "driver symlink has no UTF-8 basename"),
        },
        Err(error) => failure(path, error),
    }
}

fn memory_kib(text: &str) -> Option<u64> {
    let line = text.lines().find(|line| line.starts_with("MemTotal:"))?;
    let mut parts = line.split_whitespace();
    if parts.next()? != "MemTotal:" { return None; }
    let value = parts.next()?.parse().ok()?;
    if parts.next()? != "kB" || parts.next().is_some() { return None; }
    Some(value)
}

fn memory(path: &Path) -> String {
    match fs::read_to_string(path) {
        Ok(text) => match memory_kib(&text) {
            Some(value) => observation(path, value.to_string()),
            None => invalid(path, "MemTotal must contain an unsigned integer in kB"),
        },
        Err(error) => failure(path, error),
    }
}

fn os_fields(text: &str) -> String {
    // /etc/os-release is data, never source it in a shell. Keep only public OS fields.
    let mut fields = Vec::new();
    for key in ["PRETTY_NAME", "NAME", "VERSION_ID", "ID"] {
        if let Some(raw) = text.lines().find_map(|line| line.strip_prefix(&format!("{key}="))) {
            let value = raw.trim();
            let value = if value.len() >= 2 &&
                ((value.starts_with('"') && value.ends_with('"')) ||
                 (value.starts_with('\'') && value.ends_with('\''))) {
                &value[1..value.len() - 1]
            } else { value };
            fields.push((key, quote(value)));
        }
    }
    object(&fields)
}

fn os_release(path: &Path) -> String {
    match fs::read_to_string(path) {
        Ok(text) => observation(path, os_fields(&text)),
        Err(error) => failure(path, error),
    }
}

/// Retain entry errors instead of silently converting an unreadable directory to [].
fn entries(path: &Path) -> Result<(Vec<PathBuf>, Vec<String>), io::Error> {
    let mut paths = Vec::new();
    let mut errors = Vec::new();
    for entry in fs::read_dir(path)? {
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(error) => errors.push(failure(path, error)),
        }
    }
    paths.sort();
    Ok((paths, errors))
}

fn listing(path: &Path, items: Vec<String>, errors: Vec<String>) -> String {
    object(&[
        ("status", quote(if errors.is_empty() { "ok" } else { "partial" })),
        ("path", quote(&path.to_string_lossy())),
        ("items", array(items)),
        ("errors", array(errors)),
    ])
}

fn name(path: &Path) -> String {
    quote(&path.file_name().unwrap_or_default().to_string_lossy())
}

fn udcs(path: &Path) -> String {
    match entries(path) {
        Ok((paths, errors)) => listing(path, paths.iter().map(|p| object(&[
            ("name", name(p)),
            ("state", read_text(&p.join("state"))),
            ("current_speed", read_text(&p.join("current_speed"))),
        ])).collect(), errors),
        Err(error) => failure(path, error),
    }
}

fn usb(path: &Path) -> String {
    match entries(path) {
        Ok((paths, mut errors)) => {
            let mut devices = Vec::new();
            for device in paths {
                // USB interfaces (e.g. 1-1:1.0) are not devices. A failed vendor
                // read on a device stays visible, including disappearance races.
                if device.file_name().unwrap_or_default().to_string_lossy().contains(':') { continue; }
                match fs::metadata(&device) {
                    Ok(meta) if meta.is_dir() => devices.push(object(&[
                        ("name", name(&device)),
                        ("vendor_id", read_text(&device.join("idVendor"))),
                        ("product_id", read_text(&device.join("idProduct"))),
                        ("product", read_text(&device.join("product"))),
                        ("driver", driver(&device.join("driver"))),
                    ])),
                    Ok(_) => {},
                    Err(error) => errors.push(failure(&device, error)),
                }
            }
            listing(path, devices, errors)
        },
        Err(error) => failure(path, error),
    }
}

fn network(path: &Path) -> String {
    match entries(path) {
        Ok((paths, errors)) => listing(path, paths.iter().map(|p| object(&[
            ("name", name(p)),
            ("operstate", read_text(&p.join("operstate"))),
            ("driver", driver(&p.join("device/driver"))),
        ])).collect(), errors),
        Err(error) => failure(path, error),
    }
}

fn report(root: &Path) -> String {
    object(&[
        ("schema_version", "1".into()),
        ("application", quote("carlink")),
        ("version", quote(env!("CARGO_PKG_VERSION"))),
        ("compiled_arch", quote(env::consts::ARCH)),
        ("compiled_os", quote(env::consts::OS)),
        ("os", os_release(&root.join("etc/os-release"))),
        ("kernel", read_text(&root.join("proc/sys/kernel/osrelease"))),
        ("mem_total_kib", memory(&root.join("proc/meminfo"))),
        ("udc", udcs(&root.join("sys/class/udc"))),
        ("usb", usb(&root.join("sys/bus/usb/devices"))),
        ("network", network(&root.join("sys/class/net"))),
        ("aoa", object(&[
            ("status", quote("not_tested")),
            ("reason", quote("Read-only inventory does not test USB roles, AOA negotiation, bulk transport, or CarLife connectivity")),
        ])),
    ])
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args == ["--help"] || args == ["-h"] {
        println!("Usage: carlink doctor --json\nRead-only Linux inventory; no USB mode changes or connection tests.");
        return;
    }
    if args != ["doctor", "--json"] {
        eprintln!("Usage: carlink doctor --json");
        std::process::exit(2);
    }
    println!("{}", report(Path::new("/")));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_json_controls_and_preserves_unicode() {
        assert_eq!(quote("a\"\\\n\r\t\0\u{1f}车"), "\"a\\\"\\\\\\n\\r\\t\\u0000\\u001f车\"");
    }

    #[test]
    fn memory_requires_correct_field_unit_and_integer() {
        assert_eq!(memory_kib("MemFree: 3 kB\nMemTotal:  524288 kB\n"), Some(524288));
        for value in ["MemTotal: -1 kB", "MemTotal: 9 MB", "MemTotal: 2 kB extra", "MemFree: 2 kB"] {
            assert_eq!(memory_kib(value), None, "{value}");
        }
    }

    #[test]
    fn os_release_does_not_expose_unknown_fields_or_execute_values() {
        let value = os_fields("NAME=\"Car Linux\"\nID=car\nSECRET=password\nVERSION_ID='$(touch /tmp/never)'\n");
        assert_eq!(value, "{\"NAME\":\"Car Linux\",\"VERSION_ID\":\"$(touch /tmp/never)\",\"ID\":\"car\"}");
        assert!(!value.contains("password"));
    }

    #[test]
    fn missing_linux_interfaces_remain_errors_and_aoa_is_not_tested() {
        let missing = env::temp_dir().join(format!("carlink-missing-fixture-{}", std::process::id()));
        assert!(!missing.exists(), "test fixture path must not exist");
        let value = report(&missing);
        assert!(value.contains("\"status\":\"not_present\""));
        assert!(value.contains("\"status\":\"not_tested\""));
        assert!(!value.contains("\"status\":\"ok\""));
    }

    #[test]
    fn permission_denial_is_not_reported_as_missing_hardware() {
        assert_eq!(error_status(&io::Error::from(io::ErrorKind::PermissionDenied)), "permission_denied");
        assert_eq!(error_status(&io::Error::from(io::ErrorKind::NotFound)), "not_present");
    }
}
