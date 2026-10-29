use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use lankvm_platform::logfile::{maybe_rotate, Rotation};

/// The service, the session agent and the winjector helper all append to the
/// same file. `writeln!` issues several writes per line, which used to
/// interleave mid-line. Format first, then write once while holding a lock.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// 5 MB per file with three generations: at most ~20 MB per component. The
/// session agent logs an input counter every 5 s, so this file grows even when
/// LAN-KVM sits idle.
const ROTATION: Rotation = Rotation::new(5 * 1024 * 1024, 3);

pub fn log(component: &str, message: impl AsRef<str>) {
    let path = log_path(component);
    if let Some(parent) = path.parent() {
        let _ = create_dir_all(parent);
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("[{stamp}] [{}] {}\n", std::process::id(), message.as_ref());
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let written = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(mut file) => match file.write_all(line.as_bytes()) {
            Ok(()) => line.len() as u64,
            Err(_) => 0,
        },
        Err(_) => 0,
    };
    // The handle above is closed here on purpose: Windows cannot rename a file
    // that another handle still has open.
    if written > 0 {
        maybe_rotate(&path, ROTATION, written);
    }
}

pub fn log_path(component: &str) -> PathBuf {
    let base = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("LAN-KVM")
        .join("logs")
        .join(format!("{component}.log"))
}
