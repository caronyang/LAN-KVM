use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// The service, the session agent and the winjector helper all append to the
/// same file. `writeln!` issues several writes per line, which used to
/// interleave mid-line. Format first, then write once while holding a lock.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

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
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
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
